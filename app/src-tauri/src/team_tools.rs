//! Team helpers that work on the project rather than the thread: what a turn changed and undoing
//! it, worktree setup and teardown scripts, sweeping landed worktrees, opening a member in its own
//! terminal, and custom CLI agents.

use crate::Hidden;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::workspace::data_dir;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// "A", "M" or "D".
    pub status: String,
}

/// What one turn changed in a Git project: snapshots of the whole working tree (tracked and
/// untracked, ignored files left out) before and after, and the files that differ.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub root: String,
    pub before: String,
    pub after: String,
    pub files: Vec<FileChange>,
    #[serde(default)]
    pub undone: bool,
}

fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(root).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn output(mut command: Command) -> Option<String> {
    let out = command.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A tree object of the working tree as it is now, written through a copy of the real index so
/// Git only rehashes files that changed. None outside a Git work tree.
pub fn snapshot(root: &Path) -> Option<String> {
    let mut check = git(root);
    check.args(["rev-parse", "--is-inside-work-tree"]);
    if output(check)? != "true" {
        return None;
    }
    let mut index_path = git(root);
    index_path.args(["rev-parse", "--git-path", "index"]);
    let index = root.join(output(index_path)?);
    let temp = std::env::temp_dir().join(format!("neru-snapshot-{}.index", uuid::Uuid::new_v4()));
    if index.is_file() {
        std::fs::copy(&index, &temp).ok()?;
    }
    let with_index = |args: &[&str]| {
        let mut command = git(root);
        command.env("GIT_INDEX_FILE", &temp).args(args);
        output(command)
    };
    let tree = with_index(&["add", "-A", "--", "."]).and_then(|_| with_index(&["write-tree"]));
    let _ = std::fs::remove_file(&temp);
    tree
}

pub fn diff(root: &Path, before: &str, after: &str) -> Vec<FileChange> {
    if before == after {
        return Vec::new();
    }
    let mut command = git(root);
    command.args(["diff", "--name-status", "--no-renames", "-z", before, after]);
    let Some(text) = output(command) else { return Vec::new() };
    let parts: Vec<&str> = text.split('\0').filter(|part| !part.is_empty()).collect();
    let mut files: Vec<FileChange> = parts
        .chunks(2)
        .filter_map(|pair| Some(FileChange { status: pair.first()?.chars().next()?.to_string(), path: pair.get(1)?.to_string() }))
        // oh-my-claudecode keeps its runtime state in .omc; it is never the turn's work.
        .filter(|file| !file.path.starts_with(".omc/") && !file.path.contains("/.omc/"))
        .collect();
    // The project's own files first; tool state in dot-folders (.omc, .claude) after.
    files.sort_by_key(|file| (file.path.starts_with('.') || file.path.contains("/."), file.path.clone()));
    files
}

/// Puts every file the turn changed back as it was before the turn. Later edits to those files go
/// too; the conversation stays as it is.
pub fn undo(changes: &Changes) -> Result<usize, String> {
    let root = Path::new(&changes.root);
    let mut restored = 0;
    for file in &changes.files {
        let target = root.join(&file.path);
        if file.status == "A" {
            if target.is_file() {
                std::fs::remove_file(&target).map_err(|e| format!("{}: {e}", file.path))?;
            }
        } else {
            let mut show = git(root);
            show.args(["show", &format!("{}:{}", changes.before, file.path)]);
            let out = show.output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!("Could not read the earlier {}", file.path));
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, out.stdout).map_err(|e| format!("{}: {e}", file.path))?;
        }
        restored += 1;
    }
    Ok(restored)
}

/// A repository's worktree scripts, from `.neru/environment.json` or Traycer's
/// `.traycer/environment.json`: `{"setup": "…" | {"default","windows","macos","linux"}, "teardown": …}`.
pub fn script(root: &Path, which: &str) -> Option<String> {
    let value: Value = [".neru", ".traycer"]
        .iter()
        .find_map(|dir| std::fs::read_to_string(root.join(dir).join("environment.json")).ok())
        .and_then(|text| serde_json::from_str(&text).ok())?;
    let entry = &value[which];
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
    entry.as_str().or_else(|| entry[os].as_str()).or_else(|| entry["default"].as_str()).map(String::from).filter(|text| !text.trim().is_empty())
}

/// Runs a script in `cwd` through the user's shell, with a time limit. Returns (ok, output tail).
pub async fn run_script(command: &str, cwd: &Path, limit: Duration) -> (bool, String) {
    let mut process = crate::shells::command(command);
    process.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let Ok(child) = process.spawn() else { return (false, "The script could not start".into()) };
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
            (out.status.success(), lines[lines.len().saturating_sub(12)..].join("\n"))
        }
        Ok(Err(error)) => (false, error.to_string()),
        Err(_) => (false, format!("Stopped after {} seconds", limit.as_secs())),
    }
}

/// Why a member's worktree can or cannot be swept away: it must be clean and its branch must be in
/// the base already (landed) or have no commits of its own.
pub fn sweep_state(project: &Path, tree: &crate::sessions::Worktree) -> Result<&'static str, &'static str> {
    let path = Path::new(&tree.path);
    if !path.is_dir() {
        return Ok("missing");
    }
    let mut status = git(path);
    status.args(["status", "--porcelain"]);
    match output(status) {
        Some(text) if !text.is_empty() => return Err("has uncommitted work"),
        None => return Err("status unknown"),
        _ => {}
    }
    let mut ahead = git(project);
    ahead.args(["rev-list", "--count", &format!("{}..{}", tree.base, tree.branch)]);
    match output(ahead).and_then(|count| count.parse::<u32>().ok()) {
        Some(0) => Ok("landed"),
        Some(_) => Err("has commits not in the base branch"),
        None => Err("status unknown"),
    }
}

/// A worktree's uncommitted files and the commits its branch has that its base does not.
pub fn worktree_info(project: &Path, tree: &crate::sessions::Worktree) -> (usize, usize) {
    let mut status = git(Path::new(&tree.path));
    status.args(["status", "--porcelain"]);
    let dirty = output(status).map(|text| text.lines().filter(|line| !line.trim().is_empty()).count()).unwrap_or(0);
    let mut ahead = git(project);
    ahead.args(["rev-list", "--count", &format!("{}..{}", tree.base, tree.branch)]);
    (dirty, output(ahead).and_then(|count| count.parse().ok()).unwrap_or(0))
}

/// One changed file across a whole task, as a unified diff.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TaskFile {
    pub path: String,
    pub diff: String,
    pub additions: usize,
    pub deletions: usize,
    pub status: String,
    /// Which checkout it is in: "project" or a member's handle.
    pub place: String,
}

/// Every file changed in `root` between the tree `before` and now, with its unified diff.
pub fn changes_since(root: &Path, before: &str, place: &str) -> Vec<TaskFile> {
    let Some(now) = snapshot(root) else { return Vec::new() };
    diff(root, before, &now)
        .into_iter()
        .map(|file| {
            let mut command = git(root);
            command.args(["diff", "--no-color", "--no-ext-diff", before, &now, "--", &file.path]);
            let text = output(command).unwrap_or_default();
            let additions = text.lines().filter(|line| line.starts_with('+') && !line.starts_with("+++")).count();
            let deletions = text.lines().filter(|line| line.starts_with('-') && !line.starts_with("---")).count();
            TaskFile { path: file.path, diff: text, additions, deletions, status: file.status, place: place.into() }
        })
        .collect()
}

/// Opens a terminal window running `program args` in `cwd`.
pub fn open_terminal(program: &Path, args: &[String], cwd: &Path, env: &[(String, String)]) -> Result<(), String> {
    let quoted: Vec<String> = args.iter().map(|arg| if arg.contains(' ') { format!("\"{arg}\"") } else { arg.clone() }).collect();
    let line = format!("\"{}\" {}", program.display(), quoted.join(" "));
    let cwd = cwd.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        // cmd parses its own quotes, so the line goes through untouched.
        Command::new("cmd").raw_arg(format!("/C start \"\" /D \"{cwd}\" cmd /K {line}")).envs(env.iter().map(|(k, v)| (k, v))).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = Command::new("osascript")
        .envs(env.iter().map(|(k, v)| (k, v)))
        .args(["-e", &format!("tell application \"Terminal\" to do script \"cd '{cwd}' && {}\"", line.replace('"', "\\\""))])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("x-terminal-emulator").envs(env.iter().map(|(k, v)| (k, v))).args(["-e", "sh", "-c", &format!("cd '{cwd}' && {line}; exec $SHELL")]).spawn();
    result.map(|_| ()).map_err(|e| format!("Could not open a terminal: {e}"))
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomAgent {
    /// "custom:<name>", the member kind.
    pub kind: String,
    pub name: String,
    pub path: String,
}

/// Your own agents: scripts in Neru's `cli-agents` folder or the project's `.neru/cli-agents`.
/// Each gets the prompt in NERU_PROMPT (and a file in NERU_PROMPT_FILE) and prints its reply.
pub fn custom_agents(project: Option<&Path>) -> Vec<CustomAgent> {
    let mut dirs: Vec<PathBuf> = data_dir().map(|dir| vec![dir.join("cli-agents")]).unwrap_or_default();
    if let Some(project) = project {
        dirs.push(project.join(".neru").join("cli-agents"));
    }
    let runnable = |path: &Path| {
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_lowercase();
        if cfg!(windows) { matches!(ext.as_str(), "cmd" | "bat" | "ps1") } else { matches!(ext.as_str(), "sh" | "") }
    };
    let mut found: Vec<CustomAgent> = Vec::new();
    for dir in dirs {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()) else { continue };
            if !path.is_file() || !runnable(&path) || !stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                continue;
            }
            // A project's script wins over a personal one with the same name.
            found.retain(|agent| agent.name != stem);
            found.push(CustomAgent { kind: format!("custom:{stem}"), name: stem, path: path.to_string_lossy().into_owned() });
        }
    }
    found
}

/// Runs a custom agent script for one turn; every line it prints is part of the reply.
pub async fn run_custom(
    script: &Path,
    root: &Path,
    env: &[(&str, String)],
    prompt: &str,
    cancel: Arc<tokio::sync::Notify>,
    mut on_line: impl FnMut(&str),
) -> Result<String, String> {
    let prompt_file = std::env::temp_dir().join(format!("neru-prompt-{}.md", uuid::Uuid::new_v4()));
    std::fs::write(&prompt_file, prompt).map_err(|e| e.to_string())?;
    let ext = script.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_lowercase();
    let mut command = match ext.as_str() {
        "ps1" => {
            let mut command = tokio::process::Command::new("powershell.exe").hidden();
            command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(script);
            command
        }
        "sh" => {
            let mut command = tokio::process::Command::new("sh").hidden();
            command.arg(script);
            command
        }
        _ => tokio::process::Command::new(script),
    };
    command
        .current_dir(root)
        .env("NERU_PROMPT", prompt)
        .env("NERU_PROMPT_FILE", &prompt_file)
        .envs(env.iter().map(|(key, value)| (*key, value.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| format!("Could not start {}: {e}", script.display()))?;
    let stderr = child.stderr.take().map(|mut stream| {
        tokio::spawn(async move {
            let mut text = String::new();
            let _ = stream.read_to_string(&mut text).await;
            text
        })
    });
    let mut lines = BufReader::new(child.stdout.take().ok_or("No output from the agent")?).lines();
    let mut reply = String::new();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    reply.push_str(&line);
                    reply.push('\n');
                    on_line(&reply);
                }
                _ => break,
            },
            _ = cancel.notified() => {
                crate::shells::kill_tree(&mut child);
                let _ = std::fs::remove_file(&prompt_file);
                return Err("Stopped".into());
            }
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&prompt_file);
    let stderr = match stderr {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if !status.success() && reply.trim().is_empty() {
        let tail: Vec<&str> = stderr.lines().filter(|line| !line.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(6)..].join("\n");
        return Err(if tail.is_empty() { format!("The agent exited with {status}") } else { tail });
    }
    Ok(reply.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        let root = std::env::temp_dir().join(format!("neru-team-tools-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        for args in [&["init", "-q"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"]] {
            git(&root).args(args).status().unwrap();
        }
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git(&root).args(["add", "."]).status().unwrap();
        git(&root).args(["commit", "-qm", "init"]).status().unwrap();
        root
    }

    #[test]
    fn a_custom_agent_is_found_and_answers_from_its_prompt() {
        let root = std::env::temp_dir().join(format!("neru-custom-agent-{}", uuid::Uuid::new_v4()));
        let dir = root.join(".neru").join("cli-agents");
        std::fs::create_dir_all(&dir).unwrap();
        let (name, body) = if cfg!(windows) {
            ("echoer.cmd", "@echo off\r\necho got:\r\ntype \"%NERU_PROMPT_FILE%\"\r\n")
        } else {
            ("echoer.sh", "echo got:\ncat \"$NERU_PROMPT_FILE\"\n")
        };
        std::fs::write(dir.join(name), body).unwrap();
        let found = custom_agents(Some(&root));
        let agent = found.iter().find(|agent| agent.kind == "custom:echoer").expect("script listed as an agent");
        let mut streamed = String::new();
        let reply = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_custom(Path::new(&agent.path), &root, &[], "hello team", Arc::new(tokio::sync::Notify::new()), |text| streamed = text.to_string()))
            .unwrap();
        assert!(reply.starts_with("got:"), "{reply}");
        assert!(reply.contains("hello team"), "{reply}");
        assert!(!streamed.is_empty());
    }

    #[test]
    fn a_turn_is_seen_and_undone_with_new_and_untracked_files() {
        let root = repo();
        std::fs::write(root.join("dirty.txt"), "kept before\n").unwrap();
        let before = snapshot(&root).unwrap();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        std::fs::write(root.join("dirty.txt"), "changed\n").unwrap();
        // A plugin's state written during the turn is not the turn's work.
        std::fs::create_dir_all(root.join(".omc/state")).unwrap();
        std::fs::write(root.join(".omc/state/session.json"), "{}").unwrap();
        let after = snapshot(&root).unwrap();
        let files = diff(&root, &before, &after);
        let mut paths: Vec<_> = files.iter().map(|file| format!("{} {}", file.status, file.path)).collect();
        paths.sort();
        assert_eq!(paths, ["A new.txt", "M a.txt", "M dirty.txt"]);
        std::fs::remove_dir_all(root.join(".omc")).unwrap();
        let changes = Changes { root: root.to_string_lossy().into(), before, after, files, undone: false };
        assert_eq!(undo(&changes).unwrap(), 3);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(root.join("dirty.txt")).unwrap(), "kept before\n");
        assert!(!root.join("new.txt").exists());
        // The real index is untouched.
        let status = output({ let mut c = git(&root); c.args(["status", "--porcelain"]); c }).unwrap();
        assert_eq!(status, "?? dirty.txt");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn environment_scripts_are_read_per_platform() {
        let root = std::env::temp_dir().join(format!("neru-env-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".traycer")).unwrap();
        std::fs::write(root.join(".traycer/environment.json"), r#"{"setup": {"default": "npm ci"}, "teardown": "echo bye"}"#).unwrap();
        assert_eq!(script(&root, "setup").as_deref(), Some("npm ci"));
        assert_eq!(script(&root, "teardown").as_deref(), Some("echo bye"));
        assert!(script(&root, "other").is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
