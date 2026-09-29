//! Project hooks. `.neru/hooks.json` lists shell commands to run after a file is written.
//! `{file}` in a command is replaced with the project-relative path.

use std::{fs, io::Read, path::Path, process::Command, time::Duration};

use serde_json::Value;

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
        match run(root, &command) {
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

fn run(root: &Path, command: &str) -> Result<String, String> {
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
    let mut child = process
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            return Err("hook exceeded 20 seconds".into());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                let mut text = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut text);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut text);
                }
                if !status.success() {
                    return Err(format!("exit {}\n{}", status.code().unwrap_or(-1), text.chars().take(2_000).collect::<String>()));
                }
                return Ok(text.chars().take(2_000).collect());
            }
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    }
}
