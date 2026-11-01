//! Language servers (typescript-language-server, pyright, rust-analyzer, gopls) found on PATH,
//! started lazily, one per language and project, spoken to over stdio JSON-RPC. After an edit
//! Neru sends the new text and waits briefly for the server's diagnostics, so type errors reach
//! the agent with the edit that caused them. Nothing here is required: without a server the
//! edit-time check is tree-sitter syntax only.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Condvar, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

/// How long an edit waits for the server's verdict.
const EDIT_WAIT: Duration = Duration::from_millis(1_500);
/// How long the `diagnostics` tool waits, which may include the server starting.
const TOOL_WAIT: Duration = Duration::from_secs(12);
const MAX_SERVERS: usize = 8;

/// A server Neru knows how to run: the language key, the program and its arguments.
struct Spec {
    key: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

const SPECS: &[Spec] = &[
    Spec { key: "typescript", program: "typescript-language-server", args: &["--stdio"] },
    Spec { key: "python", program: "pyright-langserver", args: &["--stdio"] },
    Spec { key: "rust", program: "rust-analyzer", args: &[] },
    Spec { key: "go", program: "gopls", args: &[] },
];

/// The server key and LSP language id for a path.
fn language(path: &str) -> Option<(&'static str, &'static str)> {
    let ext = path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "ts" | "mts" | "cts" => ("typescript", "typescript"),
        "tsx" => ("typescript", "typescriptreact"),
        "js" | "mjs" | "cjs" => ("typescript", "javascript"),
        "jsx" => ("typescript", "javascriptreact"),
        "py" | "pyw" => ("python", "python"),
        "rs" => ("rust", "rust"),
        "go" => ("go", "go"),
        _ => return None,
    })
}

fn disabled() -> bool {
    cfg!(test) || std::env::var_os("NERU_NO_LSP").is_some_and(|v| v != "0")
}

/// The program on PATH (npm installs `.cmd` shims on Windows).
fn which(program: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| exts.iter().map(|ext| dir.join(format!("{program}{ext}"))).find(|path| path.is_file()))
}

/// One diagnostic as the server reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// 1-based.
    pub line: u64,
    pub column: u64,
    /// 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u64,
    pub message: String,
    pub source: String,
}

impl Diagnostic {
    fn from_value(value: &Value) -> Self {
        let start = &value["range"]["start"];
        Diagnostic {
            line: start["line"].as_u64().unwrap_or(0) + 1,
            column: start["character"].as_u64().unwrap_or(0) + 1,
            severity: value["severity"].as_u64().unwrap_or(1),
            message: value["message"].as_str().unwrap_or("").lines().next().unwrap_or("").chars().take(300).collect(),
            source: value["source"].as_str().unwrap_or("").to_string(),
        }
    }

    pub fn render(&self, path: &str) -> String {
        let level = match self.severity {
            1 => "error",
            2 => "warning",
            3 => "info",
            _ => "hint",
        };
        let source = if self.source.is_empty() { String::new() } else { format!(" [{}]", self.source) };
        format!("{path}:{}:{} {level}: {}{source}", self.line, self.column, self.message)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A file URI, and the key the same file has whatever way a server spells its URI back
/// (`file:///d%3A/...` and `file:///D:/...` are one file).
fn uri_for(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let encoded: String = percent_encoding::utf8_percent_encode(&text, &URI_SET).to_string();
    if encoded.starts_with('/') { format!("file://{encoded}") } else { format!("file:///{encoded}") }
}

const URI_SET: percent_encoding::AsciiSet = percent_encoding::NON_ALPHANUMERIC.remove(b'/').remove(b':').remove(b'-').remove(b'_').remove(b'.').remove(b'~');

fn uri_key(uri: &str) -> String {
    let decoded = percent_encoding::percent_decode_str(uri).decode_utf8_lossy().replace('\\', "/");
    let path = decoded.trim_start_matches("file://").trim_start_matches('/');
    if cfg!(windows) { path.to_lowercase() } else { path.to_string() }
}

#[derive(Default)]
struct Published {
    /// Diagnostics per file key, with a counter that moves on every publish.
    files: HashMap<String, (u64, Vec<Diagnostic>)>,
    sequence: u64,
}

struct Server {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    next_id: AtomicU64,
    replies: Mutex<HashMap<u64, mpsc::Sender<Value>>>,
    published: Mutex<Published>,
    changed: Condvar,
    /// Open documents and their versions, by file key.
    open: Mutex<HashMap<String, i64>>,
    alive: AtomicBool,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = lock(&self.child).kill();
    }
}

impl Server {
    fn send(&self, message: &Value) -> Result<(), String> {
        let body = message.to_string();
        let mut stdin = lock(&self.stdin);
        stdin.write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).and_then(|_| stdin.flush()).map_err(|e| {
            self.alive.store(false, Ordering::Relaxed);
            e.to_string()
        })
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        lock(&self.replies).insert(id, sender);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let reply = receiver.recv_timeout(wait).map_err(|_| format!("{method} timed out"));
        lock(&self.replies).remove(&id);
        let reply = reply?;
        match reply.get("error") {
            Some(error) if !error.is_null() => Err(error["message"].as_str().unwrap_or("server error").to_string()),
            _ => Ok(reply["result"].clone()),
        }
    }

    /// Answers what the server asks of the client: settings and registrations get empty answers.
    fn answer(&self, message: &Value) {
        let result = match message["method"].as_str() {
            Some("workspace/configuration") => Value::Array(vec![Value::Null; message["params"]["items"].as_array().map_or(1, Vec::len)]),
            _ => Value::Null,
        };
        let _ = self.send(&json!({"jsonrpc": "2.0", "id": message["id"], "result": result}));
    }

    fn handle(&self, message: Value) {
        if message.get("method").is_some() && message.get("id").is_some() {
            self.answer(&message);
        } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if let Some(sender) = lock(&self.replies).remove(&id) {
                let _ = sender.send(message);
            }
        } else if message["method"] == "textDocument/publishDiagnostics" {
            let key = uri_key(message["params"]["uri"].as_str().unwrap_or(""));
            let list: Vec<Diagnostic> = message["params"]["diagnostics"].as_array().into_iter().flatten().map(Diagnostic::from_value).collect();
            let mut published = lock(&self.published);
            published.sequence += 1;
            let sequence = published.sequence;
            published.files.insert(key, (sequence, list));
            self.changed.notify_all();
        }
    }

    fn sequence_of(&self, key: &str) -> u64 {
        lock(&self.published).files.get(key).map_or(0, |(sequence, _)| *sequence)
    }

    /// Waits until the server publishes for `key` after `since`, and returns what it published.
    fn wait_for(&self, key: &str, since: u64, wait: Duration) -> Option<Vec<Diagnostic>> {
        let deadline = Instant::now() + wait;
        let mut published = lock(&self.published);
        loop {
            if let Some((sequence, list)) = published.files.get(key) {
                if *sequence > since {
                    return Some(list.clone());
                }
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            published = self.changed.wait_timeout(published, left).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    /// Sends `text` as the file's content (opening it first if needed) and returns the server's
    /// diagnostics for it, or None when it says nothing in time.
    fn sync(&self, path: &Path, language_id: &str, text: &str, wait: Duration) -> Option<Vec<Diagnostic>> {
        let uri = uri_for(path);
        let key = uri_key(&uri);
        let since = self.sequence_of(&key);
        let version = {
            let mut open = lock(&self.open);
            let entry = open.entry(key.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        let sent = if version == 1 {
            self.notify("textDocument/didOpen", json!({"textDocument": {"uri": uri, "languageId": language_id, "version": version, "text": text}}))
        } else {
            self.notify("textDocument/didChange", json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}))
        };
        sent.ok()?;
        self.wait_for(&key, since, wait)
    }
}

/// Servers by (project, language key). None marks a server that is starting, failed or absent.
static SERVERS: LazyLock<Mutex<HashMap<(PathBuf, &'static str), Option<Arc<Server>>>>> = LazyLock::new(Default::default);

fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn start(root: &Path, spec: &Spec) -> Result<Arc<Server>, String> {
    let program = which(spec.program).ok_or_else(|| format!("{} is not on PATH", spec.program))?;
    let mut command = Command::new(program);
    command.args(spec.args).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let server = Arc::new(Server {
        stdin: Mutex::new(stdin),
        child: Mutex::new(child),
        next_id: AtomicU64::new(1),
        replies: Mutex::default(),
        published: Mutex::default(),
        changed: Condvar::new(),
        open: Mutex::default(),
        alive: AtomicBool::new(true),
    });
    let reader = Arc::downgrade(&server);
    thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        while let Some(message) = read_message(&mut stdout) {
            let Some(server) = reader.upgrade() else { return };
            server.handle(message);
        }
        if let Some(server) = reader.upgrade() {
            server.alive.store(false, Ordering::Relaxed);
        }
    });
    let root_uri = uri_for(root);
    // rust-analyzer would otherwise run build scripts, proc macros and cargo check: project code
    // the user did not ask to run, and minutes of CPU. Syntax and name resolution are enough here.
    let options = if spec.key == "rust" {
        json!({"checkOnSave": false, "cargo": {"buildScripts": {"enable": false}}, "procMacro": {"enable": false}})
    } else {
        Value::Null
    };
    server.request(
        "initialize",
        json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "workspaceFolders": [{"uri": root_uri, "name": root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()}],
            "initializationOptions": options,
            "capabilities": {
                "textDocument": {"publishDiagnostics": {"relatedInformation": false}, "synchronization": {"didSave": false}},
                "workspace": {"configuration": true, "workspaceFolders": true}
            }
        }),
        Duration::from_secs(60),
    )?;
    server.notify("initialized", json!({}))?;
    Ok(server)
}

/// The running server for a path's language, starting it in the background the first time.
/// Returns None while it starts, or when there is none.
fn server(root: &Path, key: &'static str) -> Option<Arc<Server>> {
    if disabled() {
        return None;
    }
    let spec = SPECS.iter().find(|spec| spec.key == key)?;
    let slot = (root.to_path_buf(), key);
    let mut servers = lock(&SERVERS);
    match servers.get(&slot) {
        Some(Some(server)) if server.alive.load(Ordering::Relaxed) => return Some(server.clone()),
        Some(None) => return None,
        _ => {}
    }
    if which(spec.program).is_none() {
        servers.insert(slot, None);
        return None;
    }
    if servers.len() >= MAX_SERVERS {
        return None;
    }
    servers.insert(slot.clone(), None);
    let root = root.to_path_buf();
    thread::spawn(move || {
        let spec = SPECS.iter().find(|spec| spec.key == key).expect("known spec");
        let started = start(&root, spec).ok();
        if started.is_none() {
            log::info!("{} did not start for {}", spec.program, root.display());
        }
        lock(&SERVERS).insert(slot, started);
    });
    None
}

/// Waits up to `wait` for the server of `key` to be up.
fn server_within(root: &Path, key: &'static str, wait: Duration) -> Option<Arc<Server>> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(server) = server(root, key) {
            return Some(server);
        }
        let starting = matches!(lock(&SERVERS).get(&(root.to_path_buf(), key)), Some(None)) && SPECS.iter().any(|spec| spec.key == key && which(spec.program).is_some());
        if !starting || Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn errors(list: &[Diagnostic]) -> impl Iterator<Item = &Diagnostic> {
    list.iter().filter(|diagnostic| diagnostic.severity == 1)
}

/// Errors in `relative` after an edit from `before` to `after` that it did not have before (by
/// message, since lines move). Empty when no server runs for the language yet; the first edit
/// starts one for the next.
pub fn after_edit(root: &Path, relative: &str, before: &str, after: &str) -> Vec<Diagnostic> {
    let Some((key, language_id)) = language(relative) else { return Vec::new() };
    let Some(server) = server(root, key) else { return Vec::new() };
    let path = root.join(relative);
    let file = uri_key(&uri_for(&path));
    let known = lock(&server.open).contains_key(&file);
    let old: Vec<Diagnostic> = if known {
        lock(&server.published).files.get(&file).map(|(_, list)| list.clone()).unwrap_or_default()
    } else {
        // First sight of the file: let the server see the old text so its old errors are known.
        server.sync(&path, language_id, before, EDIT_WAIT / 2).unwrap_or_default()
    };
    let Some(now) = server.sync(&path, language_id, after, EDIT_WAIT) else { return Vec::new() };
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for diagnostic in errors(&old) {
        *seen.entry(&diagnostic.message).or_default() += 1;
    }
    errors(&now)
        .filter(|diagnostic| match seen.get_mut(diagnostic.message.as_str()) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .cloned()
        .collect()
}

/// Every diagnostic for one file, starting the server if needed. None without a server.
pub fn check_file(root: &Path, relative: &str, text: &str) -> Option<Vec<Diagnostic>> {
    let (key, language_id) = language(relative)?;
    let server = server_within(root, key, TOOL_WAIT)?;
    let mut list = server.sync(&root.join(relative), language_id, text, Duration::from_secs(6))?;
    list.sort_by_key(|diagnostic| (diagnostic.severity, diagnostic.line));
    Some(list)
}

/// Whether any language server runs for the project.
pub fn any_running(root: &Path) -> bool {
    lock(&SERVERS).iter().any(|((at, _), server)| at == root && server.is_some())
}

/// Errors and warnings every server reported for the project, by relative path, errors first.
pub fn project_diagnostics(root: &Path) -> Vec<(String, Vec<Diagnostic>)> {
    let root_key = uri_key(&uri_for(root));
    let servers: Vec<Arc<Server>> = lock(&SERVERS).iter().filter(|((at, _), _)| at == root).filter_map(|(_, server)| server.clone()).collect();
    let mut out = Vec::new();
    for server in servers {
        for (key, (_, list)) in lock(&server.published).files.iter() {
            let list: Vec<Diagnostic> = list.iter().filter(|diagnostic| diagnostic.severity <= 2).cloned().collect();
            if list.is_empty() {
                continue;
            }
            let relative = key.strip_prefix(&root_key).map(|rest| rest.trim_start_matches('/').to_string()).unwrap_or_else(|| key.clone());
            out.push((relative, list));
        }
    }
    out.sort_by_key(|(path, list)| (std::cmp::Reverse(errors(list).count()), path.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_uris() {
        let mut input = std::io::Cursor::new(b"Content-Length: 17\r\n\r\n{\"jsonrpc\":\"2.0\"}Content-Length: 2\r\n\r\n{}".to_vec());
        assert_eq!(read_message(&mut input).unwrap()["jsonrpc"], "2.0");
        assert_eq!(read_message(&mut input).unwrap(), json!({}));
        assert!(read_message(&mut input).is_none());
        if cfg!(windows) {
            let uri = uri_for(Path::new(r"D:\My Project\src\a.ts"));
            assert_eq!(uri, "file:///D:/My%20Project/src/a.ts");
            assert_eq!(uri_key(&uri), uri_key("file:///d%3A/My%20Project/src/a.ts"));
        }
        assert_eq!(language("src/App.tsx"), Some(("typescript", "typescriptreact")));
        assert_eq!(language("README.md"), None);
    }
}
