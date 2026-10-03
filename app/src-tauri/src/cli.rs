//! `neru` in the terminal. The same core as the window (settings, keys, sessions, tools, approvals,
//! memory, hooks, sub-agents) runs without a window, and this module draws it the way Claude Code
//! does: a welcome box, streamed Markdown, tool rows, diffs to approve, slash commands, `!` shell
//! commands, `#` memory, and subcommands such as `neru login`, `neru mcp` and `neru update`.

use std::{
    collections::HashMap,
    io::{BufRead, IsTerminal, Read, Write},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use serde_json::{Value, json};
use tauri::{Listener, Manager};

#[path = "cli_login.rs"]
mod login;
#[path = "cli_update.rs"]
mod update;

use crate::{
    AppState, agent,
    cli_ui::{self, BOLD, CREAM, CYAN, DIM, FAINT, GREEN, Input, MOSS, RED, RESET, SAGE, STRIKE, Screen, Suggestion, YELLOW},
    commands, sessions, workspace,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// println! for command output: colors only when stdout is a terminal, so pipes and files get plain text.
macro_rules! say {
    ($($arg:tt)*) => {{
        let text = format!($($arg)*);
        if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() { println!("{text}") } else { println!("{}", cli_ui::strip(&text)) }
    }};
}

/// The same for errors on stderr.
macro_rules! esay {
    ($($arg:tt)*) => {{
        let text = format!($($arg)*);
        if std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() { eprintln!("{text}") } else { eprintln!("{}", cli_ui::strip(&text)) }
    }};
}

const ISSUES: &str = "https://github.com/DiaeEddineJamal/Neru/issues/new";

#[derive(Clone, Copy, PartialEq)]
enum Output {
    Text,
    Json,
    StreamJson,
}

/// Commands that run and exit instead of starting a session.
enum Sub {
    Login,
    Logout,
    Models(Option<String>),
    Mcp(Vec<String>),
    Sessions,
    Doctor,
    Config,
    Update,
}

struct Options {
    prompt: Option<String>,
    print: bool,
    resume_latest: bool,
    resume: Option<Option<String>>,
    mode: String,
    model: Option<String>,
    effort: Option<String>,
    output: Output,
    web: bool,
    verbose: bool,
    sub: Option<Sub>,
    /// --allowedTools / --disallowedTools / --max-turns / --fallback-model / --system-prompt / --add-dir.
    run: crate::run_options::RunOptions,
    /// --session-id: resume the session with this id, or start one with it.
    session_id: Option<String>,
}

fn usage() -> String {
    format!(
        "{MOSS}✻{RESET} {BOLD}Neru{RESET} {VERSION} {DIM}— a local-first coding agent in your terminal{RESET}

{BOLD}Usage{RESET}
  neru [prompt]                 Start a session in this folder, optionally with a first request
  neru -p \"prompt\"              Print mode: answer once and exit (reads piped stdin too)
  neru -c                       Continue the latest session in this folder
  neru -r [search]              Resume a session: pick one, or match its title or id

{BOLD}Commands{RESET}
  neru login                    Connect a model provider (several are free)
  neru logout                   Forget the saved API keys
  neru models [search]          List the models your key can use
  neru mcp list|get|add|remove  Manage MCP connectors, as claude mcp does (neru mcp help)
  neru sessions                 List this folder's sessions
  neru doctor                   Check the model, tools and setup
  neru config                   Show where settings live and what is set
  neru update                   Update neru to the latest release

{BOLD}Options{RESET}
  --permission-mode <default|plan|acceptEdits|auto|bypassPermissions>
                                How much Neru may do without asking (also --mode review|edits|bypass)
  --dangerously-skip-permissions
                                Same as --mode bypass
  --model <name>                Use this model for this run (part of a name is enough)
  --fallback-model <names>      Switch to these models first (comma-separated) when the model
                                is rate limited, out of credits or unavailable
  --effort <low|medium|high>    Reasoning effort, for models that support it
  --output-format <text|json|stream-json>
                                Print mode output (default: text)
  --append-system-prompt <text> Extra instructions for this run
  --system-prompt <text>        Use this system prompt instead of Neru's own
  --add-dir <dir>               Let Neru also read and edit this folder (repeatable)
  --session-id <uuid>           Resume this session, or start a new one with this id
  --cwd <dir>                   Work in this folder instead of the current one
  --allowedTools <rules>        Tools that run without asking, e.g. \"Bash(npm test:*) Edit\"
  --disallowedTools <rules>     Tools Neru may not use, e.g. \"Bash WebFetch\"
  --max-turns <n>               Most tool rounds per request (default 40)
  --no-web                      Turn web search and fetch off
  --verbose                     Show timings and extra detail
  -v, --version                 Show the version
  -h, --help                    Show this help
  Options also take --name=value, and may follow the prompt.

{BOLD}Environment{RESET}
  NERU_API_KEY                  Connect a model without neru login (CI, scripts); with
                                NERU_PROVIDER, NERU_MODEL and NERU_BASE_URL. Nothing is saved.
  NO_COLOR                      Plain output

In a session: / commands · @ mention files · ! run a shell command · # remember something
shift+tab switches the permission mode · esc interrupts · esc esc rewinds · ? shows shortcuts"
    )
}

/// Why parsing stopped: help or the version (stdout, exit 0), or a mistake (stderr, exit 1).
#[derive(Debug)]
enum Stop {
    Info(String),
    Usage(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Stop::Usage(message)
    }
}

impl From<&str> for Stop {
    fn from(message: &str) -> Self {
        Stop::Usage(message.to_string())
    }
}

fn value(args: &mut std::collections::VecDeque<String>, flag: &str) -> Result<String, String> {
    args.pop_front().ok_or_else(|| format!("{flag} needs a value"))
}

/// Subcommands that take their own flags, such as `neru mcp add x --transport http …`.
const RAW_SUBCOMMANDS: [&str; 1] = ["mcp"];

fn parse_args(raw: Vec<String>) -> Result<Options, Stop> {
    let mut options = Options { prompt: None, print: false, resume_latest: false, resume: None, mode: "manual".into(), model: None, effort: None, output: Output::Text, web: true, verbose: false, sub: None, run: Default::default(), session_id: None };
    let mut args: std::collections::VecDeque<String> = raw.into();
    let mut words: Vec<String> = Vec::new();
    while let Some(mut arg) = args.pop_front() {
        if words.first().is_some_and(|first| RAW_SUBCOMMANDS.contains(&first.as_str())) {
            words.push(arg);
            continue;
        }
        // Options may come anywhere, as in Claude Code; after `--`, everything is the prompt.
        if arg == "--" {
            words.extend(args.drain(..));
            break;
        }
        if !arg.starts_with('-') || arg.len() == 1 {
            words.push(arg);
            continue;
        }
        // --flag=value is the same as --flag value.
        if arg.starts_with("--") {
            if let Some((flag, rest)) = arg.clone().split_once('=') {
                args.push_front(rest.to_string());
                arg = flag.to_string();
            }
        }
        match arg.as_str() {
            "-h" | "--help" => return Err(Stop::Info(usage())),
            "-v" | "--version" => return Err(Stop::Info(format!("{VERSION} (Neru)"))),
            "-p" | "--print" => options.print = true,
            "-c" | "--continue" => options.resume_latest = true,
            "-r" | "--resume" => {
                let search = args.front().filter(|next| !next.starts_with('-')).cloned();
                if search.is_some() {
                    args.pop_front();
                }
                options.resume = Some(search);
            }
            "--mode" | "--permission-mode" => {
                let name = value(&mut args, &arg)?;
                options.mode = mode_id(&name).ok_or("Modes: review, plan, edits, auto, bypass")?.into();
            }
            "--dangerously-skip-permissions" => options.mode = "bypass".into(),
            "--model" => options.model = Some(value(&mut args, &arg)?),
            "--effort" => {
                let effort = value(&mut args, &arg)?.to_lowercase();
                if !matches!(effort.as_str(), "low" | "medium" | "high") {
                    return Err(Stop::Usage("--effort is low, medium or high".into()));
                }
                options.effort = Some(effort);
            }
            "--output-format" => {
                options.output = match value(&mut args, &arg)?.as_str() {
                    "text" => Output::Text,
                    "json" => Output::Json,
                    "stream-json" => Output::StreamJson,
                    _ => return Err(Stop::Usage("--output-format is text, json or stream-json".into())),
                };
            }
            "--append-system-prompt" => options.run.append_system_prompt = Some(value(&mut args, &arg)?),
            "--system-prompt" => options.run.system_prompt = Some(value(&mut args, &arg)?),
            "--session-id" => options.session_id = Some(value(&mut args, &arg)?),
            "--add-dir" => {
                let dir = value(&mut args, &arg)?;
                let path = std::path::Path::new(&dir).canonicalize().map_err(|e| format!("--add-dir {dir}: {e}"))?;
                if !path.is_dir() {
                    return Err(format!("--add-dir {dir} is not a folder").into());
                }
                options.run.add_dirs.push(path);
            }
            "--cwd" => {
                let dir = value(&mut args, &arg)?;
                std::env::set_current_dir(&dir).map_err(|e| format!("Could not open {dir}: {e}"))?;
            }
            "--no-web" => options.web = false,
            "--allowedTools" | "--allowed-tools" => options.run.allow.extend(crate::run_options::split_rules(&value(&mut args, &arg)?)),
            "--disallowedTools" | "--disallowed-tools" => options.run.deny.extend(crate::run_options::split_rules(&value(&mut args, &arg)?)),
            "--max-turns" => options.run.max_rounds = Some(value(&mut args, &arg)?.parse().ok().filter(|turns: &usize| *turns > 0).ok_or("--max-turns needs a number of 1 or more")?),
            "--fallback-model" => options.run.fallback_models.extend(value(&mut args, &arg)?.split(',').map(str::trim).filter(|model| !model.is_empty()).map(str::to_string)),
            "--verbose" | "--debug" => options.verbose = true,
            other => return Err(Stop::Usage(format!("error: unknown option '{other}'\nRun neru --help for the options."))),
        }
    }
    // `neru update` is a command; `neru update the readme` is a request.
    let alone = words.len() == 1;
    options.sub = match words.first().map(String::as_str) {
        Some("login") if alone => Some(Sub::Login),
        Some("logout") if alone => Some(Sub::Logout),
        Some("models") if words.len() <= 2 => Some(Sub::Models(words.get(1).cloned())),
        Some("mcp") => Some(Sub::Mcp(words[1..].to_vec())),
        Some("sessions") if alone => Some(Sub::Sessions),
        Some("doctor") if alone => Some(Sub::Doctor),
        Some("config") if alone => Some(Sub::Config),
        Some("update" | "upgrade") if alone => Some(Sub::Update),
        _ => None,
    };
    if options.sub.is_none() && !words.is_empty() {
        options.prompt = Some(words.join(" "));
    }
    if options.verbose {
        // SAFETY: set once at startup, before any other thread reads the environment.
        unsafe { std::env::set_var("NERU_DEBUG", "1") };
    }
    Ok(options)
}

/// Writes one JSON line and flushes, for --output-format json and stream-json.
fn emit_json(out: &mut std::io::Stdout, value: Value) {
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

/// Claude Code's names for the permission modes, used in machine-readable output.
fn claude_mode(id: &str) -> &'static str {
    match id {
        "plan" => "plan",
        "accept_edits" => "acceptEdits",
        "auto" => "auto",
        "bypass" => "bypassPermissions",
        _ => "default",
    }
}

fn mode_id(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "review" | "manual" | "default" => "manual",
        "plan" => "plan",
        "edits" | "accept" | "accept_edits" | "acceptedits" => "accept_edits",
        "auto" => "auto",
        "bypass" | "yolo" | "bypasspermissions" => "bypass",
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

/// The status line's left side, as Claude Code shows the permission mode under its prompt.
fn mode_badge(id: &str) -> String {
    let (color, text) = match id {
        "plan" => (CYAN, "⏸ plan mode on"),
        "accept_edits" => (GREEN, "⏵⏵ accept edits on"),
        "auto" => (YELLOW, "⏵⏵ auto mode on"),
        "bypass" => (RED, "⏵⏵ bypass permissions on"),
        _ => return format!("{DIM}? for shortcuts{RESET}"),
    };
    format!("{color}{text}{RESET} {DIM}(shift+tab to cycle){RESET}")
}

pub fn run() {
    let options = match parse_args(std::env::args().skip(1).collect()) {
        Ok(options) => options,
        Err(Stop::Info(text)) => {
            say!("{text}");
            return;
        }
        Err(Stop::Usage(message)) => {
            esay!("{message}");
            std::process::exit(1);
        }
    };
    // A self-update on Windows leaves the previous executable beside the new one (neru.exe.old
    // from install.ps1, neru.old from neru update).
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
    // Updating needs no project, settings or window.
    if matches!(options.sub, Some(Sub::Update)) {
        std::process::exit(update::run());
    }
    // The core starts GTK, which needs a display. Over SSH, in CI or in a container there is none,
    // so run again inside a virtual one (Xvfb), which needs no setup beyond installing it.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("NERU_XVFB").is_none() {
        let exe = std::env::current_exe().unwrap_or_else(|_| "neru".into());
        match std::process::Command::new("xvfb-run").arg("-a").arg(exe).args(std::env::args().skip(1)).env("NERU_XVFB", "1").status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(_) => {
                esay!("neru needs a display on Linux, and this terminal has none (SSH, CI or a container). Install Xvfb and neru uses it by itself:\n  sudo apt install xvfb                    (Debian, Ubuntu)\n  sudo dnf install xorg-x11-server-Xvfb    (Fedora, RHEL)");
                std::process::exit(1);
            }
        }
    }
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
        #[allow(unused_mut)]
        Ok(mut app) => {
            // A terminal program: no Dock icon or menu bar on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Prohibited);
            app.run(|_, _| {})
        }
        Err(error) => {
            eprintln!("Neru could not start: {error}");
            if cfg!(target_os = "linux") {
                eprintln!("The terminal version needs the WebKitGTK 4.1 runtime (sudo apt install libwebkit2gtk-4.1-0) and a desktop session.");
            }
        }
    }
}

/// Ctrl-C when the terminal is not in raw mode (print mode, a command running after approval,
/// /compact): stops the reply if one is running, as Claude Code does; otherwise, or pressed again,
/// leaves cleanly, with the session's background commands stopped and its end hooks run.
fn interrupt_on_ctrl_c(app: &tauri::AppHandle, session: &str) {
    let (app, session) = (app.clone(), session.to_string());
    tauri::async_runtime::spawn(async move {
        let mut stopped: Option<Instant> = None;
        while tokio::signal::ctrl_c().await.is_ok() {
            let running = sessions::runtime(&app.state(), &session).ok().and_then(|shared| sessions::lock(&shared).ok().map(|runtime| runtime.running)).unwrap_or(false);
            if running && stopped.is_none_or(|at| at.elapsed() > Duration::from_secs(2)) {
                let _ = agent::stop_chat(Some(session.clone()), app.state());
                stopped = Some(Instant::now());
                continue;
            }
            let _ = terminal::disable_raw_mode();
            let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
            crate::shells::kill_session(&session);
            if let Ok(root) = workspace::project_root(&app.state()) {
                crate::hooks::session_end(&root, &session, "interrupt");
            }
            std::process::exit(130);
        }
    });
}

// ---------- shared state for one CLI session ----------

struct Cli<'a> {
    app: &'a tauri::AppHandle,
    screen: Screen,
    editor: cli_ui::Editor,
    events: mpsc::Receiver<Value>,
    session: String,
    mode: String,
    /// The mode was bypass at start, so shift+tab may cycle back to it.
    bypass_allowed: bool,
    effort: Option<String>,
    /// Shell commands run with `!` since the last request: sent along with the next one.
    shell_notes: Vec<String>,
    web: bool,
    commands: Vec<commands::SlashCommand>,
    files: Vec<String>,
    context: Option<agent::ContextUsage>,
    last_interrupt: Option<Instant>,
    output: Output,
}

fn session(app: &tauri::AppHandle, options: Options) -> i32 {
    if std::env::var_os("NERU_DEBUG").is_some() {
        eprintln!("[neru] core ready");
    }
    let interactive = !options.print && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let cwd = std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
    if let Err(error) = workspace::open_project(cwd.clone(), app.state()) {
        esay!("{RED}Could not open {cwd}: {error}{RESET}");
        return 1;
    }
    if let Some(sub) = &options.sub {
        return subcommand(app, sub);
    }
    crate::run_options::set(crate::run_options::RunOptions { keep_saved_model: true, ..options.run.clone() });
    let (sender, events) = mpsc::channel();
    app.listen("agent://event", move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            let _ = sender.send(value);
        }
    });
    tauri::async_runtime::spawn(crate::mcp::start_enabled(app.clone()));

    if let Err(error) = provider_from_env(app) {
        esay!("{RED}{error}{RESET}");
        return 1;
    }
    if let Some(model) = &options.model {
        if let Err(error) = switch_model(app, model) {
            esay!("{RED}{error}{RESET}");
            return 1;
        }
    }
    let earlier = sessions::list_sessions(app.state()).unwrap_or_default();
    // Pick the session: continue, resume, or a fresh one.
    let chosen = if options.resume_latest { earlier.iter().max_by_key(|item| item.updated_at).map(|item| item.id.clone()) } else { None };
    let mut cli = Cli {
        app,
        screen: Screen::new(std::io::stdout().is_terminal()),
        editor: cli_ui::Editor::new(load_history()),
        events,
        session: String::new(),
        mode: options.mode.clone(),
        bypass_allowed: options.mode == "bypass",
        effort: options.effort.clone(),
        shell_notes: Vec::new(),
        web: options.web,
        commands: commands::list_commands(app.state()),
        files: workspace::list_project_files(app.state()).unwrap_or_default(),
        context: None,
        last_interrupt: None,
        output: options.output,
    };
    let opened = match (chosen, &options.resume) {
        _ if options.session_id.is_some() => {
            let id = options.session_id.clone().unwrap_or_default();
            if earlier.iter().any(|item| item.id == id) {
                sessions::select_session(id, app.state())
            } else {
                workspace::project_root(&app.state()).and_then(|root| sessions::create_with_id(&app.state(), &root, false, id))
            }
        }
        (Some(id), _) => sessions::select_session(id, app.state()),
        (None, Some(Some(search))) if earlier.iter().any(|item| item.id == *search) => sessions::select_session(search.clone(), app.state()),
        (None, Some(search)) if interactive => match cli.choose_session(search.as_deref()) {
            Some(id) => sessions::select_session(id, app.state()),
            None => sessions::create_session(Some(false), app.state()),
        },
        (None, Some(search)) => {
            let terms = search.as_deref().unwrap_or("").to_lowercase();
            let mut found: Vec<&sessions::SessionSummary> = earlier.iter().filter(|item| !item.title.trim().is_empty() && (terms.is_empty() || item.title.to_lowercase().contains(&terms) || item.id.starts_with(&terms))).collect();
            found.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
            match found.first() {
                Some(item) => sessions::select_session(item.id.clone(), app.state()),
                None => {
                    esay!("No conversation found to resume{}.", search.as_deref().map(|search| format!(" matching “{search}”")).unwrap_or_default());
                    return 1;
                }
            }
        }
        _ => sessions::create_session(Some(false), app.state()),
    };
    match opened {
        Ok(snapshot) => cli.session = snapshot.session.id.clone(),
        Err(error) => {
            esay!("{RED}Could not start a session: {error}{RESET}");
            return 1;
        }
    }
    interrupt_on_ctrl_c(app, &cli.session);

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
        let code = cli.print_mode(&prompt);
        cli.finish();
        return code;
    }

    let _ = crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste);
    let returning = earlier.iter().any(|item| item.id != cli.session);
    let welcome = cli.welcome(returning);
    cli.screen.print(&welcome);
    if let Ok(snapshot) = sessions::session_snapshot(cli.session.clone(), app.state()) {
        if !snapshot.messages.is_empty() {
            cli.screen.print(&format!(" {DIM}Resumed “{}” ({} messages){RESET}\n", snapshot.session.title, snapshot.messages.len()));
            for entry in snapshot.messages.iter().filter(|entry| entry.role != "note").rev().take(2).collect::<Vec<_>>().into_iter().rev() {
                let who = if entry.role == "user" { format!("{DIM}❯{RESET}") } else { format!("{CREAM}⏺{RESET}") };
                let first = entry.content.lines().next().unwrap_or("");
                cli.screen.print(&format!(" {who} {DIM}{}{RESET}", first.chars().take(cli_ui::width() - 6).collect::<String>()));
            }
            cli.screen.print("");
        }
    }
    cli.ask_trust();
    if !configured(app) {
        cli.screen.print(&format!(" {YELLOW}⚠{RESET} No model is connected yet. Run {CYAN}/login{RESET} to pick a provider (several are free).\n"));
    } else if let Ok(config) = app.state::<AppState>().provider.lock().map(|config| config.clone()) {
        // A check from the last day answers at once; otherwise one runs in the background for next
        // time, and a failure in this session switches models with a notice.
        match crate::models::cached_health(&config) {
            Some(health) if health.status == "unavailable" => cli.screen.print(&format!(
                " {YELLOW}⚠{RESET} {} does not work with this key: {}.\n   {DIM}Neru will switch to another model when it fails; pick one yourself with{RESET} /model{DIM}.{RESET}\n",
                config.model, health.reason
            )),
            Some(_) => {}
            None => {
                tauri::async_runtime::spawn(async move {
                    crate::models::health(&config, false).await;
                });
            }
        }
    }
    if let Some(prompt) = options.prompt.clone() {
        cli.screen.print(&format!("{DIM}❯{RESET} {prompt}\n"));
        cli.turn(&prompt);
    }
    loop {
        let suggest = cli.suggester();
        let prompt = cli.prompt_info();
        match cli.editor.read(&mut cli.screen, &prompt, &suggest) {
            Input::Line(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                cli.last_interrupt = None;
                cli.screen.print(&format!("{DIM}❯{RESET} {}", line.replace('\n', "\n  ")));
                if let Some(command) = line.strip_prefix('!') {
                    cli.shell(command.trim());
                } else if let Some(fact) = line.strip_prefix('#') {
                    cli.remember(fact.trim());
                } else if line.starts_with('/') {
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
            Input::CycleMode => cli.cycle_mode(),
            Input::Rewind => cli.rewind(),
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
    cli.finish();
    cli.screen.clear();
    let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
    say!("{DIM}Session saved. Continue it with {RESET}{CYAN}neru -c{RESET}");
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

/// Uses `wanted` (or the model whose id contains it). For --model it lasts this run, as in Claude
/// Code; /model also makes it the default, as choosing a model in the app does.
fn switch_model(app: &tauri::AppHandle, wanted: &str) -> Result<String, String> {
    let view = agent::provider_status(app.state())?;
    let config = app.state::<AppState>().provider.lock().map_err(|e| e.to_string())?.clone();
    let models = tauri::async_runtime::block_on(crate::models::list_model_ids(view.provider_id.clone(), view.api_format.clone(), view.base_url.clone(), app.clone())).unwrap_or_default();
    let terms: Vec<String> = wanted.to_lowercase().split_whitespace().map(str::to_string).collect();
    let chosen = models
        .iter()
        .find(|model| model.eq_ignore_ascii_case(wanted))
        .or_else(|| models.iter().find(|model| terms.iter().all(|term| model.to_lowercase().contains(term))))
        .cloned();
    if chosen.is_none() && !models.is_empty() {
        esay!("{YELLOW}warning:{RESET} {} does not list a model called “{wanted}”; trying it anyway. See neru models.", view.provider_id);
    }
    let chosen = chosen.unwrap_or_else(|| wanted.to_string());
    let format = login::format_for(&config.provider_id, &chosen, &config.base_url, &config.api_format);
    agent::configure_provider(config.provider_id, format, config.base_url, String::new(), chosen.clone(), app.state())?;
    Ok(chosen)
}

/// NERU_API_KEY, with NERU_PROVIDER, NERU_BASE_URL and NERU_MODEL, connects a model for this run
/// without `neru login`, for CI and scripts, the way ANTHROPIC_API_KEY works for Claude Code.
/// Nothing is saved.
fn provider_from_env(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(key) = std::env::var("NERU_API_KEY").ok().filter(|key| !key.trim().is_empty()) else { return Ok(()) };
    let var = |name: &str| std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let mut config = app.state::<AppState>().provider.lock().map_err(|e| e.to_string())?.clone();
    let provider_id = var("NERU_PROVIDER").or_else(|| (!config.provider_id.is_empty()).then(|| config.provider_id.clone())).ok_or("NERU_API_KEY is set: also set NERU_PROVIDER (for example groq, openrouter or openai). neru login lists them.")?;
    let preset = login::PRESETS.iter().find(|preset| preset.id == provider_id);
    let same = provider_id == config.provider_id;
    let base_url = var("NERU_BASE_URL").or_else(|| preset.map(|preset| preset.base_url.to_string())).or_else(|| same.then(|| config.base_url.clone())).ok_or(format!("Unknown provider {provider_id}: set NERU_BASE_URL to its OpenAI-compatible endpoint."))?;
    let model = var("NERU_MODEL").or_else(|| (same && !config.model.is_empty()).then(|| config.model.clone())).or_else(|| preset.map(|preset| preset.model.to_string())).unwrap_or_default();
    config.api_format = login::format_for(&provider_id, &model, &base_url, &config.api_format);
    config.provider_id = provider_id;
    config.base_url = base_url;
    config.model = model;
    config.api_key = key.trim().to_string();
    *app.state::<AppState>().provider.lock().map_err(|e| e.to_string())? = config;
    Ok(())
}

/// /model: the choice becomes the default, like picking a model in the app.
fn choose_model(app: &tauri::AppHandle, wanted: &str) -> Result<String, String> {
    crate::run_options::update(|run| run.keep_saved_model = false);
    switch_model(app, wanted)
}

/// How long ago a session was updated; its timestamps are milliseconds since 1970.
fn ago(millis: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let elapsed = now.saturating_sub(millis) / 1000;
    match elapsed {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", elapsed / 60),
        3600..=86_399 => format!("{}h ago", elapsed / 3600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

/// Runs a command through the system shell in `root`, as `!` does.
fn shell_command(command: &str, root: &Path) -> std::process::Command {
    #[cfg(windows)]
    let mut process = {
        let mut process = std::process::Command::new("cmd");
        process.arg("/C").arg(command);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = std::process::Command::new(std::env::var("SHELL").unwrap_or_else(|_| "sh".into()));
        process.arg("-c").arg(command);
        process
    };
    process.current_dir(root).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    process
}

/// Puts text on the system clipboard.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[("powershell", &["-NoProfile", "-Command", "[Console]::InputEncoding=[Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())"])]
    } else if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    };
    for (program, args) in candidates {
        let Ok(mut child) = std::process::Command::new(program).args(*args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() else { continue };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    Err("No clipboard tool was found (install wl-clipboard or xclip).".into())
}

// ---------- subcommands ----------

fn subcommand(app: &tauri::AppHandle, sub: &Sub) -> i32 {
    let mut screen = Screen::new(std::io::stdout().is_terminal());
    match sub {
        Sub::Login => {
            if !std::io::stdin().is_terminal() {
                esay!("neru login needs an interactive terminal.");
                return 2;
            }
            let _ = crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste);
            let result = login::login(app, &mut screen);
            let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
            match result {
                Ok(Some(_)) => {
                    say!("  Run {CYAN}neru{RESET} in a project folder to start.");
                    0
                }
                Ok(None) => {
                    say!("{DIM}Cancelled. Nothing changed.{RESET}");
                    1
                }
                Err(error) => {
                    esay!("{RED}✗{RESET} {}", friendly(&error));
                    1
                }
            }
        }
        Sub::Logout => match crate::settings::forget_keys(app.state()) {
            Ok(()) => {
                say!("{GREEN}✓{RESET} Forgot every saved API key. Run {CYAN}neru login{RESET} to connect again.");
                0
            }
            Err(error) => {
                esay!("{RED}✗{RESET} {error}");
                1
            }
        },
        Sub::Models(search) => {
            let Ok(view) = agent::provider_status(app.state()) else { return 1 };
            if !view.configured {
                esay!("No model is connected. Run {CYAN}neru login{RESET} first.");
                return 1;
            }
            match tauri::async_runtime::block_on(crate::models::list_models(view.provider_id.clone(), view.api_format.clone(), view.base_url.clone(), String::new(), app.clone())) {
                Ok(models) => {
                    let terms: Vec<String> = search.as_deref().unwrap_or("").to_lowercase().split_whitespace().map(str::to_string).collect();
                    say!("{BOLD}{}{RESET} {DIM}· {}{RESET}", view.provider_id, view.base_url);
                    for info in models.iter().filter(|info| terms.iter().all(|term| info.id.to_lowercase().contains(term))) {
                        let mark = if info.id == view.model { format!("{GREEN}●{RESET}") } else { " ".into() };
                        let mut detail = Vec::new();
                        if info.free {
                            detail.push("free".to_string());
                        }
                        if info.kind == "code" {
                            detail.push("coding".into());
                        }
                        if let Some(window) = info.context_window {
                            detail.push(format!("{}K", window / 1000));
                        }
                        if info.verified == "unavailable" {
                            detail.push("unavailable".into());
                        }
                        say!(" {mark} {}  {DIM}{}{RESET}", info.id, detail.join(" · "));
                    }
                    say!("\n{DIM}Switch with{RESET} neru --model <name> {DIM}or /model in a session.{RESET}");
                    0
                }
                Err(error) => {
                    esay!("{RED}✗{RESET} {}", friendly(&error));
                    1
                }
            }
        }
        Sub::Mcp(args) => mcp(app, args),
        Sub::Sessions => {
            let mut list = sessions::list_sessions(app.state()).unwrap_or_default();
            list.retain(|item| !item.title.trim().is_empty());
            list.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
            if list.is_empty() {
                say!("{DIM}No sessions in this folder yet. Run{RESET} neru {DIM}to start one.{RESET}");
            }
            for item in list.iter().take(30) {
                say!(" {DIM}{:>9}{RESET}  {}  {FAINT}{}{RESET}", ago(item.updated_at), item.title, item.id);
            }
            if !list.is_empty() {
                say!("\n{DIM}Resume one with{RESET} neru -r <title or id>");
            }
            0
        }
        Sub::Doctor => {
            let checks = tauri::async_runtime::block_on(crate::extras::doctor(app.clone())).unwrap_or_default();
            print_checks(&checks);
            if checks.iter().all(|check| check.ok) { 0 } else { 1 }
        }
        Sub::Config => {
            let view = agent::provider_status(app.state()).ok();
            let data = workspace::data_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
            let row = |name: &str, value: String| say!(" {DIM}{name:<10}{RESET} {value}");
            row("version", VERSION.into());
            row("provider", view.as_ref().map_or("none".into(), |view| format!("{} · {}", view.provider_id, view.base_url)));
            row("model", view.as_ref().map_or("none".into(), |view| if view.model.is_empty() { "none".into() } else { view.model.clone() }));
            row("api key", view.as_ref().map_or("none".into(), |view| if view.has_key { "saved (encrypted)".into() } else { "none".into() }));
            row("data", data);
            row("memory", crate::extras::user_memory_path().map(|path| path.display().to_string()).unwrap_or_default());
            row("install", format!("{:?}", update::install_kind()).to_lowercase());
            say!("\n{DIM}Settings, keys and sessions are shared with the Neru app. Change the model with{RESET} neru login {DIM}or /model.{RESET}");
            0
        }
        Sub::Update => update::run(),
    }
}

fn print_checks(checks: &[crate::extras::Check]) {
    for check in checks {
        if check.ok {
            say!("  {GREEN}✓{RESET} {BOLD}{}{RESET} {DIM}{}{RESET}", check.name, check.detail);
        } else {
            say!("  {YELLOW}!{RESET} {BOLD}{}{RESET} {}\n    {DIM}{}{RESET}", check.name, check.detail, check.fix);
        }
    }
}

const MCP_HELP: &str = "Usage:
  neru mcp list                                  Connectors and their status
  neru mcp get <name>                            One connector's settings and tools
  neru mcp add [options] <name> <command> [args…] A local server started with a command
  neru mcp add [options] <name> -- <command> [args…]
  neru mcp add --transport http <name> <url>     A hosted (Streamable HTTP) server
  neru mcp add-json <name> '<json>'              From JSON, as in .mcp.json
  neru mcp remove <name>                         Remove a connector

Options:
  -t, --transport <stdio|http>   How Neru talks to it (default: stdio, or http for a URL)
  -s, --scope <user|project>     user: every project (default); project: this project's .mcp.json
  -e, --env KEY=value            Environment variable (repeatable)
  -H, --header \"Name: value\"     Request header for hosted servers (repeatable)";

fn take(rest: &mut impl Iterator<Item = String>, inline: &Option<String>, name: &str) -> Result<String, String> {
    inline.clone().or_else(|| rest.next()).ok_or(format!("{name} needs a value"))
}

/// Reads `neru mcp add`'s arguments the way `claude mcp add` takes them. Returns the server and
/// whether it goes in the project's .mcp.json.
fn mcp_add_args(args: &[String]) -> Result<(crate::mcp::ServerConfig, bool), String> {
    let mut server = crate::mcp::ServerConfig { name: String::new(), command: String::new(), url: None, headers: HashMap::new(), args: Vec::new(), env: HashMap::new(), enabled: true };
    let mut transport = None;
    let mut project = false;
    let mut positional: Vec<String> = Vec::new();
    let mut rest = args.iter().cloned();
    while let Some(arg) = rest.next() {
        // After the name and a command, everything belongs to the command (its own flags too);
        // after a URL, options still follow (claude mcp add --transport http x <url> --header …).
        if positional.len() >= 2 && !positional[1].starts_with("http") {
            positional.push(arg);
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if arg.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (arg.clone(), None),
        };
        match flag.as_str() {
            "-t" | "--transport" => transport = Some(take(&mut rest, &inline, "--transport")?.to_lowercase()),
            "-s" | "--scope" => {
                project = match take(&mut rest, &inline, "--scope")?.to_lowercase().as_str() {
                    "project" => true,
                    // Neru's saved connectors serve every project, so local and user are the same.
                    "user" | "local" => false,
                    other => return Err(format!("Unknown scope {other}: use user or project")),
                }
            }
            "-e" | "--env" => {
                let pair = take(&mut rest, &inline, "--env")?;
                let (key, value) = pair.split_once('=').ok_or(format!("--env takes KEY=value, not {pair}"))?;
                server.env.insert(key.to_string(), value.to_string());
            }
            "-H" | "--header" => {
                let pair = take(&mut rest, &inline, "--header")?;
                let (key, value) = pair.split_once(':').ok_or(format!("--header takes \"Name: value\", not {pair}"))?;
                server.headers.insert(key.trim().to_string(), value.trim().to_string());
            }
            "--url" => {
                server.url = Some(take(&mut rest, &inline, "--url")?);
                transport.get_or_insert_with(|| "http".into());
            }
            "--" => positional.extend(rest.by_ref()),
            other if other.starts_with('-') && positional.is_empty() => return Err(format!("Unknown option {other}")),
            _ => positional.push(arg),
        }
    }
    let mut positional = positional.into_iter();
    server.name = positional.next().ok_or("Give the connector a name")?;
    let target: Vec<String> = positional.collect();
    let transport = transport.unwrap_or_else(|| if server.url.is_some() || target.first().is_some_and(|first| first.starts_with("http://") || first.starts_with("https://")) { "http".into() } else { "stdio".into() });
    match transport.as_str() {
        "http" => {
            if server.url.is_none() {
                server.url = Some(target.first().cloned().ok_or("Give the server's URL")?);
            }
        }
        "stdio" => {
            let mut target = target.into_iter();
            server.command = target.next().ok_or("Give the command that starts the server")?;
            server.args = target.collect();
        }
        "sse" => return Err("Neru speaks Streamable HTTP, not the older SSE transport. Most servers offer it at /mcp: use --transport http.".into()),
        other => return Err(format!("Unknown transport {other}: use stdio or http")),
    }
    Ok((server, project))
}

/// Adds a connector to the project's .mcp.json, the file Claude Code reads too.
fn add_to_project(app: &tauri::AppHandle, server: &crate::mcp::ServerConfig) -> Result<std::path::PathBuf, String> {
    let file = workspace::project_root(&app.state())?.join(".mcp.json");
    let mut config: Value = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_else(|| json!({}));
    if !config["mcpServers"].is_object() {
        config["mcpServers"] = json!({});
    }
    config["mcpServers"][&server.name] = match &server.url {
        Some(url) => json!({"type": "http", "url": url, "headers": server.headers}),
        None => json!({"command": server.command, "args": server.args, "env": server.env}),
    };
    std::fs::write(&file, serde_json::to_string_pretty(&config).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("Could not write {}: {e}", file.display()))?;
    Ok(file)
}

fn mcp(app: &tauri::AppHandle, args: &[String]) -> i32 {
    let print_view = |view: &crate::mcp::ServerView, full: bool| {
        let (color, status) = match view.status.as_str() {
            "connected" => (GREEN, "connected"),
            "needs_auth" => (YELLOW, "needs sign-in (open the Neru app → Settings → Connectors)"),
            "off" => (DIM, "off"),
            "error" => (RED, "error"),
            other => (DIM, other),
        };
        let target = view.config.url.clone().unwrap_or_else(|| format!("{} {}", view.config.command, view.config.args.join(" ")).trim().to_string());
        say!(" {color}●{RESET} {BOLD}{}{RESET}  {DIM}{target}{RESET}  {color}{status}{RESET}{}", view.config.name, if view.tools.is_empty() { String::new() } else { format!(" {DIM}· {} tools{RESET}", view.tools.len()) });
        if let Some(error) = &view.error {
            say!("     {RED}{}{RESET}", error.lines().next().unwrap_or(""));
        }
        if full {
            for tool in &view.tools {
                say!("     {CYAN}{}{RESET} {DIM}{}{RESET}", tool.name, tool.description.lines().next().unwrap_or(""));
            }
        }
    };
    let started = || tauri::async_runtime::block_on(crate::mcp::start_enabled(app.clone()));
    match args.first().map(String::as_str) {
        None | Some("list" | "ls") => {
            started();
            let views = crate::mcp::mcp_servers(app.clone());
            if views.is_empty() {
                say!("{DIM}No connectors yet. Add one with{RESET} neru mcp add <name> -- <command>\n\n{MCP_HELP}");
            }
            for view in &views {
                print_view(view, false);
            }
            0
        }
        Some("get") => {
            let Some(name) = args.get(1) else {
                esay!("{MCP_HELP}");
                return 2;
            };
            started();
            match crate::mcp::mcp_servers(app.clone()).iter().find(|view| &view.config.name == name) {
                Some(view) => {
                    print_view(view, true);
                    0
                }
                None => {
                    esay!("There is no connector named {name}.");
                    1
                }
            }
        }
        Some(verb @ ("add" | "add-json")) => {
            let parsed = if verb == "add" {
                mcp_add_args(&args[1..])
            } else {
                match (args.get(1), args.get(2)) {
                    (Some(name), Some(text)) => serde_json::from_str::<Value>(text).map_err(|e| format!("That is not valid JSON: {e}")).and_then(|config| {
                        let strings = |value: &Value| value.as_object().map(|map| map.iter().map(|(key, value)| (key.clone(), value.as_str().unwrap_or_default().to_string())).collect()).unwrap_or_default();
                        let server = crate::mcp::ServerConfig {
                            name: name.clone(),
                            command: config["command"].as_str().unwrap_or_default().to_string(),
                            url: config["url"].as_str().map(str::to_string),
                            headers: strings(&config["headers"]),
                            args: config["args"].as_array().map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                            env: strings(&config["env"]),
                            enabled: true,
                        };
                        if server.command.is_empty() && server.url.is_none() { Err("The JSON needs a \"command\" or a \"url\".".to_string()) } else { Ok((server, args.iter().any(|arg| arg == "--scope=project") || args.windows(2).any(|pair| matches!(pair[0].as_str(), "-s" | "--scope") && pair[1] == "project"))) }
                    }),
                    _ => Err("Usage: neru mcp add-json <name> '<json>'".into()),
                }
            };
            let (server, project) = match parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    esay!("{RED}✗{RESET} {error}\n\n{MCP_HELP}");
                    return 2;
                }
            };
            if project {
                return match add_to_project(app, &server) {
                    Ok(file) => {
                        say!("{GREEN}✓{RESET} Added {BOLD}{}{RESET} to {}. It starts once you trust this project.", server.name, workspace::shown(&file));
                        0
                    }
                    Err(error) => {
                        esay!("{RED}✗{RESET} {error}");
                        1
                    }
                };
            }
            match tauri::async_runtime::block_on(crate::mcp::mcp_save_server(server.clone(), None, app.clone())) {
                Ok(views) => {
                    say!("{GREEN}✓{RESET} Added {BOLD}{}{RESET}. The app and every neru session can use it.", server.name);
                    if let Some(view) = views.iter().find(|view| view.config.name == server.name) {
                        print_view(view, false);
                    }
                    0
                }
                Err(error) => {
                    esay!("{RED}✗{RESET} {error}");
                    1
                }
            }
        }
        Some("remove" | "rm") => {
            let Some(name) = args.get(1) else {
                esay!("{MCP_HELP}");
                return 2;
            };
            if !crate::mcp::mcp_servers(app.clone()).iter().any(|view| &view.config.name == name) {
                esay!("There is no connector named {name}. See neru mcp list.");
                return 1;
            }
            match crate::mcp::mcp_remove_server(name.clone(), app.clone()) {
                Ok(_) => {
                    say!("{GREEN}✓{RESET} Removed {name}.");
                    0
                }
                Err(error) => {
                    esay!("{RED}✗{RESET} {error}");
                    1
                }
            }
        }
        Some("help" | "--help" | "-h") => {
            say!("{MCP_HELP}");
            0
        }
        Some(other) => {
            esay!("Unknown command neru mcp {other}.\n\n{MCP_HELP}");
            2
        }
    }
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
    /// Any streamed text arrived, so the final content is already on screen.
    wrote: bool,
}

impl Live {
    fn new() -> Self {
        Self { line: String::new(), fence: None, started_block: false, tools: HashMap::new(), drafts: HashMap::new(), status: "Thinking".into(), reasoning: 0, since: Instant::now(), typed: String::new(), wrote: false }
    }
}

/// Words for the spinner while the model thinks, the way Claude Code varies its own.
const THINKING: [&str; 8] = ["Thinking", "Kneading", "Pondering", "Considering", "Working", "Folding it in", "Shaping", "Proofing"];

impl<'a> Cli<'a> {
    fn state(&self) -> tauri::State<'a, AppState> {
        self.app.state::<AppState>()
    }

    fn cwd(&self) -> String {
        std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default()
    }

    fn model_label(&self) -> String {
        self.state().provider.lock().map(|config| if config.model.is_empty() { "no model yet · /login".into() } else { format!("{} · {}", config.model, config.provider_id) }).unwrap_or_default()
    }

    fn welcome(&self, returning: bool) -> String {
        let cwd = self.cwd();
        let root = Path::new(&cwd);
        let mut tips = Vec::new();
        if !configured(self.app) {
            tips.push(format!("Run {CYAN}/login{RESET} to connect a model {DIM}(several are free){RESET}"));
        }
        if !["AGENTS.md", "CLAUDE.md", "NERU.md"].iter().any(|name| root.join(name).is_file()) {
            tips.push(format!("Run {CYAN}/init{RESET} to write an AGENTS.md for this project"));
        }
        tips.push(format!("Ask Neru to build, fix or explain something"));
        tips.push(format!("{CYAN}@{RESET} mentions a file, {CYAN}!{RESET} runs a command, {CYAN}#{RESET} remembers"));
        tips.push(format!("{CYAN}shift+tab{RESET} changes the mode, {CYAN}?{RESET} lists shortcuts"));
        let mut recent = sessions::list_sessions(self.app.state()).unwrap_or_default();
        recent.retain(|item| item.id != self.session && !item.title.trim().is_empty() && item.title != "New session");
        recent.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
        let recent = recent.iter().take(3).map(|item| (item.title.clone(), ago(item.updated_at))).collect();
        let model = self.model_label();
        cli_ui::welcome(&cli_ui::Welcome { cwd: &cwd, model: &model, mode: mode_name(&self.mode), version: VERSION, returning, tips, recent })
    }

    fn prompt_info(&self) -> cli_ui::Prompt<'static> {
        let hint = if self.mode == "plan" { "Describe what to plan · Neru reads, then proposes" } else { "Ask Neru to build, fix or explain something" };
        let model = self.state().provider.lock().map(|config| config.model.clone()).unwrap_or_default();
        let model = model.rsplit('/').next().unwrap_or(&model).to_string();
        let right = match &self.context {
            Some(usage) if usage.limit > 0 => {
                let left = 100usize.saturating_sub(usage.used * 100 / usage.limit);
                let color = if left < 15 { RED } else if left < 35 { YELLOW } else { DIM };
                format!("{DIM}{model}{RESET} {FAINT}·{RESET} {color}{left}% context left{RESET}")
            }
            _ if !model.is_empty() => format!("{DIM}{model}{RESET}"),
            _ => String::new(),
        };
        let right = self.custom_status(&model).unwrap_or(right);
        cli_ui::Prompt { hint, left: mode_badge(&self.mode), right }
    }

    /// The first line of the user's status-line command (styles.rs), given the session as JSON.
    fn custom_status(&self, model: &str) -> Option<String> {
        let root = workspace::project_root(&self.state()).ok()?;
        let command = crate::styles::status_command(&root)?;
        let input = json!({
            "session_id": self.session,
            "cwd": self.cwd(),
            "model": {"id": model, "display_name": model},
            "workspace": {"current_dir": self.cwd(), "project_dir": root.display().to_string()},
            "permission_mode": self.mode,
            "output_style": {"name": crate::styles::current(&root).name},
            "version": VERSION,
            "context": self.context.as_ref().map(|usage| json!({"used": usage.used, "limit": usage.limit})),
        });
        crate::styles::run_status_line(&root, &command, &input).map(|line| format!("{DIM}{line}{RESET}"))
    }

    /// A project's own .mcp.json servers and settings hooks run only once its folder is trusted.
    fn ask_trust(&mut self) {
        let Ok(root) = workspace::project_root(&self.state()) else { return };
        let status = crate::trust::project_trust(&root);
        if status.trusted || (status.mcp_servers.is_empty() && status.hook_events.is_empty()) {
            return;
        }
        let mut what = Vec::new();
        if !status.mcp_servers.is_empty() {
            what.push(format!("MCP servers {}", status.mcp_servers.join(", ")));
        }
        if !status.hook_events.is_empty() {
            what.push(format!("hooks on {}", status.hook_events.join(", ")));
        }
        self.screen.print(&format!(" {YELLOW}⚠{RESET} {BOLD}This project wants to run its own tools{RESET}: {}\n   {DIM}They come from .mcp.json and .claude/settings.json. Trust the folder only if you trust its code.{RESET}", what.join(" · ")));
        let choices = vec![("Trust this folder".to_string(), "run them now and next time".to_string()), ("Not now".to_string(), "Neru works without them".to_string())];
        if cli_ui::pick(&mut self.screen, "Trust this folder?", &choices, Some(1)) == Some(0) {
            match crate::trust::trust_project(&root) {
                Ok(()) => {
                    tauri::async_runtime::block_on(self.state().mcp.sync_project(Some(&root)));
                    self.note("Trusted. Its servers and hooks are on.");
                }
                Err(error) => self.note(&friendly(&error)),
            }
        }
        self.screen.print("");
    }

    /// Stops this session's background commands and runs the sessionEnd hooks.
    fn finish(&self) {
        crate::shells::kill_session(&self.session);
        if let Ok(root) = workspace::project_root(&self.state()) {
            crate::hooks::session_end(&root, &self.session, "exit");
        }
    }

    fn cycle_mode(&mut self) {
        let mut order = vec!["manual", "accept_edits", "auto", "plan"];
        if self.bypass_allowed {
            order.push("bypass");
        }
        let at = order.iter().position(|id| *id == self.mode).unwrap_or(0);
        self.mode = order[(at + 1) % order.len()].into();
    }

    fn note(&mut self, text: &str) {
        self.screen.print(&format!("  {FAINT}⎿{RESET}  {text}"));
    }

    /// `!command`: runs it in the project (esc stops it), shows the output, and hands it to Neru
    /// with the next request.
    fn shell(&mut self, command: &str) {
        if command.is_empty() {
            self.note("Type a command after !, e.g. !git status");
            return;
        }
        let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
        let mut child = match shell_command(command, &root).spawn() {
            Ok(child) => child,
            Err(error) => {
                self.note(&format!("{RED}Could not run it: {error}{RESET}"));
                return;
            }
        };
        let (sender, lines) = mpsc::channel::<String>();
        for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)].into_iter().flatten() {
            let sender = sender.clone();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(stream).lines().map_while(Result::ok) {
                    let _ = sender.send(line);
                }
            });
        }
        drop(sender);
        let started = Instant::now();
        let mut output: Vec<String> = Vec::new();
        let mut stopped = false;
        let _ = terminal::enable_raw_mode();
        let status = loop {
            while let Ok(line) = lines.try_recv() {
                output.push(cli_ui::strip(&line));
            }
            if let Ok(Some(status)) = child.try_wait() {
                break Some(status);
            }
            while event::poll(Duration::from_millis(0)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if key.kind == KeyEventKind::Press && (key.code == KeyCode::Esc || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))) {
                        let _ = child.kill();
                        stopped = true;
                    }
                }
            }
            if stopped {
                let _ = child.wait();
                break None;
            }
            let frame = cli_ui::SPINNER[(started.elapsed().as_millis() / 90) as usize % cli_ui::SPINNER.len()];
            let last = output.last().map(|line| cli_ui::strip(line)).unwrap_or_default();
            self.screen.footer(&[format!("{MOSS}{frame}{RESET} {SAGE}Running{RESET} {DIM}{command} ({} · esc to stop){RESET}", cli_ui::elapsed(started)), format!("  {FAINT}{}{RESET}", last.chars().take(cli_ui::width().saturating_sub(4)).collect::<String>())], None);
            std::thread::sleep(Duration::from_millis(40));
        };
        let _ = terminal::disable_raw_mode();
        // Give the readers a moment to drain what the command printed last.
        std::thread::sleep(Duration::from_millis(30));
        while let Ok(line) = lines.try_recv() {
            output.push(cli_ui::strip(&line));
        }
        self.screen.clear();
        let code = status.and_then(|status| status.code());
        let shown: Vec<&String> = output.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
        let mut text = String::new();
        if output.len() > shown.len() {
            text.push_str(&format!("  {FAINT}… {} earlier lines{RESET}\n", output.len() - shown.len()));
        }
        for line in &shown {
            text.push_str(&format!("  {DIM}{}{RESET}\n", line.chars().take(cli_ui::width().saturating_sub(4)).collect::<String>()));
        }
        let (color, verdict) = match (stopped, code) {
            (true, _) => (YELLOW, "stopped".to_string()),
            (_, Some(0)) => (GREEN, "done".to_string()),
            (_, Some(code)) => (RED, format!("exit code {code}")),
            _ => (RED, "ended".to_string()),
        };
        self.screen.print(&format!("{color}⏺{RESET} {BOLD}{command}{RESET} {DIM}({verdict}, {}){RESET}", cli_ui::elapsed(started)));
        if !text.is_empty() {
            self.screen.print(text.trim_end());
        }
        // The model sees the tail of the output with the next request, as Claude Code's bash mode does.
        let tail: Vec<&String> = output.iter().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
        let body = tail.iter().map(|line| line.as_str()).collect::<Vec<_>>().join("\n");
        self.shell_notes.push(format!("I ran `{command}` in the project ({verdict}). Its output{}:\n```\n{body}\n```", if output.len() > tail.len() { " (last 200 lines)" } else { "" }));
        self.note(&format!("{DIM}Neru will see this output with your next message.{RESET}"));
    }

    /// `#fact`: saves it to the project's memory, or the user's with `#user …`.
    fn remember(&mut self, fact: &str) {
        let (scope, fact) = match fact.strip_prefix("user ").or_else(|| fact.strip_prefix("me ")) {
            Some(rest) => ("user", rest.trim()),
            None => ("project", fact),
        };
        let root = workspace::project_root(&self.state()).ok();
        match crate::extras::save_memory(root.as_deref(), fact, scope) {
            Ok(message) => self.note(&format!("{GREEN}✓{RESET} {message} {DIM}· /memory to edit{RESET}")),
            Err(error) => self.note(&friendly(&error)),
        }
    }

    /// Esc twice or /rewind: pick an earlier message, drop everything after it, and optionally undo
    /// the file changes since. The message comes back into the prompt to edit and send again.
    fn rewind(&mut self) {
        let Ok(snapshot) = sessions::session_snapshot(self.session.clone(), self.app.state()) else { return };
        let asked: Vec<&sessions::TranscriptEntry> = snapshot.messages.iter().filter(|entry| entry.role == "user").collect();
        if asked.is_empty() {
            self.note("Nothing to rewind yet.");
            return;
        }
        let items: Vec<(String, String)> = asked.iter().map(|entry| (entry.content.lines().next().unwrap_or("").chars().take(70).collect(), String::new())).collect();
        let Some(index) = cli_ui::pick(&mut self.screen, "Rewind to before…", &items, Some(items.len() - 1)) else { return };
        let choices = vec![("Conversation only".to_string(), "keep the files as they are".to_string()), ("Conversation and code".to_string(), "also undo Neru's edits since".to_string())];
        let Some(choice) = cli_ui::pick(&mut self.screen, "Restore", &choices, Some(0)) else { return };
        match sessions::rewind(&self.state(), index, choice == 1) {
            Ok(result) => {
                self.note(&format!("Rewound to before “{}”{}", items[index].0, if result.restored.is_empty() { String::new() } else { format!(", restored {} files", result.restored.len()) }));
                self.editor.set_draft(&result.prompt);
                self.files = workspace::list_project_files(self.state()).unwrap_or_default();
            }
            Err(error) => self.note(&friendly(&error)),
        }
    }

    fn notes(&mut self) -> Option<String> {
        let parts: Vec<String> = self.shell_notes.drain(..).collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
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
                    if error.to_lowercase().contains("configure") {
                        self.note(&format!("Run {CYAN}/login{RESET} to connect a model."));
                    }
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
        let notes = if prompt.trim().is_empty() { None } else { self.notes() };
        let (prompt_owned, mode, web, session, effort) = (prompt.to_string(), self.mode.clone(), self.web, self.session.clone(), self.effort.clone());
        tauri::async_runtime::spawn(async move {
            let result = agent::ai_chat(prompt_owned, vec![], mode, Some(web), None, None, effort, Some(session), notes, Some("code".into()), app).await;
            let _ = done_tx.send(result);
        });
        let mut live = Live::new();
        live.status = THINKING[(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as usize) % THINKING.len()].into();
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
        if !live.wrote && !response.content.trim().is_empty() {
            // The provider sent the whole answer at the end instead of streaming it.
            for line in response.content.lines() {
                self.print_text_line(&mut live, line);
            }
        }
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
        let extra = if live.reasoning > 0 && THINKING.contains(&live.status.as_str()) { format!(" · {} chars of reasoning", live.reasoning) } else { String::new() };
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
                live.wrote |= !text.trim().is_empty();
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
                let color = match status.as_str() {
                    "error" => RED,
                    "pending" => YELLOW,
                    _ => GREEN,
                };
                self.screen.print(&format!("{color}⏺{RESET} {BOLD}{}{RESET}", label));
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
            "subagent" => {
                if let Some(request) = event["approval"]["requestId"].as_str() {
                    self.flush_text(live);
                    live.started_block = false;
                    let approve = self.approve_subagent(event["description"].as_str().unwrap_or("A sub-agent"), &event["approval"]);
                    let _ = crate::subagent::answer_subagent_approval(request.to_string(), approve);
                    self.screen.print(&if approve { format!("{GREEN}⏺{RESET} {DIM}Allowed{RESET}") } else { format!("{RED}⏺{RESET} {DIM}Declined{RESET}") });
                }
            }
            "context" => {
                if let Ok(usage) = agent::session_context(self.session.clone(), self.state()) {
                    self.context = Some(usage);
                }
            }
            _ => {}
        }
    }

    // ---------- approvals ----------

    /// A sub-agent's edit or command, asked while the turn keeps running.
    fn approve_subagent(&mut self, agent: &str, approval: &Value) -> bool {
        let label = approval["label"].as_str().unwrap_or("");
        let (title, body) = match approval["kind"].as_str().unwrap_or("") {
            "task" => (format!("{agent} wants to run a command"), format!("   {CYAN}{label}{RESET}")),
            "delete" => (format!("{agent} wants to delete {label}"), cli_ui::diff(approval["diff"].as_str().unwrap_or(""), 20)),
            "move" => (format!("{agent} wants to move {label}"), String::new()),
            "mkdir" => (format!("{agent} wants to create folder {label}"), String::new()),
            _ => (format!("{agent} wants to edit {label}"), cli_ui::diff(approval["diff"].as_str().unwrap_or(""), 60)),
        };
        let _ = terminal::disable_raw_mode();
        let width = cli_ui::width().saturating_sub(2);
        self.screen.clear();
        self.screen.print(&format!("{YELLOW}╭{}╮{RESET}", "─".repeat(width)));
        self.screen.print(&format!("{YELLOW}│{RESET} {BOLD}{title}{RESET}"));
        if !body.is_empty() {
            self.screen.print(&body);
        }
        self.screen.print(&format!("{YELLOW}╰{}╯{RESET}", "─".repeat(width)));
        let options = vec![("Yes".to_string(), String::new()), ("No".to_string(), "esc".to_string())];
        let choice = cli_ui::pick(&mut self.screen, "Allow the sub-agent to do this?", &options, Some(0));
        let _ = terminal::enable_raw_mode();
        choice == Some(0)
    }

    fn approve(&mut self, pending: &agent::PendingView) -> Approval {
        match pending.kind.as_str() {
            "plan" => return self.review_plan(pending),
            "question" => return self.answer_questions(pending),
            _ => {}
        }
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
            options.push(("Yes, and accept edits for the rest of this session".into(), "shift+tab".into()));
        }
        options.push(("No, and tell Neru what to do differently".into(), "esc".into()));
        let choice = cli_ui::pick(&mut self.screen, "Do you want to proceed?", &options, Some(0));
        match choice {
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
                let prompt = cli_ui::Prompt { hint: "What should Neru do instead? (enter to skip)", left: format!("{DIM}enter sends · empty enter stops here{RESET}"), right: String::new() };
                match self.editor.read(&mut self.screen, &prompt, &suggest) {
                    Input::Line(line) if !line.trim().is_empty() => {
                        self.screen.print(&format!("{DIM}❯{RESET} {}", line.trim()));
                        Approval::Feedback(line.trim().to_string())
                    }
                    _ => Approval::Stop,
                }
            }
        }
    }

    /// Shows the plan from exit_plan_mode and lets the user approve it or keep planning.
    fn review_plan(&mut self, pending: &agent::PendingView) -> Approval {
        let width = cli_ui::width().saturating_sub(2);
        self.screen.print(&format!("{YELLOW}╭{}╮{RESET}", "─".repeat(width)));
        self.screen.print(&format!("{YELLOW}│{RESET} {BOLD}Neru's plan{RESET}"));
        let mut fence = None;
        for line in pending.diff.as_deref().unwrap_or("").lines() {
            self.screen.print(&format!("{YELLOW}│{RESET} {}", cli_ui::markdown_line(line, &mut fence)));
        }
        self.screen.print(&format!("{YELLOW}╰{}╯{RESET}", "─".repeat(width)));
        let options = vec![
            ("Yes, and let Neru edit".to_string(), "accept edits".to_string()),
            ("Yes, and ask before each change".to_string(), "review".to_string()),
            ("No, keep planning".to_string(), "tell Neru what to change".to_string()),
        ];
        let choice = cli_ui::pick(&mut self.screen, "Would you like to proceed?", &options, Some(0));
        let session = Some(self.session.clone());
        let (result, feedback) = match choice {
            Some(0) => (crate::extras::resolve_plan(session, true, Some("accept_edits".into()), None, self.state()), None),
            Some(1) => (crate::extras::resolve_plan(session, true, Some("manual".into()), None, self.state()), None),
            _ => {
                let prompt = cli_ui::Prompt { hint: "What should change in the plan? (enter to skip)", left: format!("{DIM}enter sends · empty enter stops here{RESET}"), right: String::new() };
                let feedback = match self.editor.read(&mut self.screen, &prompt, &|_: &str| Vec::new()) {
                    Input::Line(line) if !line.trim().is_empty() => Some(line.trim().to_string()),
                    _ => None,
                };
                (crate::extras::resolve_plan(session, false, None, feedback.clone(), self.state()), feedback)
            }
        };
        match result {
            Ok(mode) => {
                let approved = mode != "plan";
                self.mode = mode;
                if approved {
                    self.screen.print(&format!("{GREEN}⏺{RESET} Plan approved {DIM}· {}{RESET}", if self.mode == "accept_edits" { "Neru edits files without asking" } else { "Neru asks before each change" }));
                    Approval::Continue
                } else if let Some(feedback) = feedback {
                    self.screen.print(&format!("{DIM}❯{RESET} {feedback}"));
                    Approval::Continue
                } else {
                    self.screen.print(&format!("{DIM}  Still planning. Tell Neru what to change.{RESET}"));
                    Approval::Stop
                }
            }
            Err(error) => {
                self.screen.print(&format!("{RED}⏺{RESET} {}", friendly(&error)));
                Approval::Stop
            }
        }
    }

    /// Asks each question from ask_user_question with a picker, plus a typed answer.
    fn answer_questions(&mut self, pending: &agent::PendingView) -> Approval {
        let questions = pending.questions.clone().unwrap_or_default();
        let mut answers = Vec::new();
        for question in &questions {
            let header = if question.header.is_empty() { String::new() } else { format!("{DIM}{}{RESET} ", question.header) };
            self.screen.print(&format!("{YELLOW}⏺{RESET} {header}{BOLD}{}{RESET}", question.question));
            let count = question.options.len();
            let answer = if question.multi_select {
                let mut chosen = vec![false; count];
                let mut at = 0;
                loop {
                    let mut items: Vec<(String, String)> = question.options.iter().zip(&chosen).map(|(option, on)| (format!("{} {}", if *on { "☒" } else { "☐" }, option.label), option.description.clone())).collect();
                    items.push(("Done".into(), "send the checked answers".into()));
                    items.push(("Other…".into(), "type your own answer".into()));
                    let picked: Vec<String> = question.options.iter().zip(&chosen).filter(|(_, on)| **on).map(|(option, _)| option.label.clone()).collect();
                    match cli_ui::pick(&mut self.screen, &format!("{} (enter checks or unchecks)", question.question), &items, Some(at)) {
                        Some(index) if index < count => {
                            chosen[index] = !chosen[index];
                            at = index;
                        }
                        Some(index) if index == count => break (!picked.is_empty()).then(|| picked.join(", ")),
                        Some(_) => break self.typed_answer().map(|typed| picked.into_iter().chain([typed]).collect::<Vec<_>>().join(", ")),
                        None => break None,
                    }
                }
            } else {
                let mut items: Vec<(String, String)> = question.options.iter().map(|option| (option.label.clone(), option.description.clone())).collect();
                items.push(("Other…".into(), "type your own answer".into()));
                match cli_ui::pick(&mut self.screen, &question.question, &items, Some(0)) {
                    Some(index) if index < count => Some(question.options[index].label.clone()),
                    Some(_) => self.typed_answer(),
                    None => None,
                }
            };
            let Some(answer) = answer else {
                let _ = workspace::reject_pending(self.state());
                self.screen.print(&format!("{RED}⏺{RESET} {DIM}Skipped the questions.{RESET}"));
                return Approval::Stop;
            };
            self.screen.print(&format!("  {FAINT}⎿{RESET}  {answer}"));
            answers.push(answer);
        }
        match crate::extras::answer_question(Some(self.session.clone()), answers, self.state()) {
            Ok(()) => Approval::Continue,
            Err(error) => {
                self.screen.print(&format!("{RED}⏺{RESET} {}", friendly(&error)));
                Approval::Stop
            }
        }
    }

    fn typed_answer(&mut self) -> Option<String> {
        let prompt = cli_ui::Prompt { hint: "Type your answer", left: format!("{DIM}enter sends · esc cancels{RESET}"), right: String::new() };
        match self.editor.read(&mut self.screen, &prompt, &|_: &str| Vec::new()) {
            Input::Line(line) if !line.trim().is_empty() => Some(line.trim().to_string()),
            _ => None,
        }
    }

    // ---------- print mode ----------

    /// `neru -p`: answers once and exits, as `claude -p` does. A call that needs approval is
    /// refused (no one can answer) and the model carries on; the refusals are reported at the end.
    fn print_mode(&mut self, prompt: &str) -> i32 {
        // /init, /review and the project's own commands work here too.
        let mut prompt = prompt.to_string();
        if let Some(name) = prompt.strip_prefix('/').and_then(|rest| rest.split_whitespace().next()).map(str::to_string) {
            if matches!(name.as_str(), "init" | "review" | "security-review") || self.commands.iter().any(|command| command.name == name) {
                if let Flow::Send(expanded) = self.command(prompt.trim()) {
                    prompt = expanded;
                }
            }
        }
        let started = Instant::now();
        let model = self.state().provider.lock().map(|config| config.model.clone()).unwrap_or_default();
        let mut out = std::io::stdout();
        if self.output == Output::StreamJson {
            let tools: Vec<&str> = Vec::new();
            emit_json(&mut out, json!({"type": "system", "subtype": "init", "session_id": self.session, "model": model, "cwd": self.cwd(), "permission_mode": claude_mode(&self.mode), "tools": tools, "version": VERSION}));
        }
        let mut denials = Vec::new();
        let mut turns = 0;
        let mut printed = String::new();
        let mut tools = Vec::new();
        let result = loop {
            let (outcome, rounds) = self.print_round(&prompt, &mut printed, &mut tools);
            turns += rounds;
            match outcome {
                Ok(response) if response.pending.is_some() && denials.len() < 20 => {
                    let Some(pending) = response.pending else { continue };
                    let _ = workspace::deny_pending(&self.state(), "Permission denied: neru -p cannot ask for approval, so this call did not run. Carry on without it, or tell the user how to allow it (--permission-mode acceptEdits or bypassPermissions, or --allowedTools).");
                    if self.output == Output::Text {
                        esay!("{YELLOW}✗{RESET} Not allowed without asking: {}", pending.label);
                    }
                    denials.push(json!({"tool_name": pending.kind, "label": pending.label}));
                    prompt = String::new();
                }
                other => break other,
            }
        };
        let duration = started.elapsed().as_millis() as u64;
        let (content, context, error) = match &result {
            Ok(response) => (response.content.clone(), serde_json::to_value(&response.context).unwrap_or(Value::Null), None),
            Err(error) => (String::new(), Value::Null, Some(cli_ui::strip(&friendly(error)))),
        };
        if result.as_ref().is_ok_and(|response| response.pending.is_some()) {
            let _ = workspace::reject_pending(self.state());
        }
        // The agent pauses at the round limit (--max-turns) and says so at the end of its reply.
        let max_turns = error.is_none() && content.trim_end().trim_end_matches('*').ends_with("tool rounds. Say “continue” to keep going.");
        let code = if error.is_some() || max_turns { 1 } else { 0 };
        if std::env::var_os("NERU_DEBUG").is_some() {
            eprintln!("[neru] done after {duration} ms");
        }
        match self.output {
            Output::Text => {
                // Some providers send the answer only at the end, without streamed pieces.
                if printed.trim().is_empty() && !content.trim().is_empty() {
                    let _ = write!(out, "{}", content.trim_end());
                }
                println!();
                if let Some(error) = &error {
                    eprintln!("{error}");
                }
                if max_turns {
                    esay!("Stopped after the most tool rounds allowed (--max-turns). Run neru -c to keep going.");
                }
            }
            _ => {
                let subtype = if error.is_some() { "error_during_execution" } else if max_turns { "error_max_turns" } else { "success" };
                let mut summary = json!({"type": "result", "subtype": subtype, "is_error": code != 0, "duration_ms": duration, "num_turns": turns.max(1), "result": content, "session_id": self.session, "permission_denials": denials, "model": model, "context": context});
                if let Some(error) = &error {
                    summary["error"] = json!(error);
                }
                if self.output == Output::Json {
                    summary["tools"] = json!(tools);
                }
                emit_json(&mut out, summary);
            }
        }
        code
    }

    /// One `ai_chat` call for print mode, streaming its events. Returns the result and how many
    /// model rounds it took.
    fn print_round(&mut self, prompt: &str, printed: &mut String, tools: &mut Vec<Value>) -> (Result<agent::AgentResponse, String>, usize) {
        let (done_tx, done_rx) = mpsc::channel();
        let app = self.app.clone();
        let notes = if prompt.trim().is_empty() { None } else { self.notes() };
        let (prompt, mode, web, session, effort) = (prompt.to_string(), self.mode.clone(), self.web, self.session.clone(), self.effort.clone());
        tauri::async_runtime::spawn(async move {
            let _ = done_tx.send(agent::ai_chat(prompt, vec![], mode, Some(web), None, None, effort, Some(session), notes, Some("code".into()), app).await);
        });
        let color = std::io::stderr().is_terminal();
        let mut out = std::io::stdout();
        let debug = std::env::var_os("NERU_DEBUG").is_some();
        let started = Instant::now();
        let mut first = true;
        let mut rounds = 0;
        // stream-json sends whole messages, as Claude Code does: text so far goes out before a tool.
        let mut pending_text = String::new();
        let session = self.session.clone();
        let flush_text = |out: &mut std::io::Stdout, text: &mut String| {
            if !text.trim().is_empty() {
                emit_json(out, json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": std::mem::take(text)}]}, "session_id": session}));
            }
            text.clear();
        };
        loop {
            // Wait first, then drain: events are queued before the reply returns, so none are missed.
            let finished = done_rx.recv_timeout(Duration::from_millis(50));
            while let Ok(event) = self.events.try_recv() {
                if event["sessionId"].as_str() != Some(self.session.as_str()) {
                    continue;
                }
                if debug {
                    if first && event["type"] != "status" {
                        first = false;
                        eprintln!("[neru] first {} event after {} ms", event["type"].as_str().unwrap_or(""), started.elapsed().as_millis());
                    }
                    if event["type"] == "reasoning" {
                        eprint!(".");
                    }
                }
                let kind = event["type"].as_str().unwrap_or("");
                if let Some(request) = event["approval"]["requestId"].as_str() {
                    // No one can answer in print mode: the sub-agent hears no and reports it.
                    let _ = crate::subagent::answer_subagent_approval(request.to_string(), false);
                    let label = event["approval"]["label"].as_str().unwrap_or("");
                    eprintln!("Declined a sub-agent's request ({label}): print mode cannot ask. Use --permission-mode or --allowedTools to let it run.");
                }
                if kind == "context" {
                    rounds += 1;
                }
                let done = kind == "tool" && event["status"] != "running";
                let failed = event["status"] == "error";
                match self.output {
                    Output::StreamJson => match kind {
                        "delta" => pending_text.push_str(event["text"].as_str().unwrap_or("")),
                        "tool" if done => {
                            flush_text(&mut out, &mut pending_text);
                            let id = event["id"].as_str().unwrap_or("").to_string();
                            emit_json(&mut out, json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": event["label"].as_str().and_then(|label| label.split_whitespace().next()).unwrap_or("tool"), "input": {"description": event["label"]}}]}, "session_id": self.session}));
                            emit_json(&mut out, json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": if failed { "failed" } else { "done" }, "is_error": failed}]}, "session_id": self.session}));
                        }
                        "notice" => emit_json(&mut out, json!({"type": "system", "subtype": "notice", "text": event["text"], "session_id": self.session})),
                        _ => {}
                    },
                    Output::Json => {
                        if done {
                            tools.push(json!({"label": event["label"], "status": event["status"]}));
                        }
                    }
                    Output::Text => match kind {
                        "delta" => {
                            let text = event["text"].as_str().unwrap_or("");
                            printed.push_str(text);
                            let _ = write!(out, "{text}");
                            let _ = out.flush();
                        }
                        "tool" if done => {
                            let label = event["label"].as_str().unwrap_or("");
                            let (mark, tone) = if failed { ("✗", RED) } else { ("⏺", DIM) };
                            if color { eprintln!("{tone}{mark} {label}{RESET}") } else { eprintln!("{mark} {label}") }
                        }
                        _ => {}
                    },
                }
            }
            match finished {
                Ok(result) => {
                    if self.output == Output::StreamJson {
                        // A provider that sends the answer only at the end still gets its message.
                        if pending_text.trim().is_empty() {
                            if let Ok(response) = &result {
                                pending_text = response.content.clone();
                            }
                        }
                        flush_text(&mut out, &mut pending_text);
                    }
                    return (result, rounds);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return (Err("The request stopped unexpectedly".into()), rounds),
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
            if let Some(found) = list.iter().find(|item| item.id == search || item.title.to_lowercase().contains(&search.to_lowercase())) {
                return Some(found.id.clone());
            }
        }
        let items: Vec<(String, String)> = list.iter().map(|item| (item.title.clone(), ago(item.updated_at))).collect();
        cli_ui::pick(&mut self.screen, "Resume a session", &items, None).map(|index| list[index].id.clone())
    }

    fn open_session(&mut self, result: Result<sessions::SessionSnapshot, String>, verb: &str) {
        match result {
            Ok(snapshot) => {
                self.session = snapshot.session.id.clone();
                self.context = agent::session_context(self.session.clone(), self.state()).ok();
                self.note(&format!("{verb} “{}” ({} messages)", snapshot.session.title, snapshot.messages.len()));
            }
            Err(error) => self.note(&friendly(&error)),
        }
    }

    fn command(&mut self, line: &str) -> Flow {
        let (name, args) = line[1..].split_once(char::is_whitespace).map_or((&line[1..], ""), |(name, args)| (name, args.trim()));
        match name {
            "exit" | "quit" | "q" => return Flow::Quit,
            "help" | "?" => {
                let mut text = format!("{BOLD}Commands{RESET}\n");
                for (name, detail) in self.command_list() {
                    text.push_str(&format!("  {CYAN}/{name:<16}{RESET} {DIM}{detail}{RESET}\n"));
                }
                text.push_str(&format!("\n{BOLD}Input{RESET}\n  {DIM}/ commands · @ mention a file · ! run a shell command · # remember something (#user … for you){RESET}\n"));
                text.push_str(&format!("\n{BOLD}Keys{RESET}\n  {DIM}enter send · shift+enter, ctrl+j or \\ new line · ↑↓ history · tab complete · shift+tab permission mode\n  esc interrupt · esc esc rewind · ctrl+l clear screen · ctrl+c twice quit · type while Neru works to steer it{RESET}"));
                self.screen.print(&text);
            }
            "clear" | "new" | "reset" => match sessions::create_session(Some(false), self.app.state()) {
                Ok(snapshot) => {
                    self.session = snapshot.session.id;
                    self.context = None;
                    self.shell_notes.clear();
                    let _ = write!(std::io::stdout(), "\x1b[2J\x1b[3J\x1b[H");
                    let welcome = self.welcome(true);
                    self.screen.print(&welcome);
                }
                Err(error) => self.note(&friendly(&error)),
            },
            "resume" | "continue" => {
                if let Some(id) = self.choose_session(if args.is_empty() { None } else { Some(args) }) {
                    let result = sessions::select_session(id, self.app.state());
                    self.open_session(result, "Resumed");
                }
            }
            "fork" | "branch" => {
                let result = sessions::fork_session(self.session.clone(), self.app.state());
                self.open_session(result, "Forked into");
            }
            "rename" => {
                if args.is_empty() {
                    self.note("Give the new title: /rename Fix the login form");
                } else {
                    match sessions::rename_session(self.session.clone(), args.to_string(), self.app.state()) {
                        Ok(summary) => self.note(&format!("Renamed to “{}”", summary.title)),
                        Err(error) => self.note(&friendly(&error)),
                    }
                }
            }
            "rewind" | "undo" | "checkpoint" => self.rewind(),
            "login" => {
                let result = login::login(self.app, &mut self.screen);
                match result {
                    Ok(Some(_)) => self.context = None,
                    Ok(None) => self.note("Cancelled. Nothing changed."),
                    Err(error) => self.note(&friendly(&error)),
                }
            }
            "logout" => match crate::settings::forget_keys(self.state()) {
                Ok(()) => self.note("Forgot every saved API key. /login connects again."),
                Err(error) => self.note(&friendly(&error)),
            },
            "model" => {
                if !args.is_empty() {
                    match choose_model(self.app, args) {
                        Ok(model) => self.note(&format!("Model set to {BOLD}{model}{RESET}")),
                        Err(error) => self.note(&friendly(&error)),
                    }
                    return Flow::Continue;
                }
                let Ok(view) = agent::provider_status(self.state()) else { return Flow::Continue };
                if !view.configured {
                    self.note(&format!("No model is connected. Run {CYAN}/login{RESET} first."));
                    return Flow::Continue;
                }
                self.screen.footer(&[format!("{DIM}Loading models from {}…{RESET}", view.provider_id)], None);
                let models = tauri::async_runtime::block_on(crate::models::list_model_ids(view.provider_id.clone(), view.api_format.clone(), view.base_url.clone(), self.app.clone()));
                self.screen.clear();
                match models {
                    Ok(models) if !models.is_empty() => {
                        let items: Vec<(String, String)> = models.iter().map(|model| (model.clone(), String::new())).collect();
                        let current = models.iter().position(|model| *model == view.model);
                        if let Some(index) = cli_ui::pick(&mut self.screen, &format!("Model · {}", view.provider_id), &items, current) {
                            match choose_model(self.app, &models[index]) {
                                Ok(model) => self.note(&format!("Model set to {BOLD}{model}{RESET}")),
                                Err(error) => self.note(&friendly(&error)),
                            }
                        }
                    }
                    Ok(_) => self.note("This provider listed no models. Type one: /model <id>, or switch provider with /login."),
                    Err(error) => self.note(&friendly(&error)),
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
                            if id == "bypass" {
                                self.bypass_allowed = true;
                            }
                            self.note(&format!("Mode: {BOLD}{}{RESET}", mode_name(id)));
                        }
                        None => self.note("Modes: review, plan, edits, auto, bypass"),
                    }
                }
            }
            "plan" => {
                self.mode = "plan".into();
                self.note("Plan mode: Neru reads and proposes a plan without changing anything.");
                if !args.is_empty() {
                    return Flow::Send(args.to_string());
                }
            }
            "effort" => {
                let wanted = args.to_lowercase();
                match wanted.as_str() {
                    "" => self.note(&format!("Effort: {BOLD}{}{RESET}. Set it with /effort low, medium, high or auto.", self.effort.as_deref().unwrap_or("auto"))),
                    "low" | "medium" | "high" => {
                        self.effort = Some(wanted.clone());
                        let supported = agent::effort_supported(self.state()).unwrap_or(false);
                        self.note(&format!("Effort: {BOLD}{wanted}{RESET}{}", if supported { "" } else { " (this model ignores it)" }));
                    }
                    "auto" | "default" => {
                        self.effort = None;
                        self.note("Effort: auto");
                    }
                    _ => self.note("Effort is low, medium, high or auto."),
                }
            }
            "compact" => {
                self.screen.footer(&[format!("{DIM}Summarizing the conversation…{RESET}")], None);
                let result = tauri::async_runtime::block_on(agent::compact_session(self.session.clone(), Some(args.to_string()), self.app.clone()));
                self.screen.clear();
                match result {
                    Ok(usage) => {
                        self.note(&format!("Compacted conversation. Context is now {} tokens.", usage.used));
                        self.context = Some(usage);
                    }
                    Err(error) => self.note(&friendly(&error)),
                }
            }
            "cost" | "usage" | "context" | "status" => {
                let usage = agent::session_context(self.session.clone(), self.state()).ok().or_else(|| self.context.clone());
                let provider = agent::provider_status(self.state()).ok();
                let mut text = String::new();
                if name == "status" {
                    let row = |label: &str, value: String| format!("{DIM}{label:<9}{RESET}{value}\n");
                    text.push_str(&row("version", VERSION.into()));
                    text.push_str(&row("cwd", self.cwd()));
                    text.push_str(&row("model", provider.as_ref().map_or("none".into(), |view| format!("{} · {}", view.model, view.provider_id))));
                    text.push_str(&row("mode", mode_name(&self.mode).into()));
                    text.push_str(&row("effort", self.effort.clone().unwrap_or_else(|| "auto".into())));
                    text.push_str(&row("web", if self.web { "on".into() } else { "off".into() }));
                    text.push_str(&row("session", self.session.clone()));
                    let connectors = crate::mcp::mcp_servers(self.app.clone());
                    text.push_str(&row("mcp", if connectors.is_empty() { "none".into() } else { connectors.iter().map(|view| format!("{} ({})", view.config.name, view.status)).collect::<Vec<_>>().join(", ") }));
                }
                if let Some(usage) = usage {
                    let percent = usage.used * 100 / usage.limit.max(1);
                    let bar = 30;
                    let filled = (percent * bar / 100).min(bar);
                    let color = if percent > 85 { RED } else if percent > 65 { YELLOW } else { MOSS };
                    text.push_str(&format!("{DIM}context{RESET}  {color}{}{FAINT}{}{RESET} {} / {} tokens ({percent}%)\n", "█".repeat(filled), "░".repeat(bar - filled), usage.used, usage.limit));
                    if usage.window != usage.limit && usage.window > 0 {
                        text.push_str(&format!("{DIM}window{RESET}   {} tokens {DIM}(this key allows {} per request){RESET}\n", usage.window, usage.limit));
                    }
                    for quota in &usage.quotas {
                        text.push_str(&format!("{DIM}{:<8}{RESET} {} of {} left{}\n", quota.label, quota.remaining.map_or("?".into(), |n| n.to_string()), quota.limit.map_or("?".into(), |n| n.to_string()), quota.resets_in.as_ref().map(|t| format!(", resets in {t}")).unwrap_or_default()));
                    }
                    if percent > 70 {
                        text.push_str(&format!("{DIM}Run /compact to summarize older messages and free space.{RESET}\n"));
                    }
                } else if name != "status" {
                    text.push_str(&format!("{DIM}No requests yet in this session.{RESET}"));
                }
                self.screen.print(text.trim_end());
            }
            "todos" => {
                let todos = sessions::runtime(&self.state(), &self.session).ok().and_then(|shared| shared.lock().ok().map(|runtime| runtime.todos.clone())).unwrap_or_default();
                if todos.is_empty() {
                    self.note("No to-do list yet. Neru keeps one for work with several steps.");
                } else {
                    let text = todos
                        .iter()
                        .map(|todo| match todo.status.as_str() {
                            "completed" => format!("  {GREEN}☒{RESET} {DIM}{STRIKE}{}{RESET}", todo.content),
                            "in_progress" => format!("  {SAGE}☐ {BOLD}{}{RESET}", todo.content),
                            _ => format!("  ☐ {}", todo.content),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.screen.print(&text);
                }
            }
            "memory" => {
                let scope = if args.starts_with("user") { "user" } else { "project" };
                match crate::extras::open_memory_file(scope.into(), self.app.state()) {
                    Ok(path) => self.note(&format!("Opened {path} in your editor. {}", if scope == "project" { "`/memory user` opens your personal memory. `#` adds a line." } else { "" })),
                    Err(error) => self.note(&friendly(&error)),
                }
            }
            "permissions" | "allowed-tools" => {
                let rules = agent::permission_rules(self.state()).unwrap_or_default();
                if let Some(number) = args.strip_prefix("revoke").map(str::trim).and_then(|n| n.parse::<usize>().ok()) {
                    match rules.get(number.wrapping_sub(1)) {
                        Some(rule) => {
                            let _ = agent::revoke_permission(rule.clone(), self.state());
                            self.note(&format!("Revoked {rule}"));
                        }
                        None => self.note(&format!("There is no rule {number}.")),
                    }
                } else if rules.is_empty() {
                    self.note("Nothing is always allowed here. Choose “don't ask again” on an approval to add a rule.");
                } else {
                    let text = rules.iter().enumerate().map(|(i, rule)| format!("  {DIM}{}.{RESET} {rule}", i + 1)).collect::<Vec<_>>().join("\n");
                    self.screen.print(&format!("{text}\n  {DIM}Revoke one with /permissions revoke <number>{RESET}"));
                }
            }
            "mcp" => {
                if let Some(target) = args.strip_prefix("restart").map(str::trim).filter(|name| !name.is_empty()) {
                    match tauri::async_runtime::block_on(crate::mcp::mcp_restart(target.to_string(), self.app.clone())) {
                        Ok(_) => self.note(&format!("Restarted {target}")),
                        Err(error) => self.note(&friendly(&error)),
                    }
                    return Flow::Continue;
                }
                let views = crate::mcp::mcp_servers(self.app.clone());
                if views.is_empty() {
                    self.note("No connectors yet. Add one with `neru mcp add <name> -- <command>`, or in the app → Settings → Connectors.");
                } else {
                    let text = views
                        .iter()
                        .map(|view| {
                            let color = match view.status.as_str() {
                                "connected" => GREEN,
                                "error" => RED,
                                "needs_auth" => YELLOW,
                                _ => DIM,
                            };
                            format!("  {color}●{RESET} {BOLD}{}{RESET} {DIM}{} · {} tools{RESET}", view.config.name, view.status.replace('_', " "), view.tools.len())
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.screen.print(&format!("{text}\n  {DIM}/mcp restart <name> · neru mcp add|remove from a shell{RESET}"));
                }
            }
            "skills" => {
                let skills = crate::skills::list_skills(self.state());
                if skills.is_empty() {
                    self.note("No skills yet. Add them in the app → Settings → Skills, or in .neru/skills.");
                } else {
                    let text = skills.iter().filter(|skill| skill.enabled).map(|skill| format!("  {CYAN}{:<30}{RESET} {DIM}{}{RESET}", skill.name, skill.description.chars().take(cli_ui::width().saturating_sub(36)).collect::<String>())).collect::<Vec<_>>().join("\n");
                    self.screen.print(&format!("{text}\n  {DIM}Neru loads a skill when a task matches it; ask for one by name to use it now.{RESET}"));
                }
            }
            "init" => return Flow::Send("Explore this repository and write an AGENTS.md at its root for future coding sessions: what the project is, how it is organised, how to build, test and run it, coding conventions, and anything surprising. Keep it concise and factual; propose it as a file edit.".into()),
            "review" => return Flow::Send(format!("Review the current uncommitted changes{}. Inspect Git status and the diff. List every concrete bug or risk with its file and line, most severe first. Do not edit files.", if args.is_empty() { String::new() } else { format!(" ({args})") })),
            "security-review" => return Flow::Send("Do a security review of the pending changes on this branch (uncommitted changes and commits not yet on the main branch). Look for injection, authentication and authorization gaps, secrets in code, unsafe deserialization, path traversal, SSRF, XSS and insecure defaults. For each finding give the file and line, why it is exploitable, and a fix. Say plainly if you find nothing. Do not edit files.".into()),
            "pr-comments" => return Flow::Send("Fetch the review comments on this branch's pull request (use the gh CLI), summarize each with its file and line, and say which need changes.".into()),
            "diff" => {
                let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
                match crate::git::git(&root, &["status", "--short"]) {
                    Ok(status) if status.trim().is_empty() => self.note("No uncommitted changes."),
                    Ok(status) => {
                        let stat = crate::git::git(&root, &["diff", "--stat", "HEAD"]).unwrap_or_default();
                        let colored = status
                            .lines()
                            .map(|line| {
                                let color = if line.starts_with("??") || line.starts_with('A') { GREEN } else if line.contains('D') { RED } else { YELLOW };
                                format!("  {color}{}{RESET}{}", &line[..2.min(line.len())], line.get(2..).unwrap_or(""))
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let summary = stat.lines().last().unwrap_or("").trim().to_string();
                        self.screen.print(&format!("{colored}\n  {DIM}{summary}{RESET}"));
                    }
                    Err(error) => self.note(&friendly(&error)),
                }
            }
            "output-style" => {
                let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
                if args.trim().is_empty() {
                    let current = crate::styles::current(&root).name;
                    let styles = crate::styles::list(&root);
                    let rows: Vec<(String, String)> = styles.iter().map(|style| (style.name.clone(), style.description.clone())).collect();
                    let selected = styles.iter().position(|style| style.name == current);
                    if let Some(index) = cli_ui::pick(&mut self.screen, "Output style", &rows, selected) {
                        match crate::styles::choose(&root, &styles[index].name) {
                            Ok(style) => self.note(&format!("Output style: {}. Saved in .claude/settings.local.json.", style.name)),
                            Err(error) => self.note(&error),
                        }
                    }
                } else {
                    match crate::styles::choose(&root, args) {
                        Ok(style) => self.note(&format!("Output style: {}. Saved in .claude/settings.local.json.", style.name)),
                        Err(error) => self.note(&format!("{error}. /output-style lists them.")),
                    }
                }
            }
            "statusline" => {
                let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
                let text = args.trim();
                if text.is_empty() {
                    match crate::styles::status_command(&root) {
                        Some(command) => self.note(&format!("Status line: {command}
  /statusline <command> sets another; /statusline off removes it. The command gets the session as JSON on stdin.")),
                        None => self.note("No custom status line. /statusline <command> shows the first line a command prints, like Claude Code's statusLine setting (it gets the session as JSON on stdin)."),
                    }
                } else {
                    let command = (!matches!(text, "off" | "none" | "remove")).then_some(text);
                    match crate::styles::set_status_command(&root, command) {
                        Ok(()) if command.is_some() && !crate::trust::is_trusted(&root) => self.note("Saved in .claude/settings.local.json. A project's status line runs once the folder is trusted: /trust."),
                        Ok(()) => self.note(if command.is_some() { "Status line saved in .claude/settings.local.json." } else { "Custom status line removed." }),
                        Err(error) => self.note(&error),
                    }
                }
            }
            "trust" | "untrust" => {
                let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
                let result = if name == "trust" { crate::trust::trust_project(&root) } else { crate::trust::untrust_project(&root) };
                tauri::async_runtime::block_on(self.state().mcp.sync_project(Some(&root)));
                match result {
                    Ok(()) if name == "trust" => self.note("Trusted this folder: its .mcp.json connectors, settings hooks and allow rules now apply."),
                    Ok(()) => self.note("Stopped trusting this folder: its .mcp.json connectors, settings hooks and allow rules are off."),
                    Err(error) => self.note(&error),
                }
            }
            "doctor" => {
                self.screen.footer(&[format!("{DIM}Checking your setup…{RESET}")], None);
                let checks = tauri::async_runtime::block_on(crate::extras::doctor(self.app.clone())).unwrap_or_default();
                self.screen.clear();
                let text = checks.iter().map(|check| if check.ok { format!("  {GREEN}✓{RESET} {BOLD}{}{RESET} {DIM}{}{RESET}", check.name, check.detail) } else { format!("  {YELLOW}!{RESET} {BOLD}{}{RESET} {}\n    {DIM}{}{RESET}", check.name, check.detail, check.fix) }).collect::<Vec<_>>().join("\n");
                self.screen.print(&text);
            }
            "web" => {
                self.web = !self.web;
                self.note(&format!("Web search is {}", if self.web { "on" } else { "off" }));
            }
            "export" => {
                let Ok(snapshot) = sessions::session_snapshot(self.session.clone(), self.app.state()) else { return Flow::Continue };
                let body = snapshot.messages.iter().map(|entry| match entry.role.as_str() { "note" => format!("_{}_", entry.content), "user" => format!("## You\n\n{}", entry.content), _ => format!("## Neru\n\n{}", entry.content) }).collect::<Vec<_>>().join("\n\n");
                let safe: String = snapshot.session.title.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').chars().take(60).collect();
                let path = if args.is_empty() { format!(".neru/exports/{}.md", if safe.is_empty() { "conversation".into() } else { safe }) } else { args.to_string() };
                match workspace::save_file(path.clone(), format!("# {}\n\n{body}\n", snapshot.session.title), self.app.state()) {
                    Ok(_) => self.note(&format!("Saved to {path}")),
                    Err(error) => self.note(&friendly(&error)),
                }
            }
            "copy" => {
                let Ok(snapshot) = sessions::session_snapshot(self.session.clone(), self.app.state()) else { return Flow::Continue };
                match snapshot.messages.iter().rev().find(|entry| entry.role == "assistant" && !entry.content.trim().is_empty()) {
                    Some(entry) => match copy_to_clipboard(&entry.content) {
                        Ok(()) => self.note(&format!("Copied Neru's last reply ({} lines)", entry.content.lines().count())),
                        Err(error) => self.note(&error),
                    },
                    None => self.note("There is no reply to copy yet."),
                }
            }
            "release-notes" | "changelog" => {
                let entries: Vec<Value> = serde_json::from_str(include_str!("../../src/changelog.json")).unwrap_or_default();
                let mut text = String::new();
                for entry in entries.iter().take(if args == "all" { 20 } else { 1 }) {
                    text.push_str(&format!("{MOSS}✻{RESET} {BOLD}{}{RESET} {DIM}· {}{RESET}\n  {CREAM}{}{RESET}\n", entry["version"].as_str().unwrap_or(""), entry["date"].as_str().unwrap_or(""), entry["title"].as_str().unwrap_or("")));
                    for section in entry["sections"].as_array().into_iter().flatten() {
                        text.push_str(&format!("\n  {SAGE}{}{RESET}\n", section["title"].as_str().unwrap_or("")));
                        for item in section["items"].as_array().into_iter().flatten() {
                            let item = item.as_str().unwrap_or("");
                            text.push_str(&format!("  {DIM}•{RESET} {}\n", cli_ui::markdown_line(item, &mut None)));
                        }
                    }
                    text.push('\n');
                }
                if args != "all" {
                    text.push_str(&format!("{DIM}/release-notes all shows earlier versions.{RESET}"));
                }
                self.screen.print(text.trim_end());
            }
            "update" | "upgrade" => {
                self.screen.clear();
                update::run();
            }
            "bug" | "feedback" => {
                let url = format!("{ISSUES}?labels=bug&body={}", format!("Neru CLI {VERSION} on {} {}\n\nWhat happened:\n", std::env::consts::OS, std::env::consts::ARCH).replace(' ', "%20").replace('\n', "%0A"));
                match crate::web::open_url(url) {
                    Ok(()) => self.note("Opened a new issue on GitHub in your browser."),
                    Err(_) => self.note(&format!("Open {ISSUES} to report it.")),
                }
            }
            "terminal-setup" => {
                let text = if cfg!(windows) {
                    "Windows Terminal and PowerShell send shift+enter to Neru as a new line already. In VS Code's terminal, add a keybinding for shift+enter that sends \"\\u001b\\r\" (workbench.action.terminal.sendSequence), or use ctrl+j or \\ then enter."
                } else if cfg!(target_os = "macos") {
                    "iTerm2, Ghostty, Kitty and WezTerm send shift+enter as a new line. In Terminal.app, turn on Settings → Profiles → Keyboard → Use Option as Meta key and use option+enter, or use ctrl+j or \\ then enter."
                } else {
                    "Most terminals send shift+enter as a new line. If yours does not, use ctrl+j, or type \\ and then enter."
                };
                self.note(text);
            }
            "agents" => {
                let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
                let agents = crate::agents::list_agents_for(&root);
                let mut text = format!("  {CYAN}{:<20}{RESET} {DIM}built in: explores and plans in parallel{RESET}\n", "explore · plan · general");
                for agent in &agents {
                    text.push_str(&format!("  {CYAN}{:<20}{RESET} {DIM}{} ({}){RESET}\n", agent.name, agent.description.chars().take(cli_ui::width().saturating_sub(40)).collect::<String>(), agent.source));
                }
                text.push_str(&format!("  {DIM}Add your own in .neru/agents or .claude/agents: a Markdown file with name, description and tools front matter.{RESET}"));
                self.screen.print(&text);
            }
            "bashes" | "shells" => {
                if let Some(id) = args.strip_prefix("kill").map(str::trim).filter(|id| !id.is_empty()) {
                    match crate::shells::kill(&self.session, id) {
                        Ok(message) => self.note(&message),
                        Err(error) => self.note(&friendly(&error)),
                    }
                    return Flow::Continue;
                }
                let shells = crate::shells::list(Some(&self.session));
                if shells.is_empty() {
                    self.note("No background commands. Neru starts dev servers and watchers in the background when you ask it to run one.");
                } else {
                    let text = shells.iter().map(|shell| format!("  {}●{RESET} {BOLD}{}{RESET} {}  {DIM}{} · {}s · {} lines{RESET}", if shell.status == "running" { GREEN } else { DIM }, shell.id, shell.command, shell.status, shell.elapsed_secs, shell.lines)).collect::<Vec<_>>().join("\n");
                    self.screen.print(&format!("{text}\n  {DIM}/bashes kill <id> stops one{RESET}"));
                }
            }
            "hooks" => self.note("Hooks live in .neru/hooks.json, or in the \"hooks\" block of .claude/settings.json the way Claude Code writes them: PreToolUse (exit 2 or {\"decision\":\"block\"} stops a call), PostToolUse, UserPromptSubmit, SessionStart, SessionEnd, Stop, SubagentStop, PreCompact, Notification."),
            "config" | "settings" => {
                let data = workspace::data_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
                self.note(&format!("Settings are shared with the Neru app ({data}). Here: /login, /model, /mode, /effort, /web. `neru config` prints them all."));
            }
            other => {
                if let Some(command) = self.commands.iter().find(|command| command.name == other) {
                    let root = workspace::project_root(&self.state()).unwrap_or_else(|_| std::path::PathBuf::from(self.cwd()));
                    let (prompt, run) = tauri::async_runtime::block_on(crate::commands::prepare(command, args, &root));
                    if let Ok(shared) = sessions::runtime(&self.state(), &self.session) {
                        if let Ok(mut runtime) = sessions::lock(&shared) {
                            runtime.command_run = Some(run);
                        }
                    }
                    return Flow::Send(prompt);
                }
                self.note(&format!("Unknown command /{other}. Type /help to see them all."));
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

const BUILT_IN: &[(&str, &str)] = &[
    ("help", "Show commands and keys"),
    ("login", "Connect a model provider"),
    ("logout", "Forget the saved API keys"),
    ("model", "Pick or switch the model: /model qwen coder"),
    ("mode", "Permission mode: review, plan, edits, auto, bypass"),
    ("plan", "Switch to Plan mode, optionally with a task"),
    ("effort", "Reasoning effort: low, medium, high or auto"),
    ("clear", "Start a fresh session"),
    ("resume", "Resume an earlier session"),
    ("fork", "Continue in a copy of this session"),
    ("rename", "Rename this session"),
    ("rewind", "Go back to before a message (esc esc)"),
    ("compact", "Summarize older messages to free context"),
    ("context", "Context use and remaining quota"),
    ("cost", "Context use and remaining quota"),
    ("status", "Version, folder, model, mode and connectors"),
    ("todos", "The agent's current to-do list"),
    ("memory", "Edit what Neru remembers (/memory user)"),
    ("permissions", "List or revoke always-allowed commands"),
    ("mcp", "Connectors and their status"),
    ("skills", "Skills Neru can load"),
    ("init", "Write an AGENTS.md for this project"),
    ("review", "Review uncommitted changes"),
    ("security-review", "Security review of this branch's changes"),
    ("pr-comments", "Summarize this branch's PR review comments"),
    ("diff", "Uncommitted changes at a glance"),
    ("doctor", "Check the model, tools and setup"),
    ("output-style", "How Neru writes: default, explanatory, learning or your own"),
    ("statusline", "Show a command's output in the status line"),
    ("trust", "Trust this folder's connectors, hooks and rules"),
    ("untrust", "Stop trusting this folder"),
    ("web", "Turn web search on or off"),
    ("export", "Save this conversation as Markdown"),
    ("copy", "Copy Neru's last reply"),
    ("release-notes", "What's new in this version"),
    ("update", "Update neru to the latest release"),
    ("bug", "Report a problem on GitHub"),
    ("terminal-setup", "How to get shift+enter for new lines"),
    ("agents", "Sub-agents Neru can hand work to"),
    ("bashes", "Background commands (/bashes kill <id>)"),
    ("hooks", "About project hooks"),
    ("config", "Where settings live"),
    ("exit", "Quit (the session is saved)"),
];

/// Provider and tool errors in plain words, with what to do next.
pub fn friendly(error: &str) -> String {
    let lower = error.to_lowercase();
    let locked = crate::fallback::is_client_locked(error).then(|| format!("{}. Switch with /model or /login.", crate::models::client_locked_reason(error)));
    let hint = if let Some(locked) = &locked {
        locked.as_str()
    } else if crate::fallback::needs_credits(error) {
        "This account has no credits for the model. Add credits, or pick a free model with /model."
    } else if lower.contains("401") || lower.contains("invalid api key") || lower.contains("unauthorized") || lower.contains("incorrect api key") {
        "The provider rejected the API key. Run /login to enter it again."
    } else if lower.contains("429") || lower.contains("rate limit") || lower.contains("quota") {
        "The model's rate limit was reached. Wait a minute, or switch with /model."
    } else if lower.contains("configure") && (lower.contains("provider") || lower.contains("api key")) {
        "No model is set up. Run /login (or neru login) to connect one."
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

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn modes_and_errors_read_well() {
        assert_eq!(mode_id("edits"), Some("accept_edits"));
        assert_eq!(mode_id("nope"), None);
        assert!(friendly("Provider error (HTTP 401): bad key").contains("rejected the API key"));
        assert!(friendly("Provider HTTP 403 Forbidden: Free tier can only be used from within OpenCode").contains("only works inside OpenCode"));
        assert!(friendly("Provider HTTP 402 Payment Required: no funds").contains("no credits"));
        assert_eq!(friendly("plain"), "plain");
    }

    #[test]
    fn subcommands_only_when_alone() {
        assert!(matches!(parse_args(args("update")).unwrap().sub, Some(Sub::Update)));
        let request = parse_args(args("update the readme")).unwrap();
        assert!(request.sub.is_none());
        assert_eq!(request.prompt.as_deref(), Some("update the readme"));
        assert!(matches!(parse_args(args("mcp add docs -- npx -y @docs/mcp")).unwrap().sub, Some(Sub::Mcp(list)) if list.len() == 6));
        assert!(matches!(parse_args(args("models qwen")).unwrap().sub, Some(Sub::Models(Some(_)))));
    }

    #[test]
    fn flags_parse() {
        let options = parse_args(args("-p --output-format json --dangerously-skip-permissions --effort high hello there")).unwrap();
        assert!(options.print);
        assert!(options.output == Output::Json);
        assert_eq!(options.mode, "bypass");
        assert_eq!(options.effort.as_deref(), Some("high"));
        assert_eq!(options.prompt.as_deref(), Some("hello there"));
        assert!(parse_args(args("--effort extreme")).is_err());
        assert_eq!(parse_args(args("--permission-mode plan")).unwrap().mode, "plan");
        let limited = parse_args(args("--allowedTools Edit,Read --disallowedTools Bash --max-turns 5 -p hi")).unwrap();
        assert_eq!(limited.run.allow, vec!["Edit", "Read"]);
        assert_eq!(limited.run.deny, vec!["Bash"]);
        assert_eq!(limited.run.max_rounds, Some(5));
        let custom = parse_args(args("--system-prompt be-brief --session-id 2b3c --add-dir . -p hi")).unwrap();
        assert_eq!(custom.run.system_prompt.as_deref(), Some("be-brief"));
        assert_eq!(custom.session_id.as_deref(), Some("2b3c"));
        assert_eq!(custom.run.add_dirs.len(), 1);
        assert!(parse_args(args("--add-dir ./no-such-folder-here -p hi")).is_err());
        let fallback = parse_args(args("--fallback-model qwen3-coder,glm-4.6 --fallback-model kimi -p hi")).unwrap();
        assert_eq!(fallback.run.fallback_models, vec!["qwen3-coder", "glm-4.6", "kimi"]);
    }

    #[test]
    fn flags_parse_like_claude_code() {
        // --flag=value, options after the prompt, and `--` before a prompt that looks like a flag.
        let options = parse_args(args("-p hello --output-format=json --model=qwen there")).unwrap();
        assert!(options.output == Output::Json);
        assert_eq!(options.model.as_deref(), Some("qwen"));
        assert_eq!(options.prompt.as_deref(), Some("hello there"));
        assert_eq!(parse_args(args("-p -- -v means verbose")).unwrap().prompt.as_deref(), Some("-v means verbose"));
        assert_eq!(parse_args(args("--resume=abc")).unwrap().resume, Some(Some("abc".into())));
        assert_eq!(parse_args(args("--append-system-prompt be-kind -p hi")).unwrap().run.append_system_prompt.as_deref(), Some("be-kind"));
        // Mistakes are usage errors (stderr, exit 1); help and the version are not.
        assert!(matches!(parse_args(args("--bogus")), Err(Stop::Usage(_))));
        assert!(matches!(parse_args(args("-p hi --bogus")), Err(Stop::Usage(_))));
        assert!(matches!(parse_args(args("--max-turns 0")), Err(Stop::Usage(_))));
        assert!(matches!(parse_args(args("--help")), Err(Stop::Info(_))));
        assert!(matches!(parse_args(args("--version")), Err(Stop::Info(_))));
    }

    #[test]
    fn mcp_add_reads_claude_code_syntax() {
        let (server, project) = mcp_add_args(&args("--transport http docs https://example.com/mcp -H Authorization:Bearer-x")).unwrap();
        assert_eq!((server.name.as_str(), server.url.as_deref(), project), ("docs", Some("https://example.com/mcp"), false));
        assert_eq!(server.headers.get("Authorization").map(String::as_str), Some("Bearer-x"));
        let (server, project) = mcp_add_args(&args("-s project -e KEY=1 files -- npx -y @x/files --root .")).unwrap();
        assert_eq!((server.name.as_str(), server.command.as_str(), project), ("files", "npx", true));
        assert_eq!(server.args, vec!["-y", "@x/files", "--root", "."]);
        assert_eq!(server.env.get("KEY").map(String::as_str), Some("1"));
        // The command's own flags stay with the command.
        assert_eq!(mcp_add_args(&args("git uvx mcp-git --repo .")).unwrap().0.args, vec!["mcp-git", "--repo", "."]);
        assert!(mcp_add_args(&args("--transport sse old https://x/sse")).is_err());
        assert!(mcp_add_args(&args("--bogus x y")).is_err());
        assert!(mcp_add_args(&args("--scope team x y")).is_err());
    }
}
