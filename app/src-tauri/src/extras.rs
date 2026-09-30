//! Agent capabilities beyond reading and editing: persistent memory, a visible to-do list,
//! sub-agents that explore in parallel, and instructions from nested AGENTS.md / CLAUDE.md files.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use crate::workspace::data_dir;

pub const EXTRA_TOOLS: &str = r#"[
    {"type":"function","function":{"name":"update_todos","description":"Keep a visible to-do list for work with three or more steps. Send the whole list each time, with exactly one item in_progress while you work. Mark items completed as soon as they are done.","parameters":{"type":"object","properties":{"todos":{"type":"array","items":{"type":"object","properties":{"content":{"type":"string"},"status":{"type":"string","enum":["pending","in_progress","completed"]}},"required":["content","status"]}}},"required":["todos"]}}},
    {"type":"function","function":{"name":"task","description":"Hand a self-contained question to a read-only sub-agent that uses the project index and returns a short report (answer, key files with line numbers, open questions). agent_type explore finds and explains code (default, fastest); plan designs a change; general also uses the web. Call several in one turn to cover independent areas in parallel (up to 8). It cannot see this conversation, so put everything it needs in the prompt. Not for lookups you can finish in one or two calls.","parameters":{"type":"object","properties":{"description":{"type":"string","description":"3-6 word label"},"prompt":{"type":"string"},"agent_type":{"type":"string","enum":["explore","plan","general"]}},"required":["description","prompt"]}}},
    {"type":"function","function":{"name":"save_memory","description":"Remember a durable fact for future sessions: a user preference, a project convention, or a decision. scope project is for this repository, user is for all projects. Not for things already in the code or for this conversation only.","parameters":{"type":"object","properties":{"fact":{"type":"string"},"scope":{"type":"string","enum":["project","user"]}},"required":["fact","scope"]}}}
]"#;

/// Offered in Plan mode only: hands the finished plan to the user for approval.
pub const PLAN_TOOL: &str = r#"[
    {"type":"function","function":{"name":"exit_plan_mode","description":"Call when your plan is ready. Pass the whole plan in markdown: the goal, the files to change and how, and how you will verify it. The user approves it (and Neru switches to editing) or asks for changes. Only call it after you have researched enough to be concrete.","parameters":{"type":"object","properties":{"plan":{"type":"string","description":"The plan, in markdown"}},"required":["plan"]}}}
]"#;

/// Offered in every mode: asks the user multiple-choice questions and waits for the answers.
pub const QUESTION_TOOL: &str = r#"[
    {"type":"function","function":{"name":"ask_user_question","description":"Ask the user 1-4 multiple-choice questions when a decision only they can make blocks you (requirements, preferences, trade-offs between approaches). Each question has 2-4 options; the user can also type their own answer, so do not add an Other option. Not for things you can find out from the code or the web.","parameters":{"type":"object","properties":{"questions":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"object","properties":{"question":{"type":"string","description":"The full question, ending with a question mark"},"header":{"type":"string","description":"A short label of at most 12 characters, like Auth method"},"options":{"type":"array","minItems":2,"maxItems":4,"items":{"type":"object","properties":{"label":{"type":"string","description":"1-5 words"},"description":{"type":"string","description":"What this choice means or implies"}},"required":["label","description"]}},"multiSelect":{"type":"boolean","description":"true when several options can be chosen together"}},"required":["question","header","options","multiSelect"]}}},"required":["questions"]}}}
]"#;

// ---------- plans and questions ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub header: String,
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

pub fn parse_questions(args: &Value) -> Result<Vec<Question>, String> {
    let items = args["questions"].as_array().ok_or("questions must be a list of 1-4 questions")?;
    if items.is_empty() || items.len() > 4 {
        return Err("ask 1-4 questions at a time".into());
    }
    items
        .iter()
        .map(|item| {
            let question = item["question"].as_str().unwrap_or("").trim().to_string();
            if question.is_empty() {
                return Err("every question needs question text".to_string());
            }
            let options: Vec<QuestionOption> = item["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    let label = option["label"].as_str().or_else(|| option.as_str())?.trim().to_string();
                    let description = option["description"].as_str().unwrap_or("").trim().to_string();
                    (!label.is_empty()).then_some(QuestionOption { label, description })
                })
                .collect();
            if !(2..=4).contains(&options.len()) {
                return Err(format!("“{question}” needs 2-4 options with a label each"));
            }
            let header: String = item["header"].as_str().unwrap_or("").trim().chars().take(12).collect();
            let multi_select = item["multiSelect"].as_bool().or_else(|| item["multi_select"].as_bool()).unwrap_or(false);
            Ok(Question { question, header, options, multi_select })
        })
        .collect()
}

/// The tool result for exit_plan_mode once the user has decided.
pub fn plan_result(approve: bool, mode: &str, feedback: &str) -> String {
    let feedback = feedback.trim();
    if approve {
        let how = match mode {
            "accept_edits" | "auto" | "bypass" => "File edits now apply without asking.",
            _ => "Each edit and command now waits for the user's approval.",
        };
        let note = if feedback.is_empty() { String::new() } else { format!(" They added: {feedback}") };
        format!("The user approved the plan. Proceed with it now: make the changes, then verify them. {how}{note}")
    } else if feedback.is_empty() {
        "The user wants to keep planning. You are still in Plan mode: ask what to change, or refine the plan and call exit_plan_mode again.".into()
    } else {
        format!("The user wants changes to the plan: {feedback}\nYou are still in Plan mode. Revise the plan and call exit_plan_mode again.")
    }
}

/// The tool result for ask_user_question: one answer per question, in order.
pub fn answers_result(questions: &[Question], answers: &[String]) -> String {
    let pairs: Vec<String> = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let answer = answers.get(index).map(|answer| answer.trim()).filter(|answer| !answer.is_empty()).unwrap_or("(no answer)");
            format!("\"{}\" = \"{answer}\"", question.question)
        })
        .collect();
    format!("User answered: {}. Continue with these answers in mind.", pairs.join("; "))
}

fn session_for(state: &crate::AppState, session_id: Option<&str>) -> Result<crate::sessions::Shared, String> {
    match session_id {
        Some(id) => crate::sessions::runtime(state, id),
        None => crate::sessions::active(state),
    }
}

/// Answers a pending exit_plan_mode call. Returns the mode the session continues in.
#[tauri::command]
pub fn resolve_plan(session_id: Option<String>, approve: bool, mode: Option<String>, feedback: Option<String>, state: tauri::State<'_, crate::AppState>) -> Result<String, String> {
    let shared = session_for(&state, session_id.as_deref())?;
    let mut runtime = crate::sessions::lock(&shared)?;
    let Some(crate::PendingAction::Plan { tool_call_id, .. }) = runtime.pending.clone() else {
        return Err("No plan is waiting for review".into());
    };
    let mode = match (approve, mode.as_deref()) {
        (false, _) => "plan",
        (true, Some(mode @ ("accept_edits" | "auto" | "bypass"))) => mode,
        (true, _) => "manual",
    };
    runtime.pending = None;
    if let Some(id) = tool_call_id {
        runtime.conversation.push(serde_json::json!({"role":"tool","tool_call_id":id,"content":plan_result(approve, mode, feedback.as_deref().unwrap_or(""))}));
    }
    runtime.save()?;
    Ok(mode.into())
}

/// Answers a pending ask_user_question call, one answer per question (several choices joined by ", ").
#[tauri::command]
pub fn answer_question(session_id: Option<String>, answers: Vec<String>, state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let shared = session_for(&state, session_id.as_deref())?;
    let mut runtime = crate::sessions::lock(&shared)?;
    let Some(crate::PendingAction::Question { questions, tool_call_id }) = runtime.pending.clone() else {
        return Err("No question is waiting for an answer".into());
    };
    runtime.pending = None;
    if let Some(id) = tool_call_id {
        runtime.conversation.push(serde_json::json!({"role":"tool","tool_call_id":id,"content":answers_result(&questions, &answers)}));
    }
    runtime.save()
}

// ---------- memory ----------

pub fn project_memory_path(root: &Path) -> PathBuf {
    root.join(".neru").join("memory.md")
}

pub fn user_memory_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("memory.md"))
}

/// Both memory files, for the system prompt. Empty when there is nothing remembered.
pub fn memory_prompt(root: Option<&Path>) -> String {
    let mut text = String::new();
    let mut add = |label: &str, path: Option<PathBuf>| {
        if let Some(body) = path.and_then(|path| fs::read_to_string(path).ok()).map(|body| body.trim().to_string()).filter(|body| !body.is_empty()) {
            text.push_str(&format!("\n\n{label}:\n{}", body.chars().take(8_000).collect::<String>()));
        }
    };
    add("Memory about the user (saved in earlier sessions)", user_memory_path().ok());
    if let Some(root) = root {
        add("Memory about this project (saved in earlier sessions)", Some(project_memory_path(root)));
    }
    text
}

pub fn save_memory(root: Option<&Path>, fact: &str, scope: &str) -> Result<String, String> {
    let fact = fact.trim().replace('\n', " ");
    if fact.is_empty() || fact.len() > 1_000 {
        return Err("fact must be 1–1000 characters".into());
    }
    let path = match (scope, root) {
        ("project", Some(root)) => project_memory_path(root),
        _ => user_memory_path()?,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|line| line.trim_start_matches("- ").trim() == fact) {
        return Ok("Already remembered.".into());
    }
    let mut body = if existing.trim().is_empty() { "# Neru memory\n\n".to_string() } else { existing };
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&format!("- {fact}\n"));
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(format!("Remembered ({scope}): {fact}"))
}

// ---------- to-dos ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Todo {
    pub content: String,
    pub status: String,
}

pub fn parse_todos(args: &Value) -> Result<Vec<Todo>, String> {
    let items = args["todos"].as_array().ok_or("todos must be a list")?;
    let todos: Vec<Todo> = items
        .iter()
        .filter_map(|item| {
            let content = item["content"].as_str()?.trim().to_string();
            let status = match item["status"].as_str().unwrap_or("pending") {
                "in_progress" => "in_progress",
                "completed" | "done" => "completed",
                _ => "pending",
            };
            (!content.is_empty()).then(|| Todo { content, status: status.into() })
        })
        .take(50)
        .collect();
    if todos.is_empty() && !items.is_empty() {
        return Err("every item needs content and status".into());
    }
    Ok(todos)
}

pub fn todo_summary(todos: &[Todo]) -> String {
    let done = todos.iter().filter(|todo| todo.status == "completed").count();
    let current = todos.iter().find(|todo| todo.status == "in_progress").map(|todo| format!(" Now: {}.", todo.content)).unwrap_or_default();
    format!("To-do list updated: {done}/{} done.{current}", todos.len())
}

// ---------- nested instructions ----------

/// AGENTS.md / CLAUDE.md files in the folders between the project root and `path` (the root's own
/// files are already in the system prompt). Each is returned once per run.
pub fn nested_instructions(root: &Path, relative: &str, loaded: &mut HashSet<PathBuf>) -> String {
    let mut text = String::new();
    let mut folder = PathBuf::new();
    let parts: Vec<&str> = relative.split(['/', '\\']).filter(|part| !part.is_empty() && *part != ".").collect();
    let target = root.join(relative);
    let folders = if target.is_dir() { parts.len() } else { parts.len().saturating_sub(1) };
    for part in parts.iter().take(folders) {
        folder.push(part);
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = root.join(&folder).join(name);
            if path.is_file() && loaded.insert(path.clone()) {
                if let Ok(body) = fs::read_to_string(&path) {
                    let body = expand_file(&body, &path);
                    text.push_str(&format!(
                        "\n\n[Instructions from {}/{name}, which apply to files in that folder]\n{}",
                        folder.display().to_string().replace('\\', "/"),
                        body.chars().take(8_000).collect::<String>()
                    ));
                }
            }
        }
    }
    text
}

// ---------- instruction files ----------

/// How deep `@path` imports nest, as in Claude Code.
pub const IMPORT_DEPTH: usize = 5;
const FILE_CHARS: usize = 20_000;
const INSTRUCTION_CHARS: usize = 60_000;

/// Replaces each line that is exactly `@path` (relative to `base_dir`, or `~/…` for the home
/// folder) with that file's contents, recursively up to `depth` levels. Missing files leave the
/// line as it is; a file is included once, which also breaks import cycles. Fenced code is left
/// alone.
#[allow(dead_code)] // Instruction files go through `expand_file`; this is for other text.
pub fn expand_imports(text: &str, base_dir: &Path, depth: usize) -> String {
    expand_with(text, base_dir, depth, &mut HashSet::new())
}

/// `expand_imports` for the text of `file`, which is never pulled back into itself.
fn expand_file(text: &str, file: &Path) -> String {
    let folder = file.parent().unwrap_or(Path::new("."));
    let mut seen = HashSet::from([file.canonicalize().unwrap_or_else(|_| file.to_path_buf())]);
    expand_with(text.trim_start_matches('\u{feff}'), folder, IMPORT_DEPTH, &mut seen)
}

fn import_target(line: &str, base_dir: &Path) -> Option<PathBuf> {
    let spec = line.trim().strip_prefix('@')?;
    if spec.is_empty() || spec.chars().any(char::is_whitespace) {
        return None;
    }
    if let Some(rest) = spec.strip_prefix("~/").or_else(|| spec.strip_prefix("~\\")) {
        return dirs::home_dir().map(|home| home.join(rest));
    }
    let path = Path::new(spec);
    Some(if path.is_absolute() { path.to_path_buf() } else { base_dir.join(path) })
}

fn expand_with(text: &str, base_dir: &Path, depth: usize, seen: &mut HashSet<PathBuf>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let target = if fenced || depth == 0 { None } else { import_target(line, base_dir) };
        let found = target.and_then(|path| path.canonicalize().ok()).filter(|path| path.is_file());
        match found {
            Some(path) if seen.insert(path.clone()) => {
                if let Ok(body) = crate::workspace::read_limited(&path) {
                    let folder = path.parent().map(Path::to_path_buf).unwrap_or_else(|| base_dir.to_path_buf());
                    let body: String = body.trim_start_matches('\u{feff}').chars().take(FILE_CHARS).collect();
                    out.push_str(expand_with(&body, &folder, depth - 1, seen).trim_end());
                    out.push('\n');
                }
            }
            // Already included higher up (or a cycle): drop the line rather than repeat the file.
            Some(_) => {}
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// Instruction files for a session in `root`, most general first so that the project's own
/// files come last: the user's (`~/.claude/CLAUDE.md`, Neru's data-folder `AGENTS.md`), parent
/// folders' `AGENTS.md` / `CLAUDE.md` below the home folder or drive root, then the project's.
pub fn instruction_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let home = dirs::home_dir();
    if let Some(home) = &home {
        files.push(("User instructions from ~/.claude/CLAUDE.md (apply to every project)".into(), home.join(".claude").join("CLAUDE.md")));
    }
    if let Ok(dir) = data_dir() {
        files.push(("User instructions from Neru's AGENTS.md (apply to every project)".into(), dir.join("AGENTS.md")));
    }
    let home = home.and_then(|home| home.canonicalize().ok());
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut parents: Vec<&Path> = canonical
        .ancestors()
        .skip(1)
        .take_while(|folder| folder.parent().is_some() && home.as_deref() != Some(*folder))
        .collect();
    parents.reverse();
    for folder in parents {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let shown = folder.join(name).display().to_string().trim_start_matches(r"\\?\").to_string();
            files.push((format!("Instructions from the parent folder file {shown} (apply to this project)"), folder.join(name)));
        }
    }
    for name in ["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md", ".neru/instructions.md"] {
        files.push((format!("Project instructions from {name}"), root.join(name)));
    }
    let mut seen = HashSet::new();
    files.retain(|(_, path)| path.is_file() && seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())));
    files
}

/// Every instruction file for `root`, with `@path` imports expanded, as system-prompt text.
pub fn instructions_prompt(root: &Path) -> String {
    let mut text = String::new();
    for (label, path) in instruction_files(root) {
        let Ok(body) = crate::workspace::read_limited(&path) else { continue };
        let body = expand_file(&body, &path);
        let remaining = INSTRUCTION_CHARS.saturating_sub(text.chars().count());
        if remaining < 200 {
            break;
        }
        text.push_str(&format!("\n\n{label}:\n{}", body.trim().chars().take(FILE_CHARS.min(remaining)).collect::<String>()));
    }
    text
}

// ---------- /doctor ----------

#[derive(Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    /// What to do about it, in plain words, when it is not ok.
    pub fix: String,
}

async fn version(program: &str, args: &[&str]) -> Option<String> {
    #[cfg(windows)]
    let mut command = {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(program).args(args);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = tokio::process::Command::new(program);
        command.args(args);
        command
    };
    let output = tokio::time::timeout(std::time::Duration::from_secs(8), command.kill_on_drop(true).output()).await.ok()?.ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).lines().next().unwrap_or("").trim().to_string())
}

/// Checks what Neru needs: a model, tools on PATH, and the open project.
#[tauri::command]
pub async fn doctor(app: AppHandle) -> Result<Vec<Check>, String> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let config = state.provider.lock().map_err(|e| e.to_string())?.clone();
    let mut checks = Vec::new();
    let keyed = !config.api_key.is_empty() || matches!(config.provider_id.as_str(), "local" | "ollama");
    checks.push(Check {
        name: "Model".into(),
        ok: keyed && !config.model.is_empty(),
        detail: if config.model.is_empty() { "No model chosen".into() } else { format!("{} on {}", config.model, config.provider_id) },
        fix: "Open Settings → Model, add an API key and pick a model.".into(),
    });
    if keyed && !config.model.is_empty() {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(12)).build().map_err(|e| e.to_string())?;
        let reached = crate::providers::models_request(&client, &config).send().await;
        let (ok, detail) = match reached {
            Ok(response) if response.status().is_success() => (true, "The provider answered".to_string()),
            Ok(response) if response.status().as_u16() == 401 || response.status().as_u16() == 403 => (false, "The provider rejected the API key".to_string()),
            Ok(response) => (false, format!("The provider answered with HTTP {}", response.status().as_u16())),
            Err(error) => (false, format!("Could not reach the provider: {}", error.to_string().chars().take(120).collect::<String>())),
        };
        checks.push(Check { name: "Provider connection".into(), ok, detail, fix: "Check the key in Settings → Model, your internet connection, or try another provider.".into() });
    }
    for (name, program, fix) in [
        ("Git", "git", "Install Git from git-scm.com to use branches, diffs and checkpoints."),
        ("Node.js", "node", "Install Node.js LTS from nodejs.org to run JavaScript projects and previews."),
        ("npm", "npm", "npm comes with Node.js; reinstall Node.js if it is missing."),
    ] {
        let found = version(program, &["--version"]).await;
        checks.push(Check { name: name.into(), ok: found.is_some(), detail: found.unwrap_or_else(|| "Not found on PATH".into()), fix: fix.into() });
    }
    let root = crate::workspace::project_root(&state).ok();
    checks.push(Check {
        name: "Project".into(),
        ok: root.is_some(),
        detail: root.as_ref().map_or("No project open".into(), |root| root.display().to_string()),
        fix: "Open a folder from the sidebar to work on code.".into(),
    });
    if let Some(root) = root {
        let hooks = root.join(".neru/hooks.json");
        if hooks.is_file() {
            let valid = fs::read_to_string(&hooks).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()).is_some();
            checks.push(Check { name: "Hooks".into(), ok: valid, detail: if valid { ".neru/hooks.json is valid".into() } else { ".neru/hooks.json is not valid JSON".into() }, fix: "Fix the JSON in .neru/hooks.json (run /hooks to open it).".into() });
        }
    }
    Ok(checks)
}

/// Opens a memory file in the system's default editor (the user file lives outside the project).
#[tauri::command]
pub fn open_memory_file(scope: String, state: tauri::State<'_, crate::AppState>) -> Result<String, String> {
    let path = if scope == "project" { project_memory_path(&crate::workspace::project_root(&state)?) } else { user_memory_path()? };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if !path.exists() {
        fs::write(&path, "# Neru memory

").map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    let status = std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", &path.display().to_string()]).spawn();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(&path).spawn();
    status.map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn todos_normalize_statuses_and_summarize() {
        let todos = parse_todos(&json!({"todos":[{"content":"Scaffold","status":"done"},{"content":"Style","status":"in_progress"},{"content":"Test","status":"weird"}]})).unwrap();
        assert_eq!(todos[0].status, "completed");
        assert_eq!(todos[2].status, "pending");
        assert_eq!(todo_summary(&todos), "To-do list updated: 1/3 done. Now: Style.");
    }

    #[test]
    fn questions_parse_and_answers_read_back() {
        let args = json!({"questions":[
            {"question":"Which database?","header":"Database engine choice","options":[{"label":"SQLite","description":"One file"},{"label":"Postgres","description":"A server"}],"multiSelect":false},
            {"question":"Which extras?","header":"Extras","options":["Auth","Search","Tests"],"multi_select":true}
        ]});
        let questions = parse_questions(&args).unwrap();
        assert_eq!(questions[0].header, "Database eng");
        assert!(questions[1].multi_select && questions[1].options[2].label == "Tests");
        assert_eq!(
            answers_result(&questions, &["SQLite".into(), "Auth, Tests".into()]),
            "User answered: \"Which database?\" = \"SQLite\"; \"Which extras?\" = \"Auth, Tests\". Continue with these answers in mind."
        );
        assert!(answers_result(&questions, &["Postgres".into()]).contains("\"Which extras?\" = \"(no answer)\""));
        assert!(parse_questions(&json!({"questions":[{"question":"Only one?","options":[{"label":"Yes"}]}]})).is_err());
        assert!(parse_questions(&json!({"questions":[]})).is_err());
        let pending = crate::PendingAction::Question { questions: questions.clone(), tool_call_id: Some("call-1".into()) };
        let restored: crate::PendingAction = serde_json::from_str(&serde_json::to_string(&pending).unwrap()).unwrap();
        assert!(matches!(restored, crate::PendingAction::Question { questions: saved, .. } if saved == questions));
    }

    #[test]
    fn plan_results_tell_the_model_what_to_do() {
        let approved = plan_result(true, "accept_edits", "");
        assert!(approved.starts_with("The user approved the plan. Proceed") && approved.contains("without asking"));
        assert!(plan_result(true, "manual", "").contains("waits for the user's approval"));
        let changes = plan_result(false, "plan", "Use SQLite instead");
        assert!(changes.starts_with("The user wants changes to the plan: Use SQLite instead") && changes.contains("exit_plan_mode again"));
        assert!(plan_result(false, "plan", "  ").starts_with("The user wants to keep planning"));
    }

    #[test]
    fn nested_instructions_load_once_per_folder() {
        let root = std::env::temp_dir().join(format!("neru-nested-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("web/src")).unwrap();
        fs::write(root.join("AGENTS.md"), "root rules").unwrap();
        fs::write(root.join("web/AGENTS.md"), "web rules").unwrap();
        fs::write(root.join("web/src/app.ts"), "x").unwrap();
        let mut loaded = HashSet::new();
        let first = nested_instructions(&root, "web/src/app.ts", &mut loaded);
        assert!(first.contains("web rules") && !first.contains("root rules"));
        assert!(nested_instructions(&root, "web/src/app.ts", &mut loaded).is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_expand_relative_files_once_and_stop_at_cycles() {
        let root = std::env::temp_dir().join(format!("neru-imports-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/style.md"), "Use tabs.\n@../AGENTS.md\n@missing.md").unwrap();
        fs::write(root.join("AGENTS.md"), "Top rules\n@docs/style.md\n@docs/style.md\n```\n@docs/style.md\n```\nwrite @docs/style.md inline").unwrap();
        let text = expand_file(&fs::read_to_string(root.join("AGENTS.md")).unwrap(), &root.join("AGENTS.md"));
        assert_eq!(text.matches("Use tabs.").count(), 1);
        assert!(text.contains("@missing.md"), "missing imports stay as written");
        assert!(text.contains("```\n@docs/style.md\n```"), "fenced code is not expanded");
        assert!(text.contains("write @docs/style.md inline"));
        assert_eq!(text.matches("Top rules").count(), 1);
        assert!(!expand_imports("@docs/style.md", &root, 0).contains("Use tabs."));
        fs::write(root.join("CLAUDE.local.md"), "local only").unwrap();
        let prompt = instructions_prompt(&root);
        assert!(prompt.contains("Project instructions from AGENTS.md:\nTop rules"));
        assert!(prompt.contains("Project instructions from CLAUDE.local.md:\nlocal only"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn memory_is_saved_once_and_loaded() {
        let root = std::env::temp_dir().join(format!("neru-memory-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        save_memory(Some(&root), "Use pnpm, not npm", "project").unwrap();
        assert_eq!(save_memory(Some(&root), "Use pnpm, not npm", "project").unwrap(), "Already remembered.");
        let body = fs::read_to_string(project_memory_path(&root)).unwrap();
        assert_eq!(body.matches("Use pnpm").count(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
