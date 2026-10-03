//! Output styles and the custom status line, the way Claude Code sets them in its settings files.
//!
//! - `"outputStyle": "explanatory"` picks how the agent writes: the built-in `default`,
//!   `explanatory` and `learning`, or a Markdown file in `.neru/output-styles/`,
//!   `.claude/output-styles/`, `~/.claude/output-styles/` or Neru's own `output-styles` folder
//!   (front matter `name` and `description`, the body is the instruction).
//! - `"statusLine": {"type": "command", "command": "~/bin/status.sh"}` replaces the right side of the
//!   CLI's status line with the first line the command prints. It gets a JSON description of the
//!   session on stdin, as in Claude Code.
//!
//! `/output-style` and `/statusline` write the project's `.claude/settings.local.json`, where Claude
//! Code keeps the same keys. A project's status-line command runs only once the folder is trusted.

#[allow(unused_imports)] // .hidden() is called here only on some systems
use crate::Hidden;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::policy::{read_settings, settings_files};

/// A way of writing replies.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub name: String,
    pub description: String,
    /// What is added to the system prompt; empty for `default`.
    pub prompt: String,
    /// "built-in", "project" or "personal".
    pub source: String,
}

const BUILT_IN: &[(&str, &str, &str)] = &[
    ("default", "Neru's usual concise replies", ""),
    (
        "explanatory",
        "Explains its choices and the codebase's patterns as it works",
        "Output style: Explanatory. While you work, add short \"Insight\" notes (2 or 3 points) that explain why you chose an approach, the patterns and trade-offs in this codebase, and what a reader should notice. Keep the code changes themselves as focused as usual.",
    ),
    (
        "learning",
        "Teaches as it goes and leaves small parts for you to write",
        "Output style: Learning. Act as a patient teacher. Explain each step briefly as you go. For small, self-contained pieces of logic (5 to 10 lines) that are good practice, do not write them yourself: leave a clearly marked TODO(human) in the code, explain what it should do, and ask the user to write it. Write the rest of the change yourself.",
    ),
];

/// Folders that hold style files, most specific first.
fn style_folders(root: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut folders = vec![(root.join(".neru").join("output-styles"), "project"), (root.join(".claude").join("output-styles"), "project")];
    if let Some(home) = dirs::home_dir() {
        folders.push((home.join(".claude").join("output-styles"), "personal"));
    }
    if let Ok(data) = crate::workspace::data_dir() {
        folders.push((data.join("output-styles"), "personal"));
    }
    folders
}

/// Every style there is to choose from: the built-in ones, then style files.
pub fn list(root: &Path) -> Vec<Style> {
    let mut styles: Vec<Style> = BUILT_IN.iter().map(|(name, description, prompt)| Style { name: name.to_string(), description: description.to_string(), prompt: prompt.to_string(), source: "built-in".into() }).collect();
    for (folder, source) in style_folders(root) {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "md")).collect();
        paths.sort();
        for path in paths {
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
            let (name, description, body) = parse_style(&text, &stem);
            if name.is_empty() || styles.iter().any(|style| style.name.eq_ignore_ascii_case(&name)) {
                continue;
            }
            styles.push(Style { name, description, prompt: body.chars().take(20_000).collect(), source: source.into() });
        }
    }
    styles
}

/// A style file's name, description and instruction.
fn parse_style(text: &str, stem: &str) -> (String, String, String) {
    let text = text.trim_start_matches('\u{feff}');
    let (front, body) = match text.strip_prefix("---").and_then(|rest| rest.split_once("\n---")) {
        Some((front, body)) => (front, body.trim_start_matches(['-', '\r', '\n']).trim()),
        None => ("", text.trim()),
    };
    let field = |key: &str| {
        front.lines().find_map(|line| line.trim().strip_prefix(key).and_then(|rest| rest.trim_start().strip_prefix(':')).map(|value| value.trim().trim_matches(['"', '\'']).to_string()))
    };
    (field("name").filter(|name| !name.is_empty()).unwrap_or_else(|| stem.to_string()), field("description").unwrap_or_default(), body.to_string())
}

/// A settings key from every settings file, the last one that sets it winning. With `trusted_only`,
/// a project's files count only once the folder is trusted.
fn setting(root: &Path, key: &str, trusted_only: bool) -> Option<Value> {
    let mut trusted = None;
    let mut found = None;
    for (label, path) in settings_files(root) {
        let Some(value) = read_settings(&path).map(|settings| settings[key].clone()).filter(|value| !value.is_null()) else { continue };
        if trusted_only && !label.starts_with('~') && !*trusted.get_or_insert_with(|| crate::trust::is_trusted(root)) {
            continue;
        }
        found = Some(value);
    }
    found
}

fn local_settings(root: &Path) -> PathBuf {
    root.join(".claude").join("settings.local.json")
}

/// Sets (or with `None` removes) one key in the project's `.claude/settings.local.json`.
fn write_local(root: &Path, key: &str, value: Option<Value>) -> Result<(), String> {
    let path = local_settings(root);
    let mut settings = read_settings(&path).filter(Value::is_object).unwrap_or_else(|| json!({}));
    match value {
        Some(value) => settings[key] = value,
        None => {
            settings.as_object_mut().map(|object| object.remove(key));
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// The style in use, `default` when none is set or the one set no longer exists.
pub fn current(root: &Path) -> Style {
    let styles = list(root);
    let wanted = setting(root, "outputStyle", false).and_then(|value| value.as_str().map(str::to_string)).unwrap_or_default();
    styles.iter().find(|style| style.name.eq_ignore_ascii_case(&wanted)).cloned().unwrap_or_else(|| styles[0].clone())
}

/// Chooses a style for this project.
pub fn choose(root: &Path, name: &str) -> Result<Style, String> {
    let style = list(root).into_iter().find(|style| style.name.eq_ignore_ascii_case(name.trim())).ok_or_else(|| format!("No output style named {name}"))?;
    write_local(root, "outputStyle", Some(json!(style.name)))?;
    Ok(style)
}

/// What the style in use adds to the system prompt.
pub fn prompt(root: &Path) -> String {
    let style = current(root);
    if style.prompt.is_empty() { String::new() } else { format!("\n\n{}", style.prompt) }
}

/// The status-line command, when one is set and may run here.
pub fn status_command(root: &Path) -> Option<String> {
    let value = setting(root, "statusLine", true)?;
    let command = value["command"].as_str().or_else(|| value.as_str())?.trim().to_string();
    (!command.is_empty() && value["type"].as_str().is_none_or(|kind| kind == "command")).then_some(command)
}

/// Sets the project's status-line command, or removes it with `None`.
pub fn set_status_command(root: &Path, command: Option<&str>) -> Result<(), String> {
    write_local(root, "statusLine", command.map(|command| json!({"type": "command", "command": command})))
}

/// The first line the status-line command prints for `input`, within two seconds.
pub fn run_status_line(root: &Path, command: &str, input: &Value) -> Option<String> {
    let mut child = shell(command).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.to_string().as_bytes());
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    text.lines().next().map(|line| line.chars().take(200).collect())
}

fn shell(command: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut process = std::process::Command::new("powershell.exe");
        process.args(["-NoProfile", "-NonInteractive", "-Command", command]).creation_flags(0x0800_0000);
        process
    }
    #[cfg(not(windows))]
    {
        let mut process = std::process::Command::new("sh").hidden();
        process.args(["-c", command]);
        process
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_come_built_in_and_from_files() {
        let root = std::env::temp_dir().join(format!("neru-styles-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join(".claude").join("output-styles")).unwrap();
        fs::write(root.join(".claude").join("output-styles").join("terse.md"), "---\nname: Terse\ndescription: Few words\n---\nAnswer in one line.").unwrap();
        let styles = list(&root);
        assert_eq!(styles[0].name, "default");
        let terse = styles.iter().find(|style| style.name == "Terse").unwrap();
        assert_eq!((terse.description.as_str(), terse.prompt.as_str(), terse.source.as_str()), ("Few words", "Answer in one line.", "project"));
        assert_eq!(current(&root).name, "default");
        assert!(prompt(&root).is_empty());
        choose(&root, "terse").unwrap();
        assert_eq!(current(&root).name, "Terse");
        assert!(prompt(&root).ends_with("Answer in one line."));
        assert!(choose(&root, "nope").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_projects_status_line_waits_for_trust() {
        let root = std::env::temp_dir().join(format!("neru-status-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        set_status_command(&root, Some("echo hi")).unwrap();
        assert_eq!(status_command(&root), None, "untrusted");
        crate::trust::trust_project(&root).unwrap();
        assert_eq!(status_command(&root).as_deref(), Some("echo hi"));
        assert_eq!(run_status_line(&root, "echo neru-status", &json!({})).as_deref(), Some("neru-status"));
        set_status_command(&root, None).unwrap();
        assert_eq!(status_command(&root), None);
        let _ = crate::trust::untrust_project(&root);
        let _ = fs::remove_dir_all(root);
    }
}
