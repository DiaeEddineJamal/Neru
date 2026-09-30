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

use std::{fs, io::{Read, Write}, path::Path, process::Command, time::Duration};

use serde_json::Value;

/// What a blocking-capable hook decided.
pub enum Verdict {
    Allow(String),
    Block(String),
}

fn config(root: &Path) -> Option<Value> {
    let text = fs::read_to_string(root.join(".neru").join("hooks.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn matches(matcher: &str, tool: &str) -> bool {
    matcher.is_empty() || matcher == "*" || matcher.split('|').any(|part| part.trim() == tool)
}

/// Runs the hooks for `event` whose matcher fits `tool` ("" when the event has no tool).
/// Collects their output; exit code 2 turns into a block.
pub fn fire(root: &Path, event: &str, tool: &str, input: &Value) -> Verdict {
    let Some(value) = config(root) else { return Verdict::Allow(String::new()) };
    let Some(entries) = value.get(event).and_then(Value::as_array) else { return Verdict::Allow(String::new()) };
    let payload = serde_json::json!({"event": event, "tool": tool, "input": input}).to_string();
    let mut notes = String::new();
    for entry in entries.iter().take(8) {
        let (matcher, command) = match entry {
            Value::String(command) => ("*", command.as_str()),
            Value::Object(_) => (entry["matcher"].as_str().unwrap_or("*"), entry["command"].as_str().unwrap_or("")),
            _ => continue,
        };
        if command.trim().is_empty() || command.len() > 2_000 || !matches(matcher, tool) {
            continue;
        }
        match run_with(root, command, Some(&payload)) {
            Ok(output) => {
                if !output.trim().is_empty() {
                    notes.push_str(&format!("
Hook ({event}): {}", output.trim()));
                }
            }
            Err((2, output)) => return Verdict::Block(if output.trim().is_empty() { format!("blocked by a {event} hook") } else { output.trim().to_string() }),
            Err((_, error)) => notes.push_str(&format!("
Hook ({event}) failed: {error}")),
        }
    }
    Verdict::Allow(notes)
}

pub fn after_edit(root: &Path, relative: &str) -> String {
    let Ok(text) = fs::read_to_string(root.join(".neru").join("hooks.json")) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return "\nHook file .neru/hooks.json is not valid JSON.".into();
    };
    let Some(commands) = value.get("postEdit").and_then(Value::as_array) else {
        return String::new();
    };
    let mut notes = String::new();
    for command in commands.iter().filter_map(Value::as_str).take(8) {
        let command = command.replace("{file}", relative);
        if command.trim().is_empty() || command.len() > 2_000 {
            continue;
        }
        match run_with(root, &command, None).map_err(|(_, error)| error) {
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

/// Runs one hook command. Errors carry the exit code (-1 when it did not run or timed out).
fn run_with(root: &Path, command: &str, input: Option<&str>) -> Result<String, (i32, String)> {
    #[cfg(windows)]
    let mut process = {
        let mut process = Command::new("powershell.exe");
        process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = Command::new("sh");
        process.args(["-lc", command]);
        process
    };
    if let Some(input) = input {
        process.env("NERU_HOOK_INPUT", input);
    }
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
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            return Err((-1, "hook exceeded 20 seconds".into()));
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
            Verdict::Allow(_) => panic!("hook should block"),
        }
        match fire(&root, "postToolUse", "read_file", &Value::Null) {
            Verdict::Allow(notes) => assert!(notes.contains("done")),
            Verdict::Block(_) => panic!("post hooks never block"),
        }
        let _ = fs::remove_dir_all(root);
    }
}
