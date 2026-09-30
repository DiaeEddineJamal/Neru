//! Shell commands the agent leaves running in the background, the way Claude Code's
//! run_in_background works: dev servers, watchers and long builds. Each keeps the last lines of
//! its output; the agent reads what is new with shell_output and stops it with kill_shell.
//! Shells belong to the session that started them and end with it, and with the app.

use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    process::Stdio,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

/// Lines of output each shell keeps; older lines fall out.
pub const MAX_LINES: usize = 2_000;
/// A single line longer than this is cut, so one minified bundle cannot fill the buffer.
const MAX_LINE_CHARS: usize = 2_000;
/// Output one read hands back; the rest of a long burst is summarized as skipped.
const MAX_READ_CHARS: usize = 30_000;
/// Shells one session may have running at once.
const MAX_RUNNING: usize = 8;

/// The last lines a shell printed, and how far the agent has read.
#[derive(Default, Debug)]
pub struct Buffer {
    lines: VecDeque<String>,
    /// Lines ever pushed.
    total: usize,
    /// Lines ever handed out by `take_new`.
    read: usize,
}

impl Buffer {
    pub fn push(&mut self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        let line = if line.chars().count() > MAX_LINE_CHARS {
            format!("{}…", line.chars().take(MAX_LINE_CHARS).collect::<String>())
        } else {
            line.to_string()
        };
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.total += 1;
    }

    /// Lines printed since the last call, and how many unread lines fell out of the buffer first.
    pub fn take_new(&mut self) -> (Vec<String>, usize) {
        let first_kept = self.total - self.lines.len();
        let start = self.read.max(first_kept);
        let dropped = start - self.read;
        let new = self.lines.iter().skip(start - first_kept).cloned().collect();
        self.read = self.total;
        (new, dropped)
    }

    pub fn total(&self) -> usize {
        self.total
    }
}

/// The lines matching `pattern`, a regular expression. An invalid pattern is an error.
pub fn filter_lines(lines: Vec<String>, pattern: &str) -> Result<Vec<String>, String> {
    let regex = compile(pattern)?;
    Ok(lines.into_iter().filter(|line| regex.is_match(line)).collect())
}

fn compile(pattern: &str) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(pattern).size_limit(1 << 20).build().map_err(|e| format!("filter is not a valid regular expression: {e}"))
}

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Running,
    Exited(Option<i32>),
    /// Stopped by kill_shell, the session or the app ending, or its time limit; says which.
    Killed(String),
}

struct Shell {
    session: String,
    command: String,
    started: Instant,
    ended: Option<Instant>,
    output: Arc<Mutex<Buffer>>,
    child: tokio::process::Child,
    status: Status,
}

impl Shell {
    /// Notices a process that exited on its own.
    fn refresh(&mut self) {
        if self.status == Status::Running {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.status = Status::Exited(status.code());
                self.ended = Some(Instant::now());
            }
        }
    }

    fn stop(&mut self, reason: &str) {
        self.refresh();
        if self.status != Status::Running {
            return;
        }
        kill_tree(&mut self.child);
        self.status = Status::Killed(reason.into());
        self.ended = Some(Instant::now());
    }

    fn elapsed(&self) -> Duration {
        self.ended.unwrap_or_else(Instant::now).duration_since(self.started)
    }

    fn describe(&self) -> String {
        let secs = self.elapsed().as_secs();
        match &self.status {
            Status::Running => format!("running for {secs}s"),
            Status::Exited(Some(code)) => format!("exited with code {code} after {secs}s"),
            Status::Exited(None) => format!("exited after {secs}s"),
            Status::Killed(reason) => format!("stopped ({reason}) after {secs}s"),
        }
    }
}

static SHELLS: LazyLock<Mutex<HashMap<String, Shell>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT: AtomicUsize = AtomicUsize::new(1);

/// The shell a command runs in: PowerShell on Windows, `sh -lc` elsewhere. Shared with the
/// agent's foreground commands.
pub(crate) fn command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut process = tokio::process::Command::new("powershell.exe");
        process.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            // The storage script only exists on the development machine; elsewhere run the command as is.
            &format!("if (Test-Path 'D:\\Neru\\Use-NeruStorage.ps1') {{ . 'D:\\Neru\\Use-NeruStorage.ps1' }}; {command}"),
        ]);
        process
    }
    #[cfg(not(windows))]
    {
        let mut process = tokio::process::Command::new("sh");
        process.args(["-lc", command]);
        process
    }
}

/// Ends the process and everything it started (a shell wraps npm, which wraps node).
fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The shell leads its own process group, so the group goes with it.
        let _ = std::process::Command::new("kill").args(["-TERM", "--", &format!("-{pid}")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.start_kill();
}

fn read_into(stream: impl AsyncRead + Unpin + Send + 'static, output: Arc<Mutex<Buffer>>) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(mut output) = output.lock() {
                        output.push(&String::from_utf8_lossy(&line));
                    }
                }
            }
        }
    });
}

/// Starts `command` in `root` without waiting for it and returns its id, like `shell-3`. With
/// `timeout` it is stopped after that long.
pub fn start(session: &str, root: &Path, command_text: &str, timeout: Option<Duration>) -> Result<String, String> {
    {
        let mut shells = SHELLS.lock().map_err(|e| e.to_string())?;
        let running = shells
            .values_mut()
            .filter(|shell| shell.session == session)
            .map(|shell| {
                shell.refresh();
                shell.status == Status::Running
            })
            .filter(|running| *running)
            .count();
        if running >= MAX_RUNNING {
            return Err(format!("{MAX_RUNNING} background shells are already running in this session. Stop one with kill_shell first."));
        }
    }
    let mut process = command(command_text);
    process
        .current_dir(root)
        // Keeps dev servers from opening a browser tab of their own.
        .env("BROWSER", "none")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    process.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process.spawn().map_err(|e| format!("Could not start the command: {e}"))?;
    let output = Arc::new(Mutex::new(Buffer::default()));
    if let Some(stdout) = child.stdout.take() {
        read_into(stdout, output.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        read_into(stderr, output.clone());
    }
    let id = format!("shell-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    SHELLS.lock().map_err(|e| e.to_string())?.insert(
        id.clone(),
        Shell { session: session.into(), command: command_text.into(), started: Instant::now(), ended: None, output, child, status: Status::Running },
    );
    if let Some(limit) = timeout {
        let id = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(limit).await;
            if let Ok(mut shells) = SHELLS.lock() {
                if let Some(shell) = shells.get_mut(&id) {
                    shell.stop(&format!("hit its {}s time limit", limit.as_secs()));
                }
            }
        });
    }
    Ok(id)
}

fn not_found(id: &str) -> String {
    format!("No background shell {id} in this session. Background shells get ids like shell-1 when run_shell_command starts them with run_in_background.")
}

/// What the shell printed since the last read (only lines matching `filter`, a regex, when
/// given), after a status line.
pub fn output(session: &str, id: &str, filter: Option<&str>) -> Result<String, String> {
    let filter = filter.map(str::trim).filter(|pattern| !pattern.is_empty());
    // Checked before reading, so a bad pattern does not use up the output.
    if let Some(pattern) = filter {
        compile(pattern)?;
    }
    let mut shells = SHELLS.lock().map_err(|e| e.to_string())?;
    let shell = shells.get_mut(id).filter(|shell| shell.session == session).ok_or_else(|| not_found(id))?;
    shell.refresh();
    let (lines, dropped) = shell.output.lock().map_err(|e| e.to_string())?.take_new();
    let lines = match filter {
        Some(pattern) => filter_lines(lines, pattern)?,
        None => lines,
    };
    Ok(render(&shell.command, &shell.describe(), lines, dropped, filter.is_some()))
}

fn render(command: &str, status: &str, lines: Vec<String>, dropped: usize, filtered: bool) -> String {
    let mut text = format!("Command: {command}\nStatus: {status}\n");
    if dropped > 0 {
        text.push_str(&format!("[{dropped} earlier lines were dropped before this read]\n"));
    }
    if lines.is_empty() {
        text.push_str(if filtered { "(no new output matches the filter)" } else { "(no new output)" });
        return text;
    }
    // Keep the end of a long burst: the latest lines say the most about where it is now.
    let mut kept = Vec::new();
    let mut size = 0;
    for line in lines.iter().rev() {
        size += line.len() + 1;
        if size > MAX_READ_CHARS {
            break;
        }
        kept.push(line.as_str());
    }
    if kept.len() < lines.len() {
        text.push_str(&format!("[{} more lines before these]\n", lines.len() - kept.len()));
    }
    kept.reverse();
    text.push_str(&kept.join("\n"));
    text
}

/// Stops a background shell of this session. Its output can still be read afterwards.
pub fn kill(session: &str, id: &str) -> Result<String, String> {
    let mut shells = SHELLS.lock().map_err(|e| e.to_string())?;
    let shell = shells.get_mut(id).filter(|shell| shell.session == session).ok_or_else(|| not_found(id))?;
    shell.refresh();
    if shell.status != Status::Running {
        return Ok(format!("{id} ({}) was not running: {}.", shell.command, shell.describe()));
    }
    shell.stop("kill_shell");
    Ok(format!("Stopped {id} ({}).", shell.command))
}

/// Stops and forgets every shell a session started, when the session is deleted.
pub fn kill_session(session: &str) {
    if let Ok(mut shells) = SHELLS.lock() {
        shells.retain(|_, shell| {
            if shell.session != session {
                return true;
            }
            shell.stop("session deleted");
            false
        });
    }
}

/// Stops every background shell; called as the app or the CLI exits.
pub fn kill_all() {
    if let Ok(mut shells) = SHELLS.lock() {
        for shell in shells.values_mut() {
            shell.stop("Neru closed");
        }
    }
}

/// A background shell as the window and the CLI list it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShellView {
    pub id: String,
    pub session_id: String,
    pub command: String,
    /// "running", "exited" or "killed".
    pub status: String,
    pub exit_code: Option<i32>,
    /// Human-readable status, like "running for 42s".
    pub detail: String,
    pub elapsed_secs: u64,
    /// Lines printed so far.
    pub lines: usize,
}

/// Background shells, newest first; only one session's when `session` is given.
pub fn list(session: Option<&str>) -> Vec<ShellView> {
    let Ok(mut shells) = SHELLS.lock() else { return Vec::new() };
    let mut views: Vec<(Instant, ShellView)> = shells
        .iter_mut()
        .filter(|(_, shell)| session.is_none_or(|session| shell.session == session))
        .map(|(id, shell)| {
            shell.refresh();
            let (status, exit_code) = match &shell.status {
                Status::Running => ("running", None),
                Status::Exited(code) => ("exited", *code),
                Status::Killed(_) => ("killed", None),
            };
            let view = ShellView {
                id: id.clone(),
                session_id: shell.session.clone(),
                command: shell.command.clone(),
                status: status.into(),
                exit_code,
                detail: shell.describe(),
                elapsed_secs: shell.elapsed().as_secs(),
                lines: shell.output.lock().map(|output| output.total()).unwrap_or(0),
            };
            (shell.started, view)
        })
        .collect();
    views.sort_by(|a, b| b.0.cmp(&a.0));
    views.into_iter().map(|(_, view)| view).collect()
}

/// The background shells the agent started, for the window.
#[tauri::command]
pub fn list_shells(session_id: Option<String>) -> Vec<ShellView> {
    list(session_id.as_deref())
}

/// Stops a background shell from the window.
#[tauri::command]
pub fn stop_shell(id: String) -> Result<(), String> {
    let mut shells = SHELLS.lock().map_err(|e| e.to_string())?;
    let shell = shells.get_mut(&id).ok_or_else(|| format!("No background shell {id}"))?;
    shell.stop("stopped by the user");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_hands_out_only_new_lines() {
        let mut buffer = Buffer::default();
        buffer.push("one\r\n");
        buffer.push("two\n");
        assert_eq!(buffer.take_new(), (vec!["one".to_string(), "two".to_string()], 0));
        assert_eq!(buffer.take_new(), (vec![], 0));
        buffer.push("three");
        assert_eq!(buffer.take_new(), (vec!["three".to_string()], 0));
    }

    #[test]
    fn buffer_keeps_the_last_lines_and_counts_what_fell_out() {
        let mut buffer = Buffer::default();
        for n in 0..MAX_LINES + 5 {
            buffer.push(&n.to_string());
        }
        let (lines, dropped) = buffer.take_new();
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(dropped, 5);
        assert_eq!(lines[0], "5");
        assert_eq!(lines.last().map(String::as_str), Some(&*(MAX_LINES + 4).to_string()));
        // Partly read, then overflowed: only the unread lines that fell out count as dropped.
        for n in 0..10 {
            buffer.push(&format!("x{n}"));
        }
        let (lines, dropped) = buffer.take_new();
        assert_eq!((lines.len(), dropped), (10, 0));
        assert_eq!(buffer.total(), MAX_LINES + 15);
    }

    #[test]
    fn long_lines_are_cut() {
        let mut buffer = Buffer::default();
        buffer.push(&"a".repeat(MAX_LINE_CHARS * 2));
        let (lines, _) = buffer.take_new();
        assert_eq!(lines[0].chars().count(), MAX_LINE_CHARS + 1);
    }

    #[test]
    fn filter_keeps_matching_lines_and_rejects_bad_patterns() {
        let lines = vec!["ready in 300ms".to_string(), "GET / 200".to_string(), "error: port in use".to_string()];
        assert_eq!(filter_lines(lines.clone(), "(?i)error|ready").unwrap(), vec!["ready in 300ms", "error: port in use"]);
        assert!(filter_lines(lines, "(unclosed").is_err());
    }

    #[test]
    fn render_reports_status_drops_and_empty_reads() {
        let text = render("npm run dev", "running for 3s", vec![], 0, false);
        assert!(text.contains("Status: running for 3s") && text.ends_with("(no new output)"));
        let text = render("npm run dev", "running for 3s", vec!["a".into(), "b".into()], 7, false);
        assert!(text.contains("[7 earlier lines were dropped") && text.ends_with("a\nb"));
        let many: Vec<String> = (0..20_000).map(|n| format!("line {n}")).collect();
        let text = render("x", "running", many, 0, false);
        assert!(text.len() < MAX_READ_CHARS + 200 && text.ends_with("line 19999") && text.contains("more lines before these"));
    }

    #[tokio::test]
    async fn runs_in_the_background_and_stops() {
        let root = std::env::temp_dir();
        let id = start("test-session", &root, "echo neru-ready", None).unwrap();
        let mut text = String::new();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            text.push_str(&output("test-session", &id, None).unwrap());
            if text.contains("neru-ready") && text.contains("exited") {
                break;
            }
        }
        assert!(text.contains("neru-ready") && text.contains("exited with code 0"), "{text}");
        assert!(output("other-session", &id, None).is_err(), "shells are per session");
        let sleeper = start("test-session", &root, if cfg!(windows) { "Start-Sleep -Seconds 60" } else { "sleep 60" }, None).unwrap();
        assert!(list(Some("test-session")).iter().any(|shell| shell.id == sleeper && shell.status == "running"));
        assert!(kill("test-session", &sleeper).unwrap().starts_with("Stopped"));
        assert!(output("test-session", &sleeper, None).unwrap().contains("stopped (kill_shell)"));
        kill_session("test-session");
        assert!(list(Some("test-session")).is_empty());
    }

    #[test]
    fn unknown_shells_are_errors() {
        assert!(output("s", "shell-999999", None).is_err());
        assert!(kill("s", "shell-999999").is_err());
        assert!(output("s", "shell-1", Some("(")).unwrap_err().contains("regular expression"));
    }
}
