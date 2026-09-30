//! `neru` in the terminal. The same core as the window (settings, keys, sessions, tools, approvals,
//! memory, hooks, sub-agents) runs without a window, and this module draws it the way Claude Code
//! does: a welcome box, streamed Markdown, tool rows, diffs to approve, and slash commands.

use std::{
    collections::HashMap,
    io::{IsTerminal, Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use serde_json::Value;
use tauri::{Listener, Manager};

use crate::{
    AppState, agent,
    cli_ui::{self, BOLD, CREAM, CYAN, DIM, FAINT, GREEN, Input, MOSS, RED, RESET, SAGE, STRIKE, Screen, Suggestion, YELLOW},
    commands, sessions, workspace,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Options {
    prompt: Option<String>,
    print: bool,
    resume_latest: bool,
    resume: Option<Option<String>>,
    mode: String,
    model: Option<String>,
    web: bool,
}

fn usage() -> String {
    format!(
        "Neru {VERSION} — a local-first coding agent in your terminal\n\n\
Usage:\n  neru [prompt]              Start a session in this folder (optionally with a first request)\n  neru -p \"prompt\"           Print mode: answer once and exit (reads stdin when piped)\n  neru -c                    Continue the latest session in this folder\n  neru -r [search]           Resume a session (pick one, or match its title)\n\n\
Options:\n  --mode <review|plan|edits|auto|bypass>   Permission mode (default: review)\n  --model <name>             Use this model (part of a name is enough)\n  --no-web                   Turn web search and fetch off\n  -v, --version              Show the version\n  -h, --help                 Show this help\n\n\
In a session, type / for commands and @ to mention files. Esc interrupts a reply; text you\n\
type while Neru works is sent to it at its next step."
    )
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options { prompt: None, print: false, resume_latest: false, resume: None, mode: "manual".into(), model: None, web: true };
    let mut args = std::env::args().skip(1).peekable();
    let mut words = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(usage()),
            "-v" | "--version" => return Err(format!("neru {VERSION}")),
            "-p" | "--print" => options.print = true,
            "-c" | "--continue" => options.resume_latest = true,
            "-r" | "--resume" => {
                let search = args.peek().filter(|next| !next.starts_with('-')).cloned();
                if search.is_some() {
                    args.next();
                }
                options.resume = Some(search);
            }
            "--mode" => options.mode = mode_id(&args.next().ok_or("--mode needs a value")?).ok_or("Modes: review, plan, edits, auto, bypass")?.into(),
            "--model" => options.model = Some(args.next().ok_or("--model needs a value")?),
            "--no-web" => options.web = false,
            other if other.starts_with('-') && other.len() > 1 => return Err(format!("Unknown option {other}\n\n{}", usage())),
            other => words.push(other.to_string()),
        }
    }
    if !words.is_empty() {
        options.prompt = Some(words.join(" "));
    }
    Ok(options)
}

fn mode_id(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "review" | "manual" | "default" => "manual",
        "plan" => "plan",
        "edits" | "accept" | "accept_edits" | "acceptedits" => "accept_edits",
        "auto" => "auto",
        "bypass" | "yolo" => "bypass",
        _ => return None,
    })
}

fn mode_name(id: &str) -> &'static str {
    match id {
        "plan" => "plan (read only)",
        "accept_edits" => "accept edits",
        "auto" => "auto",
        "bypass" => "bypass permissions",
        _ => "review (asks before changes)",
    }
}

pub fn run() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            println!("{message}");
            return;
        }
    };
    let mut context = crate::context();
    // No window: the terminal is the interface.
    context.config_mut().app.windows.clear();
    let app = crate::builder(move |handle| {
        std::thread::spawn(move || {
            let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session(&handle, options))).unwrap_or_else(|_| {
                let _ = terminal::disable_raw_mode();
                eprintln!("\nNeru hit an unexpected problem and stopped. Your session is saved; run `neru -c` to continue.");
                70
            });
            handle.exit(code);
        });
    })
    .build(context);
    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(error) => eprintln!("Neru could not start: {error}"),
    }
}

// ---------- shared state for one CLI session ----------

struct Cli<'a> {
    app: &'a tauri::AppHandle,
    screen: Screen,
    editor: cli_ui::Editor,
    events: mpsc::Receiver<Value>,
    session: String,
    mode: String,
    web: bool,
    commands: Vec<commands::SlashCommand>,
    files: Vec<String>,
    context: Option<agent::ContextUsage>,
    last_interrupt: Option<Instant>,
}

fn session(app: &tauri::AppHandle, options: Options) -> i32 {
    if std::env::var_os("NERU_DEBUG").is_some() {
        eprintln!("[neru] core ready");
    }
    let interactive = !options.print && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let state = app.state::<AppState>();
    let cwd = std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
    if let Err(error) = workspace::open_project(cwd.clone(), app.state()) {
        eprintln!("{RED}Could not open {cwd}: {error}{RESET}");
        return 1;
    }
    let (sender, events) = mpsc::channel();
    app.listen("agent://event", move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            let _ = sender.send(value);
        }
    });
    tauri::async_runtime::spawn(crate::mcp::start_enabled(app.clone()));

    if let Some(model) = &options.model {
        if let Err(error) = switch_model(app, model) {
            eprintln!("{RED}{error}{RESET}");
            return 1;
        }
    }
    // Pick the session: continue, resume, or a fresh one.
    let chosen = if options.resume_latest {
        sessions::list_sessions(app.state()).ok().and_then(|list| list.into_iter().max_by_key(|item| item.updated_at)).map(|item| item.id)
    } else {
        None
    };
    let mut cli = Cli {
        app,
        screen: Screen::new(std::io::stdout().is_terminal()),
        editor: cli_ui::Editor { history: load_history() },
        events,
        session: String::new(),
        mode: options.mode.clone(),
        web: options.web,
        commands: commands::list_commands(app.state()),
        files: workspace::list_project_files(app.state()).unwrap_or_default(),
        context: None,
        last_interrupt: None,
    };
    let opened = match (chosen, &options.resume) {
        (Some(id), _) => sessions::select_session(id, app.state()),
        (None, Some(search)) if interactive => match cli.choose_session(search.as_deref()) {
            Some(id) => sessions::select_session(id, app.state()),
            None => sessions::create_session(Some(false), app.state()),
        },
        _ => sessions::create_session(Some(false), app.state()),
    };
    match opened {
        Ok(snapshot) => cli.session = snapshot.session.id.clone(),
        Err(error) => {
            eprintln!("{RED}Could not start a session: {error}{RESET}");
            return 1;
        }
    }
    let provider = state.provider.lock().map(|config| format!("{} · {}", config.model, config.provider_id)).unwrap_or_default();

    if !interactive {
        let mut prompt = options.prompt.clone().unwrap_or_default();
        if !std::io::stdin().is_terminal() {
            let mut piped = String::new();
            let _ = std::io::stdin().read_to_string(&mut piped);
            if !piped.trim().is_empty() {
                prompt = if prompt.is_empty() { piped } else { format!("{prompt}\n\n{piped}") };
            }
        }
        if prompt.trim().is_empty() {
            eprintln!("Nothing to do: give a prompt, e.g. neru -p \"explain this project\"");
            return 2;
        }
        return cli.print_mode(&prompt);
    }

    cli.screen.print(&cli_ui::welcome(&cwd, &provider, mode_name(&cli.mode), VERSION));
    if let Ok(snapshot) = sessions::session_snapshot(cli.session.clone(), app.state()) {
        if !snapshot.messages.is_empty() {
            cli.screen.print(&format!(" {DIM}Resumed “{}” ({} messages){RESET}\n", snapshot.session.title, snapshot.messages.len()));
            for entry in snapshot.messages.iter().rev().take(2).collect::<Vec<_>>().into_iter().rev() {
                let who = if entry.role == "user" { format!("{DIM}❯{RESET}") } else { format!("{CREAM}⏺{RESET}") };
                let first = entry.content.lines().next().unwrap_or("");
                cli.screen.print(&format!(" {who} {DIM}{}{RESET}", first.chars().take(cli_ui::width() - 6).collect::<String>()));
            }
            cli.screen.print("");
        }
    }
    if !configured(app) {
        cli.screen.print(&format!(" {YELLOW}⚠{RESET} No model is set up yet. Open the Neru app → Settings → Model to add a key, or run {CYAN}/model{RESET} after that.\n"));
    }
    if let Some(prompt) = options.prompt.clone() {
        cli.screen.print(&format!("{DIM}❯{RESET} {prompt}\n"));
        cli.turn(&prompt);
    }
    loop {
        let suggest = cli.suggester();
        let hint = if cli.mode == "manual" { "Ask Neru to build, fix or explain something".to_string() } else { format!("Ask Neru… ({})", mode_name(&cli.mode)) };
        match cli.editor.read(&mut cli.screen, &hint, &suggest) {
            Input::Line(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                cli.last_interrupt = None;
                cli.screen.print(&format!("{DIM}❯{RESET} {}", line.replace('\n', "\n  ")));
                if line.starts_with('/') {
                    match cli.command(&line) {
                        Flow::Quit => break,
                        Flow::Continue => {}
                        Flow::Send(prompt) => cli.turn(&prompt),
                    }
                } else {
                    cli.turn(&line);
                }
                cli.screen.print("");
            }
            Input::Interrupt => {
                if cli.last_interrupt.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
                    break;
                }
                cli.last_interrupt = Some(Instant::now());
                cli.screen.print(&format!("  {DIM}Press Ctrl+C again to exit{RESET}"));
            }
            Input::Eof => break,
        }
    }
    save_history(&cli.editor.history);
    cli.screen.clear();
    println!("{DIM}Session saved. Continue it with {RESET}{CYAN}neru -c{RESET}");
    0
}

fn configured(app: &tauri::AppHandle) -> bool {
    agent::provider_status(app.state()).is_ok_and(|view| view.configured && !view.model.is_empty())
}

enum Flow {
    Quit,
    Continue,
    Send(String),
}

fn history_path() -> Option<std::path::PathBuf> {
    workspace::data_dir().ok().map(|dir| dir.join("cli-history.txt"))
}

fn load_history() -> Vec<String> {
    history_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.lines().filter(|line| !line.is_empty()).map(|line| line.replace("\\n", "\n")).collect())
        .unwrap_or_default()
}

fn save_history(history: &[String]) {
    if let Some(path) = history_path() {
        let keep: Vec<String> = history.iter().rev().take(500).rev().map(|line| line.replace('\n', "\\n")).collect();
        let _ = std::fs::write(path, keep.join("\n"));
    }
}

fn switch_model(app: &tauri::AppHandle, wanted: &str) -> Result<String, String> {
    let view = agent::provider_status(app.state())?;
    let config = app.state::<AppState>().provider.lock().map_err(|e| e.to_string())?.clone();
    let models = tauri::async_runtime::block_on(crate::models::list_model_ids(view.provider_id.clone(), view.api_format.clone(), view.base_url.clone(), app.clone())).unwrap_or_default();
    let terms: Vec<String> = wanted.to_lowercase().split_whitespace().map(str::to_string).collect();
    let chosen = models
        .iter()
        .find(|model| model.eq_ignore_ascii_case(wanted))
        .or_else(|| models.iter().find(|model| terms.iter().all(|term| model.to_lowercase().contains(term))))
        .cloned()
        .unwrap_or_else(|| wanted.to_string());
    agent::configure_provider(config.provider_id, config.api_format, config.base_url, String::new(), chosen.clone(), app.state())?;
    Ok(chosen)
}

// ---------- rendering a reply ----------

/// How the live reply is drawn while it streams.
struct Live {
    line: String,
    fence: Option<String>,
    started_block: bool,
    tools: HashMap<String, (String, String)>,
    drafts: HashMap<String, (String, String)>,
    status: String,
    reasoning: usize,
    since: Instant,
    typed: String,
}

impl Live {
    fn new() -> Self {
        Self { line: String::new(), fence: None, started_block: false, tools: HashMap::new(), drafts: HashMap::new(), status: "Thinking".into(), reasoning: 0, since: Instant::now(), typed: String::new() }
    }
}

impl<'a> Cli<'a> {
    fn state(&self) -> tauri::State<'a, AppState> {
        self.app.state::<AppState>()
    }

    /// One request, including any approvals it needs, until the agent finishes.
    fn turn(&mut self, prompt: &str) {
        let mut prompt = prompt.to_string();
        loop {
            match self.reply(&prompt) {
                Ok(Some(pending)) => match self.approve(&pending) {
                    Approval::Continue => prompt = String::new(),
                    Approval::Feedback(text) => prompt = text,
                    Approval::Stop => return,
                },
                Ok(None) => return,
                Err(error) => {
                    self.flush_text(&mut Live::new());
                    self.screen.print(&format!("{RED}⏺{RESET} {}", friendly(&error)));
                    return;
                }
            }
        }
    }

    /// Runs `ai_chat` and draws its events. Returns the pending approval, if it stopped for one.
    fn reply(&mut self, prompt: &str) -> Result<Option<agent::PendingView>, String> {
        while self.events.try_recv().is_ok() {}
        let (done_tx, done_rx) = mpsc::channel();
        let app = self.app.clone();
        let (prompt_owned, mode, web, session) = (prompt.to_string(), self.mode.clone(), self.web, self.session.clone());
        tauri::async_runtime::spawn(async move {
            let result = agent::ai_chat(prompt_owned, vec![], mode, Some(web), None, None, None, Some(session), None, Some("code".into()), app).await;
            let _ = done_tx.send(result);
        });
        let mut live = Live::new();
        let _ = terminal::enable_raw_mode();
        let result = loop {
            while let Ok(event) = self.events.try_recv() {
                if event["sessionId"].as_str() == Some(self.session.as_str()) {
                    self.render(&mut live, &event);
                }
            }
            if let Ok(result) = done_rx.try_recv() {
                // Late events (the final draft, the last tool) still belong to this reply.
                while let Ok(event) = self.events.try_recv() {
                    if event["sessionId"].as_str() == Some(self.session.as_str()) {
                        self.render(&mut live, &event);
                    }
                }
                break result;
            }
            self.keys(&mut live);
            self.draw_status(&live);
            let _ = event::poll(Duration::from_millis(60));
        };
        let _ = terminal::disable_raw_mode();
        self.flush_text(&mut live);
        self.screen.clear();
        let response = result?;
        self.context = Some(response.context.clone());
        if live.tools.is_empty() && response.content.trim().is_empty() && response.pending.is_none() {
            self.screen.print(&format!("{DIM}  (no reply){RESET}"));
        }
        Ok(response.pending)
    }

    fn keys(&mut self, live: &mut Live) {
        while event::poll(Duration::from_millis(0)).unwrap_or(false) {
            let Ok(Event::Key(key)) = event::read() else { continue };
            if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                continue;
            }
            match key.code {
                KeyCode::Esc => {
                    let _ = agent::stop_chat(Some(self.session.clone()), self.state());
                    live.status = "Interrupting".into();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = agent::stop_chat(Some(self.session.clone()), self.state());
                    live.status = "Interrupting".into();
                }
                KeyCode::Enter => {
                    let text = std::mem::take(&mut live.typed);
                    if !text.trim().is_empty() {
                        match agent::steer_session(self.session.clone(), text.clone(), self.state()) {
                            Ok(true) => {}
                            _ => live.typed = text,
                        }
                    }
                }
                KeyCode::Backspace => {
                    live.typed.pop();
                }
                KeyCode::Char(c) => live.typed.push(c),
                _ => {}
            }
        }
    }

    fn draw_status(&mut self, live: &Live) {
        let frame = cli_ui::SPINNER[(live.since.elapsed().as_millis() / 90) as usize % cli_ui::SPINNER.len()];
        let mut lines = Vec::new();
        if !live.line.is_empty() {
            // The unfinished line of text, shown until it completes.
            let prefix = if live.started_block { "  " } else { "⏺ " };
            lines.push(format!("{CREAM}{prefix}{RESET}{}", live.line));
        }
        let extra = if live.reasoning > 0 && live.status == "Thinking" { format!(" · {} chars of reasoning", live.reasoning) } else { String::new() };
        lines.push(format!("{MOSS}{frame}{RESET} {SAGE}{}…{RESET} {DIM}({}{extra} · esc to interrupt){RESET}", live.status, cli_ui::elapsed(live.since)));
        if !live.typed.is_empty() {
            lines.push(format!("{FAINT}  ↳{RESET} {}{DIM}  (enter sends it to Neru now){RESET}", live.typed));
        }
        self.screen.footer(&lines, None);
    }

    fn flush_text(&mut self, live: &mut Live) {
        if !live.line.is_empty() {
            let line = std::mem::take(&mut live.line);
            self.print_text_line(live, &line);
        }
    }

    fn print_text_line(&mut self, live: &mut Live, line: &str) {
        let rendered = cli_ui::markdown_line(line, &mut live.fence);
        let prefix = if live.started_block { "  ".to_string() } else { format!("{CREAM}⏺{RESET} ") };
        if !live.started_block && rendered.trim().is_empty() {
            return;
        }
        live.started_block = true;
        self.screen.print(&format!("{prefix}{rendered}"));
    }

    fn render(&mut self, live: &mut Live, event: &Value) {
        match event["type"].as_str().unwrap_or("") {
            "delta" => {
                let text = event["text"].as_str().unwrap_or("");
                live.status = "Writing".into();
                for c in text.chars() {
                    if c == '\n' {
                        let line = std::mem::take(&mut live.line);
                        if line.trim().is_empty() && live.fence.is_none() {
                            if live.started_block {
                                self.screen.print("");
                            }
                            continue;
                        }
                        self.print_text_line(live, &line);
                    } else {
                        live.line.push(c);
                    }
                }
            }
            "reasoning" => {
                live.reasoning = event["chars"].as_u64().unwrap_or(0) as usize;
            }
            "draft" => {
                let id = event["id"].as_str().unwrap_or("").to_string();
                let path = event["path"].as_str().unwrap_or("").to_string();
                let content = event["content"].as_str().unwrap_or("").to_string();
                let lines = content.lines().count();
                live.status = format!("Writing {} · {lines} lines", path.rsplit(['/', '\\']).next().unwrap_or(&path));
                live.drafts.insert(id, (path, content));
            }
            "tool" => {
                let id = event["id"].as_str().unwrap_or("").to_string();
                let label = event["label"].as_str().unwrap_or("").to_string();
                let status = event["status"].as_str().unwrap_or("").to_string();
                if status == "running" {
                    live.status = label.clone();
                    live.tools.insert(id, (label, status));
                    return;
                }
                self.flush_text(live);
                live.started_block = false;
                let (dot, color) = match status.as_str() {
                    "error" => ("⏺", RED),
                    "pending" => ("⏺", YELLOW),
                    _ => ("⏺", GREEN),
                };
                self.screen.print(&format!("{color}{dot}{RESET} {BOLD}{}{RESET}", label));
                if let Some((path, content)) = live.drafts.get(&id).cloned() {
                    if status != "pending" && status != "error" {
                        self.screen.print(&format!("  {FAINT}⎿{RESET}  {DIM}Wrote {} lines to {path}{RESET}", content.lines().count()));
                        self.screen.print(&cli_ui::code_preview(&content, &path, 8));
                    }
                }
                live.tools.insert(id, (label, status));
                live.status = "Thinking".into();
            }
            "todos" => {
                self.flush_text(live);
                live.started_block = false;
                let mut text = format!("{GREEN}⏺{RESET} {BOLD}Update Todos{RESET}");
                for (index, todo) in event["todos"].as_array().into_iter().flatten().enumerate() {
                    let content = todo["content"].as_str().unwrap_or("");
                    let lead = if index == 0 { format!("  {FAINT}⎿{RESET}  ") } else { "     ".into() };
                    let item = match todo["status"].as_str() {
                        Some("completed") => format!("{GREEN}☒{RESET} {DIM}{STRIKE}{content}{RESET}"),
                        Some("in_progress") => format!("{SAGE}☐ {BOLD}{content}{RESET}"),
                        _ => format!("☐ {content}"),
                    };
                    text.push_str(&format!("\n{lead}{item}"));
                }
                self.screen.print(&text);
            }
            "notice" => {
                self.flush_text(live);
                self.screen.print(&format!("  {FAINT}⎿{RESET}  {DIM}{}{RESET}", event["text"].as_str().unwrap_or("")));
            }
            "steered" => {
                self.flush_text(live);
                live.started_block = false;
                self.screen.print(&format!("{DIM}❯ {} (sent while working){RESET}", event["text"].as_str().unwrap_or("")));
            }
            "provider" => {
                self.screen.print(&format!("  {FAINT}⎿{RESET}  {DIM}Now using {} on {}{RESET}", event["model"].as_str().unwrap_or(""), event["providerId"].as_str().unwrap_or("")));
            }
            "context" => {
                if let Ok(usage) = serde_json::from_value::<ContextView>(event["usage"].clone()) {
                    let _ = usage;
                }
            }
            _ => {}
        }
    }

    // ---------- approvals ----------

    fn approve(&mut self, pending: &agent::PendingView) -> Approval {
        let (title, body) = match pending.kind.as_str() {
            "edit" => (format!("Edit {}", pending.label), cli_ui::diff(pending.diff.as_deref().unwrap_or(""), 60)),
            "delete" => (format!("Delete {}", pending.label), cli_ui::diff(pending.diff.as_deref().unwrap_or(""), 20)),
            "move" => (format!("Move {}", pending.label), String::new()),
            "mkdir" => (format!("Create folder {}", pending.label), String::new()),
            _ => ("Run this command?".to_string(), format!("   {CYAN}{}{RESET}", pending.label)),
        };
        let width = cli_ui::width().saturating_sub(2);
        self.screen.print(&format!("{YELLOW}╭{}╮{RESET}", "─".repeat(width)));
        self.screen.print(&format!("{YELLOW}│{RESET} {BOLD}{title}{RESET}"));
        if !body.is_empty() {
            self.screen.print(&body);
        }
        self.screen.print(&format!("{YELLOW}╰{}╯{RESET}", "─".repeat(width)));
        let task = pending.kind == "task";
        let mut options = vec![("Yes".to_string(), String::new())];
        if task {
            options.push(("Yes, and don't ask again for this exact command in this project".into(), String::new()));
        } else {
            options.push(("Yes, and accept edits for the rest of this session".into(), String::new()));
        }
        options.push(("No, and tell Neru what to do differently".into(), "esc".into()));
        let choice = cli_ui::pick(&mut self.screen, "Do you want to proceed?", &options, Some(0));
        let result = match choice {
            Some(0) | Some(1) => {
                if choice == Some(1) {
                    if task {
                        let _ = agent::allow_pending_always(self.state());
                    } else {
                        self.mode = "accept_edits".into();
                    }
                }
                let outcome = if task {
                    self.screen.print(&format!("{DIM}  Running…{RESET}"));
                    tauri::async_runtime::block_on(agent::run_pending_task(self.app.clone())).map(|output| {
                        let lines: Vec<&str> = output.lines().collect();
                        let shown = lines.iter().take(12).map(|line| format!("     {DIM}{line}{RESET}")).collect::<Vec<_>>().join("\n");
                        let more = if lines.len() > 12 { format!("\n     {FAINT}… +{} lines{RESET}", lines.len() - 12) } else { String::new() };
                        format!("{shown}{more}")
                    })
                } else {
                    workspace::apply_pending(self.state()).map(|_| String::new())
                };
                match outcome {
                    Ok(output) => {
                        self.screen.print(&format!("{GREEN}⏺{RESET} {}", if task { "Ran it" } else { "Applied" }));
                        if !output.trim().is_empty() {
                            self.screen.print(&output);
                        }
                        self.files = workspace::list_project_files(self.state()).unwrap_or_default();
                        Approval::Continue
                    }
                    Err(error) => {
                        self.screen.print(&format!("{RED}⏺{RESET} {}", friendly(&error)));
                        let _ = workspace::reject_pending(self.state());
                        Approval::Stop
                    }
                }
            }
            _ => {
                let _ = workspace::reject_pending(self.state());
                self.screen.print(&format!("{RED}⏺{RESET} {DIM}Declined.{RESET}"));
                let suggest = |_: &str| Vec::new();
                match self.editor.read(&mut self.screen, "What should Neru do instead? (enter to skip)", &suggest) {
                    Input::Line(line) if !line.trim().is_empty() => {
                        self.screen.print(&format!("{DIM}❯{RESET} {}", line.trim()));
                        Approval::Feedback(line.trim().to_string())
                    }
                    _ => Approval::Stop,
                }
            }
        };
        result
    }

    // ---------- print mode ----------

    fn print_mode(&mut self, prompt: &str) -> i32 {
        let (done_tx, done_rx) = mpsc::channel();
        let app = self.app.clone();
        let (prompt, mode, web, session) = (prompt.to_string(), self.mode.clone(), self.web, self.session.clone());
        tauri::async_runtime::spawn(async move {
            let _ = done_tx.send(agent::ai_chat(prompt, vec![], mode, Some(web), None, None, None, Some(session), None, Some("code".into()), app).await);
        });
        let color = std::io::stderr().is_terminal();
        let mut out = std::io::stdout();
        let debug = std::env::var_os("NERU_DEBUG").is_some();
        let started = Instant::now();
        let mut first = true;
        if debug {
            eprintln!("[neru] request sent after startup");
        }
        loop {
            while let Ok(event) = self.events.try_recv() {
                if debug {
                    if first && event["type"] != "status" {
                        first = false;
                        eprintln!("[neru] first {} event after {} ms", event["type"].as_str().unwrap_or(""), started.elapsed().as_millis());
                    }
                    if event["type"] == "reasoning" {
                        eprint!(".");
                    }
                }
                match event["type"].as_str().unwrap_or("") {
                    "delta" => {
                        let _ = write!(out, "{}", event["text"].as_str().unwrap_or(""));
                        let _ = out.flush();
                    }
                    "tool" if event["status"] != "running" => {
                        let label = event["label"].as_str().unwrap_or("");
                        if color { eprintln!("{DIM}⏺ {label}{RESET}") } else { eprintln!("⏺ {label}") }
                    }
                    _ => {}
                }
            }
            match done_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(response)) => {
                    println!();
                    if debug {
                        eprintln!("[neru] done after {} ms", started.elapsed().as_millis());
                    }
                    if let Some(pending) = response.pending {
                        let _ = workspace::reject_pending(self.state());
                        eprintln!("Stopped: “{}” needs approval. Run with --mode edits, auto or bypass to allow it without asking.", pending.label);
                        return 3;
                    }
                    return 0;
                }
                Ok(Err(error)) => {
                    eprintln!("{}", friendly(&error));
                    return 1;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return 1,
            }
        }
    }

    // ---------- slash commands ----------

    fn suggester(&self) -> impl Fn(&str) -> Vec<Suggestion> + 'static {
        let commands = self.command_list();
        let files = self.files.clone();
        move |before: &str| {
            let word = before.rsplit(char::is_whitespace).next().unwrap_or("");
            if before.starts_with('/') && !before.contains(char::is_whitespace) {
                let query = &before[1..];
                return commands.iter().filter(|(name, _)| name.starts_with(query)).map(|(name, detail)| Suggestion { label: format!("/{name}"), detail: detail.clone(), insert: format!("/{name} ") }).collect();
            }
            if let Some(query) = word.strip_prefix('@') {
                let query = query.to_lowercase();
                let mut hits: Vec<&String> = files.iter().filter(|path| path.to_lowercase().contains(&query)).collect();
                hits.sort_by_key(|path| (!path.to_lowercase().rsplit('/').next().unwrap_or("").starts_with(&query), path.len()));
                return hits.into_iter().take(40).map(|path| Suggestion { label: path.clone(), detail: String::new(), insert: format!("@{path} ") }).collect();
            }
            Vec::new()
        }
    }

    fn command_list(&self) -> Vec<(String, String)> {
        let mut list: Vec<(String, String)> = BUILT_IN.iter().map(|(name, detail)| (name.to_string(), detail.to_string())).collect();
        for command in &self.commands {
            if !list.iter().any(|(name, _)| name == &command.name) {
                list.push((command.name.clone(), format!("{} ({})", command.description, command.source)));
            }
        }
        list
    }

    fn choose_session(&mut self, search: Option<&str>) -> Option<String> {
        let mut list = sessions::list_sessions(self.app.state()).ok()?;
        list.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
        if let Some(search) = search {
            if let Some(found) = list.iter().find(|item| item.title.to_lowercase().contains(&search.to_lowercase())) {
                return Some(found.id.clone());
            }
        }
        let items: Vec<(String, String)> = list.iter().map(|item| (item.title.clone(), ago(item.updated_at))).collect();
        cli_ui::pick(&mut self.screen, "Resume a session", &items, None).map(|index| list[index].id.clone())
    }

    fn command(&mut self, line: &str) -> Flow {
        let (name, args) = line[1..].split_once(char::is_whitespace).map_or((&line[1..], ""), |(name, args)| (name, args.trim()));
        let note = |screen: &mut Screen, text: &str| screen.print(&format!("  {FAINT}⎿{RESET}  {text}"));
        match name {
            "exit" | "quit" | "q" => return Flow::Quit,
            "help" | "?" => {
                let mut text = format!("{BOLD}Commands{RESET}\n");
                for (name, detail) in self.command_list() {
                    text.push_str(&format!("  {CYAN}/{name:<14}{RESET} {DIM}{detail}{RESET}\n"));
                }
                text.push_str(&format!("\n{BOLD}Keys{RESET}\n  {DIM}enter send · shift+enter or \\ new line · ↑↓ history · tab complete · esc interrupt a reply\n  ctrl+l clear screen · ctrl+c twice quit · @ mention a file · type while Neru works to steer it{RESET}"));
                self.screen.print(&text);
            }
            "clear" | "new" | "reset" => match sessions::create_session(Some(false), self.app.state()) {
                Ok(snapshot) => {
                    self.session = snapshot.session.id;
                    let _ = write!(std::io::stdout(), "\x1b[2J\x1b[H");
                    let provider = self.state().provider.lock().map(|config| format!("{} · {}", config.model, config.provider_id)).unwrap_or_default();
                    let cwd = std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
                    self.screen.print(&cli_ui::welcome(&cwd, &provider, mode_name(&self.mode), VERSION));
                }
                Err(error) => note(&mut self.screen, &friendly(&error)),
            },
            "resume" => {
                if let Some(id) = self.choose_session(if args.is_empty() { None } else { Some(args) }) {
                    match sessions::select_session(id, self.app.state()) {
                        Ok(snapshot) => {
                            self.session = snapshot.session.id.clone();
                            note(&mut self.screen, &format!("Resumed “{}” ({} messages)", snapshot.session.title, snapshot.messages.len()));
                        }
                        Err(error) => note(&mut self.screen, &friendly(&error)),
                    }
                }
            }
            "model" => {
                if !args.is_empty() {
                    match switch_model(self.app, args) {
                        Ok(model) => note(&mut self.screen, &format!("Model set to {BOLD}{model}{RESET}")),
                        Err(error) => note(&mut self.screen, &friendly(&error)),
                    }
                    return Flow::Continue;
                }
                let Ok(view) = agent::provider_status(self.state()) else { return Flow::Continue };
                self.screen.footer(&[format!("{DIM}Loading models from {}…{RESET}", view.provider_id)], None);
                let models = tauri::async_runtime::block_on(crate::models::list_model_ids(view.provider_id.clone(), view.api_format.clone(), view.base_url.clone(), self.app.clone()));
                self.screen.clear();
                match models {
                    Ok(models) if !models.is_empty() => {
                        let items: Vec<(String, String)> = models.iter().map(|model| (model.clone(), String::new())).collect();
                        let current = models.iter().position(|model| *model == view.model);
                        if let Some(index) = cli_ui::pick(&mut self.screen, &format!("Model · {}", view.provider_id), &items, current) {
                            match switch_model(self.app, &models[index]) {
                                Ok(model) => note(&mut self.screen, &format!("Model set to {BOLD}{model}{RESET}")),
                                Err(error) => note(&mut self.screen, &friendly(&error)),
                            }
                        }
                    }
                    Ok(_) => note(&mut self.screen, "This provider listed no models. Set one in the Neru app → Settings → Model."),
                    Err(error) => note(&mut self.screen, &friendly(&error)),
                }
            }
            "mode" | "permissions-mode" => {
                let chosen = if args.is_empty() {
                    let items: Vec<(String, String)> = [("review", "ask before every change"), ("plan", "read and plan only"), ("edits", "apply edits, ask for commands"), ("auto", "also run routine commands"), ("bypass", "run everything except destructive commands")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
                    let current = ["manual", "plan", "accept_edits", "auto", "bypass"].iter().position(|id| *id == self.mode);
                    cli_ui::pick(&mut self.screen, "Permission mode", &items, current).map(|index| items[index].0.clone())
                } else {
                    Some(args.to_string())
                };
                if let Some(name) = chosen {
                    match mode_id(&name) {
                        Some(id) => {
                            self.mode = id.into();
                            note(&mut self.screen, &format!("Mode: {BOLD}{}{RESET}", mode_name(id)));
                        }
                        None => note(&mut self.screen, "Modes: review, plan, edits, auto, bypass"),
                    }
                }
            }
            "plan" => {
                self.mode = "plan".into();
                note(&mut self.screen, "Plan mode: Neru reads and proposes a plan without changing anything.");
                if !args.is_empty() {
                    return Flow::Send(args.to_string());
                }
            }
            "compact" => {
                self.screen.footer(&[format!("{DIM}Summarizing the conversation…{RESET}")], None);
                let result = tauri::async_runtime::block_on(agent::compact_session(self.session.clone(), self.app.clone()));
                self.screen.clear();
                match result {
                    Ok(usage) => note(&mut self.screen, &format!("Compacted. Context is now {} tokens.", usage.used)),
                    Err(error) => note(&mut self.screen, &friendly(&error)),
                }
            }
            "cost" | "context" | "status" => {
                let usage = agent::session_context(self.session.clone(), self.state()).ok().or_else(|| self.context.clone());
                let provider = agent::provider_status(self.state()).ok();
                let mut text = String::new();
                if name == "status" {
                    let cwd = std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
                    text.push_str(&format!("{DIM}cwd{RESET}      {cwd}\n{DIM}model{RESET}    {}\n{DIM}mode{RESET}     {}\n{DIM}web{RESET}      {}\n{DIM}session{RESET}  {}\n", provider.as_ref().map_or("none".into(), |view| format!("{} · {}", view.model, view.provider_id)), mode_name(&self.mode), if self.web { "on" } else { "off" }, self.session));
                }
                if let Some(usage) = usage {
                    let percent = usage.used * 100 / usage.limit.max(1);
                    let bar = 30;
                    let filled = (percent * bar / 100).min(bar);
                    text.push_str(&format!("{DIM}context{RESET}  {MOSS}{}{FAINT}{}{RESET} {} / {} tokens ({percent}%)\n", "█".repeat(filled), "░".repeat(bar - filled), usage.used, usage.limit));
                    for quota in &usage.quotas {
                        text.push_str(&format!("{DIM}{:<8}{RESET} {} of {} left{}\n", quota.label, quota.remaining.map_or("?".into(), |n| n.to_string()), quota.limit.map_or("?".into(), |n| n.to_string()), quota.resets_in.as_ref().map(|t| format!(", resets in {t}")).unwrap_or_default()));
                    }
                }
                self.screen.print(text.trim_end());
            }
            "todos" => {
                let todos = sessions::runtime(&self.state(), &self.session).ok().and_then(|shared| shared.lock().ok().map(|runtime| runtime.todos.clone())).unwrap_or_default();
                if todos.is_empty() {
                    note(&mut self.screen, "No to-do list yet. Neru keeps one for work with several steps.");
                } else {
                    let text = todos.iter().map(|todo| match todo.status.as_str() {
                        "completed" => format!("  {GREEN}☒{RESET} {DIM}{STRIKE}{}{RESET}", todo.content),
                        "in_progress" => format!("  {SAGE}☐ {BOLD}{}{RESET}", todo.content),
                        _ => format!("  ☐ {}", todo.content),
                    }).collect::<Vec<_>>().join("\n");
                    self.screen.print(&text);
                }
            }
            "memory" => {
                let scope = if args.starts_with("user") { "user" } else { "project" };
                match crate::extras::open_memory_file(scope.into(), self.app.state()) {
                    Ok(path) => note(&mut self.screen, &format!("Opened {path} in your editor. {}", if scope == "project" { "`/memory user` opens your personal memory." } else { "" })),
                    Err(error) => note(&mut self.screen, &friendly(&error)),
                }
            }
            "permissions" | "allowed-tools" => {
                let rules = agent::permission_rules(self.state()).unwrap_or_default();
                if let Some(number) = args.strip_prefix("revoke").map(str::trim).and_then(|n| n.parse::<usize>().ok()) {
                    match rules.get(number.wrapping_sub(1)) {
                        Some(rule) => {
                            let _ = agent::revoke_permission(rule.clone(), self.state());
                            note(&mut self.screen, &format!("Revoked {rule}"));
                        }
                        None => note(&mut self.screen, &format!("There is no rule {number}.")),
                    }
                } else if rules.is_empty() {
                    note(&mut self.screen, "Nothing is always allowed here. Choose “don't ask again” on an approval to add a rule.");
                } else {
                    let text = rules.iter().enumerate().map(|(i, rule)| format!("  {DIM}{}.{RESET} {rule}", i + 1)).collect::<Vec<_>>().join("\n");
                    self.screen.print(&format!("{text}\n  {DIM}Revoke one with /permissions revoke <number>{RESET}"));
                }
            }
            "doctor" => {
                self.screen.footer(&[format!("{DIM}Checking your setup…{RESET}")], None);
                let checks = tauri::async_runtime::block_on(crate::extras::doctor(self.app.clone())).unwrap_or_default();
                self.screen.clear();
                let text = checks.iter().map(|check| if check.ok { format!("  {GREEN}✓{RESET} {BOLD}{}{RESET} {DIM}{}{RESET}", check.name, check.detail) } else { format!("  {YELLOW}!{RESET} {BOLD}{}{RESET} {}\n    {DIM}{}{RESET}", check.name, check.detail, check.fix) }).collect::<Vec<_>>().join("\n");
                self.screen.print(&text);
            }
            "init" => return Flow::Send("Explore this repository and write an AGENTS.md at its root for future coding sessions: what the project is, how it is organised, how to build, test and run it, coding conventions, and anything surprising. Keep it concise and factual; propose it as a file edit.".into()),
            "review" => return Flow::Send("Review the current uncommitted changes. Inspect Git status and the diff. List every concrete bug or risk with its file and line, most severe first. Do not edit files.".into()),
            "web" => {
                self.web = !self.web;
                note(&mut self.screen, &format!("Web search is {}", if self.web { "on" } else { "off" }));
            }
            "export" => {
                let Ok(snapshot) = sessions::session_snapshot(self.session.clone(), self.app.state()) else { return Flow::Continue };
                let body = snapshot.messages.iter().map(|entry| format!("## {}\n\n{}", if entry.role == "user" { "You" } else { "Neru" }, entry.content)).collect::<Vec<_>>().join("\n\n");
                let safe: String = snapshot.session.title.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').chars().take(60).collect();
                let path = format!(".neru/exports/{}.md", if safe.is_empty() { "conversation".into() } else { safe });
                match workspace::save_file(path.clone(), format!("# {}\n\n{body}\n", snapshot.session.title), self.app.state()) {
                    Ok(_) => note(&mut self.screen, &format!("Saved to {path}")),
                    Err(error) => note(&mut self.screen, &friendly(&error)),
                }
            }
            "hooks" => note(&mut self.screen, "Hooks live in .neru/hooks.json: preToolUse (exit 2 blocks), postToolUse, postEdit, userPromptSubmit, sessionStart, stop. Open it in the Neru app with /hooks, or edit it in any editor."),
            "config" | "settings" => note(&mut self.screen, "Settings are shared with the Neru app: open it → Settings. Here: /model, /mode, /web."),
            other => {
                if let Some(command) = self.commands.iter().find(|command| command.name == other) {
                    let expanded = if command.template.contains("$ARGUMENTS") { command.template.replace("$ARGUMENTS", args) } else if args.is_empty() { command.template.clone() } else { format!("{}\n\n{args}", command.template) };
                    return Flow::Send(expanded);
                }
                note(&mut self.screen, &format!("Unknown command /{other}. Type /help to see them all."));
            }
        }
        Flow::Continue
    }
}

enum Approval {
    Continue,
    Feedback(String),
    Stop,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct ContextView {
    used: usize,
}

const BUILT_IN: &[(&str, &str)] = &[
    ("help", "Show commands and keys"),
    ("model", "Pick or switch the model: /model qwen coder"),
    ("mode", "Permission mode: review, plan, edits, auto, bypass"),
    ("plan", "Switch to Plan mode, optionally with a task"),
    ("clear", "Start a fresh session"),
    ("resume", "Resume an earlier session"),
    ("compact", "Summarize older messages to free context"),
    ("cost", "Context use and remaining quota"),
    ("status", "Folder, model, mode and session"),
    ("todos", "The agent's current to-do list"),
    ("memory", "Edit what Neru remembers (/memory user)"),
    ("permissions", "List or revoke always-allowed commands"),
    ("init", "Write an AGENTS.md for this project"),
    ("review", "Review uncommitted changes"),
    ("doctor", "Check the model, tools and setup"),
    ("web", "Turn web search on or off"),
    ("export", "Save this conversation as Markdown"),
    ("hooks", "About project hooks"),
    ("exit", "Quit (the session is saved)"),
];

fn ago(seconds: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let elapsed = now.saturating_sub(seconds);
    match elapsed {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", elapsed / 60),
        3600..=86_399 => format!("{}h ago", elapsed / 3600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

/// Provider and tool errors in plain words, with what to do next.
pub fn friendly(error: &str) -> String {
    let lower = error.to_lowercase();
    let hint = if lower.contains("401") || lower.contains("invalid api key") || lower.contains("unauthorized") || lower.contains("incorrect api key") {
        "The provider rejected the API key. Check it in the Neru app → Settings → Model."
    } else if lower.contains("429") || lower.contains("rate limit") || lower.contains("quota") {
        "The model's rate limit was reached. Wait a minute, or switch with /model."
    } else if lower.contains("configure") && (lower.contains("provider") || lower.contains("api key")) {
        "No model is set up. Add a key in the Neru app → Settings → Model."
    } else if lower.contains("timed out") || lower.contains("did not respond") || lower.contains("stalled") {
        "The provider is slow or down right now. Try again, or pick another model with /model."
    } else if lower.contains("error sending request") || lower.contains("dns") || lower.contains("connect") {
        "Neru could not reach the provider. Check your internet connection."
    } else {
        ""
    };
    let first = error.lines().next().unwrap_or(error).trim();
    if hint.is_empty() { first.to_string() } else { format!("{first}\n  {DIM}{hint}{RESET}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_errors_read_well() {
        assert_eq!(mode_id("edits"), Some("accept_edits"));
        assert_eq!(mode_id("nope"), None);
        assert!(friendly("Provider error (HTTP 401): bad key").contains("rejected the API key"));
        assert_eq!(friendly("plain"), "plain");
    }
}
