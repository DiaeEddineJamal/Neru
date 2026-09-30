//! Project hooks, modeled on Claude Code's. `.neru/hooks.json` maps events to shell commands:
//!
//! ```json
//! { "postEdit": ["npx prettier --write {file}"],
//!   "preToolUse": [{ "matcher": "run_shell_command", "command": "./check.ps1" }],
//!   "postToolUse": [...], "userPromptSubmit": [...], "sessionStart": [...], "stop": [...] }
//! ```
//!
//! An entry is a command string or `{matcher, command}`; the matcher is a tool name, `*`, or
//! `a|b`. Each command gets the event as JSON in `NERU_HOOK_INPUT` and on stdin. A `preToolUse`
//! or `userPromptSubmit` hook that exits with code 2 blocks the action, and its output says why.
//! `{file}` in a postEdit command is replaced with the project-relative path.
//!
//! Personal hooks for every project go in `<data_dir>/hooks.json`, in the same format; they run
//! before the project's.
//!
//! Claude Code's `hooks` block in the project's `.claude/settings.json`,
//! `.claude/settings.local.json` and `.neru/settings.json` works too, once the project is trusted
//! (`crate::trust`). The personal `~/.claude/settings.json` hooks are not read: they are written
//! for Claude Code's own tools. The format:
//! `{"hooks": {"PreToolUse": [{"matcher": "Bash|Edit", "hooks": [{"type": "command", "command": "…", "timeout": 60}]}]}}`.
//! Matchers there are regular expressions over Claude's tool names (Bash, Edit, Write, Read, …),
//! which map onto Neru's tools, and stdin carries Claude's fields (`session_id`, `cwd`,
//! `hook_event_name`, `tool_name`, `tool_input`). Hook output that is JSON may decide:
//! `{"decision": "block", "reason": "…"}` blocks, `{"decision": "approve"}` skips the approval
//! prompt of a preToolUse call.
//!
//! Events: preToolUse, postToolUse, userPromptSubmit, sessionStart, stop, preCompact,
//! subagentStop, sessionEnd, notification (Claude: PreToolUse, PostToolUse, …).

use std::{fs, io::{Read, Write}, path::Path, process::Command, time::Duration};

use serde_json::{Value, json};

/// What a blocking-capable hook decided.
pub enum Verdict {
    Allow(String),
    Block(String),
    /// A preToolUse hook approved the call: it runs without the approval prompt.
    Approve,
}

/// The personal `<data_dir>/hooks.json`. Tests must not depend on whoever runs them.
fn personal_file() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    return None;
    #[cfg(not(test))]
    crate::workspace::data_dir().ok().map(|dir| dir.join("hooks.json"))
}

/// The Neru-format hook files: personal first, then the project's `.neru/hooks.json`.
fn hook_files(root: &Path) -> Vec<std::path::PathBuf> {
    personal_file().into_iter().chain(std::iter::once(root.join(".neru").join("hooks.json"))).collect()
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// The project's Claude-format settings files that have hooks, empty until the project is trusted.
fn trusted_settings(root: &Path) -> Vec<Value> {
    let settings: Vec<Value> = crate::policy::project_settings_files(root)
        .into_iter()
        .filter_map(|(_, path)| crate::policy::read_settings(&path))
        .filter(|value| value["hooks"].as_object().is_some_and(|hooks| !hooks.is_empty()))
        .collect();
    if settings.is_empty() || !crate::trust::is_trusted(root) {
        return Vec::new();
    }
    settings
}

/// Claude-style events (`PreToolUse`, …) that have command hooks in the project's settings files,
/// trusted or not: what trusting the project would turn on.
pub fn project_hook_events(root: &Path) -> Vec<String> {
    let mut events: Vec<String> = Vec::new();
    for (_, path) in crate::policy::project_settings_files(root) {
        let Some(settings) = crate::policy::read_settings(&path) else { continue };
        for (event, groups) in settings["hooks"].as_object().into_iter().flatten() {
            let has_command = groups.as_array().into_iter().flatten().any(|group| {
                group["hooks"].as_array().into_iter().flatten().any(|hook| hook["type"].as_str().unwrap_or("command") == "command" && hook["command"].as_str().is_some_and(|command| !command.trim().is_empty()))
            });
            if has_command && !events.contains(event) {
                events.push(event.clone());
            }
        }
    }
    events
}

/// Claude Code's names for a Neru tool, used by Claude-style matchers and in `tool_name`.
pub fn claude_names(tool: &str) -> &'static [&'static str] {
    match tool {
        "run_shell_command" | "run_project_task" => &["Bash"],
        "propose_edit" => &["Edit", "MultiEdit"],
        "propose_write_file" => &["Write"],
        "propose_delete" | "propose_move" | "propose_create_folder" => &["Edit", "Write"],
        "read_file" | "read_files" => &["Read"],
        "list_directory" => &["LS"],
        "search_text" => &["Grep"],
        "find_files" => &["Glob"],
        "fetch_url" => &["WebFetch"],
        "web_search" => &["WebSearch"],
        "task" => &["Task"],
        "update_todos" => &["TodoWrite"],
        _ => &[],
    }
}

fn matches(matcher: &str, tool: &str) -> bool {
    matcher.is_empty() || matcher == "*" || matcher.split('|').map(str::trim).any(|part| part == tool || claude_names(tool).contains(&part))
}

/// A Claude matcher is a regular expression over tool names; a plain name matches exactly.
fn claude_matches(matcher: &str, tool: &str) -> bool {
    let matcher = matcher.trim();
    if matcher.is_empty() || matcher == "*" || tool.is_empty() {
        return true;
    }
    let Ok(pattern) = regex::Regex::new(&format!("^(?:{matcher})$")) else { return matches(matcher, tool) };
    pattern.is_match(tool) || claude_names(tool).iter().any(|name| pattern.is_match(name))
}

/// `preToolUse` → `PreToolUse`.
fn claude_event(event: &str) -> String {
    let mut chars = event.chars();
    chars.next().map(|first| first.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
}

struct Hook {
    command: String,
    timeout: u64,
    /// From a Claude Code settings file: runs through bash where available.
    claude: bool,
}

/// Every hook for `event` that fits `tool`, from the personal and project `hooks.json` and the
/// project's settings files (once trusted).
fn hooks_for(root: &Path, event: &str, tool: &str) -> Vec<Hook> {
    let mut hooks = Vec::new();
    for config in hook_files(root).iter().filter_map(|path| read_json(path)) {
        let Some(entries) = config.get(event).and_then(Value::as_array) else { continue };
        for entry in entries {
            let (matcher, command) = match entry {
                Value::String(command) => ("*", command.as_str()),
                Value::Object(_) => (entry["matcher"].as_str().unwrap_or("*"), entry["command"].as_str().unwrap_or("")),
                _ => continue,
            };
            if matches(matcher, tool) {
                hooks.push(Hook { command: command.to_string(), timeout: 20, claude: false });
            }
        }
    }
    let name = claude_event(event);
    for settings in trusted_settings(root) {
        for group in settings["hooks"][&name].as_array().into_iter().flatten() {
            if !claude_matches(group["matcher"].as_str().unwrap_or(""), tool) {
                continue;
            }
            for hook in group["hooks"].as_array().into_iter().flatten() {
                if hook["type"].as_str().unwrap_or("command") != "command" {
                    continue;
                }
                let Some(command) = hook["command"].as_str() else { continue };
                hooks.push(Hook { command: command.to_string(), timeout: hook["timeout"].as_u64().unwrap_or(60).clamp(1, 600), claude: true });
            }
        }
    }
    hooks.retain(|hook| !hook.command.trim().is_empty() && hook.command.len() <= 2_000);
    hooks.truncate(16);
    hooks
}

/// True when some preToolUse hook could veto `tool` calls, so they must run one at a time.
pub fn has_pre_tool_hooks(root: &Path) -> bool {
    hook_files(root).iter().any(|path| path.is_file()) || !hooks_for(root, "preToolUse", "").is_empty()
}

/// `tool_input` the way Claude Code shapes it (absolute `file_path`, `pattern`, …), on top of
/// Neru's own arguments.
fn claude_input(root: &Path, tool: &str, input: &Value) -> Value {
    let mut shaped = input.clone();
    let Some(object) = shaped.as_object_mut() else { return shaped };
    let absolute = |path: &str| root.join(path).display().to_string().trim_start_matches(r"\\?\").to_string();
    if let Some(path) = input["path"].as_str() {
        object.insert("file_path".into(), json!(absolute(path)));
    }
    match tool {
        "propose_move" => {
            if let Some(path) = input["from"].as_str() {
                object.insert("file_path".into(), json!(absolute(path)));
            }
        }
        "search_text" => {
            object.insert("pattern".into(), input["query"].clone());
        }
        "run_project_task" => {
            if let Ok(command) = crate::tasks::command(root, input["task"].as_str().unwrap_or("")) {
                object.insert("command".into(), json!(command));
            }
        }
        _ => {}
    }
    shaped
}

/// Runs the hooks for `event` whose matcher fits `tool` ("" when the event has no tool).
/// Collects their output; exit code 2 turns into a block.
#[allow(dead_code)] // For callers without a session, such as the CLI.
pub fn fire(root: &Path, event: &str, tool: &str, input: &Value) -> Verdict {
    fire_for(root, "", event, tool, input)
}

/// [`fire`] for a session, whose id reaches the hooks as `session_id`.
pub fn fire_for(root: &Path, session: &str, event: &str, tool: &str, input: &Value) -> Verdict {
    let hooks = hooks_for(root, event, tool);
    if hooks.is_empty() {
        return Verdict::Allow(String::new());
    }
    let cwd = root.display().to_string().trim_start_matches(r"\\?\").to_string();
    let mut payload = json!({
        "session_id": session,
        "cwd": cwd,
        "hook_event_name": claude_event(event),
        "event": event,
        "tool": tool,
        "input": input,
    });
    // For other events `tool` is what their matchers see, like preCompact's "manual" or "auto".
    if !tool.is_empty() && matches!(event, "preToolUse" | "postToolUse") {
        payload["tool_name"] = json!(claude_names(tool).first().copied().unwrap_or(tool));
        payload["tool_input"] = claude_input(root, tool, input);
    } else if let Some(fields) = input.as_object() {
        // Tool-less events carry their fields at the top level, as in Claude Code (`prompt`, `message`, …).
        for (key, value) in fields {
            payload.as_object_mut().map(|object| object.entry(key.clone()).or_insert(value.clone()));
        }
    }
    let payload = payload.to_string();
    let mut notes = String::new();
    let mut approved = false;
    // Only these can stop what they announce; for the others a "block" is just reported.
    let can_block = matches!(event, "preToolUse" | "userPromptSubmit");
    let block = |notes: &mut String, reason: String| -> Option<Verdict> {
        let reason = if reason.trim().is_empty() { format!("blocked by a {event} hook") } else { reason.trim().to_string() };
        if can_block {
            return Some(Verdict::Block(reason));
        }
        notes.push_str(&format!("\nHook ({event}): {reason}"));
        None
    };
    for hook in &hooks {
        match run_hook(root, hook, &payload) {
            Ok(output) => match parse_output(&output) {
                Some(Output::Block(reason)) => {
                    if let Some(verdict) = block(&mut notes, reason) {
                        return verdict;
                    }
                }
                Some(Output::Approve(note)) => {
                    approved = true;
                    if !note.is_empty() {
                        notes.push_str(&format!("\nHook ({event}): {note}"));
                    }
                }
                Some(Output::Note(note)) => {
                    if !note.is_empty() {
                        notes.push_str(&format!("\nHook ({event}): {note}"));
                    }
                }
                None => {
                    if !output.trim().is_empty() {
                        notes.push_str(&format!("\nHook ({event}): {}", output.trim()));
                    }
                }
            },
            Err((2, output)) => {
                if let Some(verdict) = block(&mut notes, output) {
                    return verdict;
                }
            }
            Err((_, error)) => notes.push_str(&format!("\nHook ({event}) failed: {error}")),
        }
    }
    if approved && event == "preToolUse" { Verdict::Approve } else { Verdict::Allow(notes) }
}

enum Output {
    Block(String),
    Approve(String),
    Note(String),
}

/// Claude Code's JSON hook output: `decision` ("block" / "approve"), `continue: false`,
/// `hookSpecificOutput.permissionDecision` ("deny" / "allow") and `additionalContext`.
fn parse_output(output: &str) -> Option<Output> {
    let text = output.trim();
    if !text.starts_with('{') {
        return None;
    }
    // Stderr may follow the JSON on stdout; read the first value only.
    let value: Value = serde_json::Deserializer::from_str(text).into_iter::<Value>().next()?.ok()?;
    let specific = &value["hookSpecificOutput"];
    let reason = [&value["reason"], &specific["permissionDecisionReason"], &value["stopReason"]]
        .iter()
        .find_map(|item| item.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let note = [&specific["additionalContext"], &value["systemMessage"], &value["reason"]]
        .iter()
        .find_map(|item| item.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let decision = specific["permissionDecision"].as_str().or(value["decision"].as_str()).unwrap_or("");
    Some(match decision {
        "block" | "deny" => Output::Block(reason),
        "approve" | "allow" => Output::Approve(note),
        _ if value["continue"].as_bool() == Some(false) => Output::Block(reason),
        _ => Output::Note(note),
    })
}

/// Tells the project's `notification` hooks that a run stopped, e.g. to wait for approval. Runs
/// off the caller's thread; nothing waits for it.
pub fn notify(root: &Path, session: &str, message: &str) {
    let (root, session, message) = (root.to_path_buf(), session.to_string(), message.to_string());
    std::thread::spawn(move || {
        let _ = fire_for(&root, &session, "notification", "", &json!({"message": message, "notification_type": "permission_prompt"}));
    });
}

/// Runs the `sessionEnd` hooks for a session that is over (deleted, or the CLI exiting). Blocks
/// until they finish, so a CLI can call it right before it exits.
pub fn session_end(root: &Path, session: &str, reason: &str) {
    let _ = fire_for(root, session, "sessionEnd", "", &json!({"reason": reason}));
}

pub fn after_edit(root: &Path, relative: &str) -> String {
    let mut notes = String::new();
    let mut commands = Vec::new();
    for path in hook_files(root) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
            let name = if path.starts_with(root) { ".neru/hooks.json".to_string() } else { path.display().to_string() };
            notes.push_str(&format!("\nHook file {name} is not valid JSON."));
            continue;
        };
        commands.extend(value.get("postEdit").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from));
    }
    for command in commands.iter().take(8) {
        let command = command.replace("{file}", relative);
        if command.trim().is_empty() || command.len() > 2_000 {
            continue;
        }
        match run_with(root, &command, None, 20, false).map_err(|(_, error)| error) {
            Ok(output) => {
                if !output.trim().is_empty() {
                    notes.push_str("\nHook: ");
                    notes.push_str(output.trim());
                }
            }
            Err(error) => {
                notes.push_str("\nHook failed: ");
                notes.push_str(&error);
            }
        }
    }
    notes
}

/// Git Bash, which Claude Code runs hooks with on Windows.
#[cfg(windows)]
fn git_bash() -> Option<std::path::PathBuf> {
    let configured = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(std::path::PathBuf::from);
    let installed = [r"C:\Program Files\Git\bin\bash.exe", r"C:\Program Files (x86)\Git\bin\bash.exe"].map(std::path::PathBuf::from);
    configured.into_iter().chain(installed).find(|path| path.is_file())
}

fn run_hook(root: &Path, hook: &Hook, payload: &str) -> Result<String, (i32, String)> {
    run_with(root, &hook.command, Some(payload), hook.timeout, hook.claude)
}

fn run_with(root: &Path, command: &str, input: Option<&str>, timeout: u64, claude: bool) -> Result<String, (i32, String)> {
    #[cfg(windows)]
    let mut process = match git_bash().filter(|_| claude) {
        Some(bash) => {
            let mut process = Command::new(bash);
            process.args(["-c", command]);
            process
        }
        None => {
            let mut process = Command::new("powershell.exe");
            process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
            process
        }
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = Command::new("sh");
        process.args([if claude { "-c" } else { "-lc" }, command]);
        process
    };
    if let Some(input) = input {
        process.env("NERU_HOOK_INPUT", input);
    }
    process.env("CLAUDE_PROJECT_DIR", root.display().to_string().trim_start_matches(r"\\?\"));
    let mut child = process
        .current_dir(root)
        .stdin(if input.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| (-1, e.to_string()))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(input.as_bytes());
    }
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(timeout) {
            let _ = child.kill();
            return Err((-1, format!("hook exceeded {timeout} seconds")));
        }
        match child.try_wait().map_err(|e| (-1, e.to_string()))? {
            Some(status) => {
                let mut text = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut text);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut text);
                }
                if !status.success() {
                    let code = status.code().unwrap_or(-1);
                    let text: String = text.chars().take(2_000).collect();
                    return Err((code, if code == 2 { text } else { format!("exit {code}\n{text}") }));
                }
                return Ok(text.chars().take(2_000).collect());
            }
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_tool_hooks_match_and_block() {
        let root = std::env::temp_dir().join(format!("neru-hooks-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join(".neru")).unwrap();
        let block = if cfg!(windows) { "Write-Output 'no shell today'; exit 2" } else { "echo 'no shell today'; exit 2" };
        let config = serde_json::json!({"preToolUse": [{"matcher": "run_shell_command|propose_delete", "command": block}], "postToolUse": ["echo done"]});
        fs::write(root.join(".neru/hooks.json"), config.to_string()).unwrap();
        assert!(matches!(fire(&root, "preToolUse", "read_file", &Value::Null), Verdict::Allow(_)));
        match fire(&root, "preToolUse", "run_shell_command", &serde_json::json!({"command": "ls"})) {
            Verdict::Block(reason) => assert!(reason.contains("no shell today")),
            _ => panic!("hook should block"),
        }
        match fire(&root, "postToolUse", "read_file", &Value::Null) {
            Verdict::Allow(notes) => assert!(notes.contains("done")),
            _ => panic!("post hooks never block"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn claude_settings_hooks_map_tool_names_and_decide_with_json() {
        let root = std::env::temp_dir().join(format!("neru-claude-hooks-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join(".claude")).unwrap();
        let settings = json!({"hooks": {
            "PreToolUse": [
                {"matcher": "Bash|Edit", "hooks": [{"type": "command", "command": "echo '{\"decision\":\"block\",\"reason\":\"not here\"}'", "timeout": 30}]},
                {"matcher": "Web.*", "hooks": [{"type": "command", "command": "echo '{\"decision\":\"approve\"}'"}]}
            ],
            "PostToolUse": [{"matcher": "Write", "hooks": [{"type": "command", "command": "echo '{\"decision\":\"block\",\"reason\":\"formatted\"}'"}]}],
            "Notification": [{"hooks": [{"type": "prompt", "prompt": "ignored"}]}]
        }});
        fs::write(root.join(".claude/settings.local.json"), settings.to_string()).unwrap();
        // Nothing from the settings files runs until the project is trusted.
        assert!(matches!(fire(&root, "preToolUse", "propose_edit", &json!({"path": "src/a.rs"})), Verdict::Allow(_)));
        assert!(!has_pre_tool_hooks(&root));
        assert_eq!(project_hook_events(&root).len(), 2);
        assert!(project_hook_events(&root).iter().all(|event| event == "PreToolUse" || event == "PostToolUse"));
        crate::trust::trust_project(&root).unwrap();
        assert!(matches!(fire(&root, "preToolUse", "read_file", &json!({"path": "a"})), Verdict::Allow(_)));
        match fire(&root, "preToolUse", "propose_edit", &json!({"path": "src/a.rs"})) {
            Verdict::Block(reason) => assert_eq!(reason, "not here"),
            _ => panic!("Edit matcher should cover propose_edit"),
        }
        assert!(matches!(fire(&root, "preToolUse", "run_shell_command", &json!({"command": "ls"})), Verdict::Block(_)));
        assert!(matches!(fire(&root, "preToolUse", "fetch_url", &json!({"url": "https://a.b"})), Verdict::Approve));
        match fire(&root, "postToolUse", "propose_write_file", &json!({"path": "a"})) {
            Verdict::Allow(notes) => assert!(notes.contains("formatted")),
            _ => panic!("postToolUse only reports"),
        }
        assert!(hooks_for(&root, "notification", "").is_empty());
        assert!(has_pre_tool_hooks(&root));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn personal_claude_settings_hooks_are_not_read() {
        let root = std::env::temp_dir().join(format!("neru-personal-hooks-{}", uuid::Uuid::new_v4()));
        // Hooks only come from inside the project (and Neru's own hooks.json), never from
        // `~/.claude/settings.json`, even though its permission rules still apply.
        assert!(crate::policy::project_settings_files(&root).iter().all(|(_, path)| path.starts_with(&root)));
        if let Some(home) = dirs::home_dir() {
            let personal = home.join(".claude").join("settings.json");
            assert!(!crate::policy::project_settings_files(&root).iter().any(|(_, path)| *path == personal));
        }
        assert!(hook_files(&root).iter().all(|path| path.file_name().is_some_and(|name| name == "hooks.json")));
        assert!(hooks_for(&root, "preToolUse", "run_shell_command").is_empty());
    }

    #[test]
    fn matchers_and_payloads_follow_claude_code() {
        assert!(claude_matches("Bash", "run_shell_command"));
        assert!(claude_matches("Edit|Write", "propose_write_file"));
        assert!(claude_matches("mcp__github__.*", "mcp__github__create_issue"));
        assert!(!claude_matches("Write", "propose_edit"));
        assert!(!claude_matches("Read", "read_files_extra"));
        assert!(claude_matches("", "anything"));
        assert!(matches("Bash|read_file", "run_shell_command"));
        assert_eq!(claude_event("preToolUse"), "PreToolUse");
        assert_eq!(claude_event("sessionEnd"), "SessionEnd");
        let input = claude_input(Path::new("/work"), "search_text", &json!({"query": "todo", "path": "src/a.rs"}));
        assert_eq!(input["pattern"], "todo");
        assert!(input["file_path"].as_str().unwrap().ends_with("a.rs"));
        assert!(matches!(parse_output("{\"decision\":\"block\",\"reason\":\"x\"}\nwarning on stderr"), Some(Output::Block(reason)) if reason == "x"));
        assert!(matches!(parse_output("{\"hookSpecificOutput\":{\"permissionDecision\":\"allow\"}}"), Some(Output::Approve(_))));
        assert!(matches!(parse_output("{\"continue\":false,\"stopReason\":\"halt\"}"), Some(Output::Block(reason)) if reason == "halt"));
        assert!(parse_output("plain text").is_none());
    }
}
