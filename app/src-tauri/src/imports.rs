//! Imports: finds the coding agents and editors already on this machine and brings their work into
//! Neru. Chat history becomes Team tasks (one per project, one member per conversation, set to
//! resume it where the CLI can), MCP servers become connectors and personal instructions join
//! Neru's AGENTS.md. Skills are listed here and copied with `skills::import_skills`. The other
//! tools' files are only read, never changed.
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, mcp, team, workspace::data_dir};

/// Longest post an imported message becomes; the rest stays in the original tool.
const MAX_POST: usize = 20_000;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: &'static str,
    pub name: &'static str,
    /// "agent", "app" or "editor".
    pub kind: &'static str,
    pub found: bool,
    pub path: String,
    pub chats: usize,
    pub skills: usize,
    pub servers: usize,
    pub rules: usize,
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    /// "source:id", what `import_chats` takes.
    pub key: String,
    pub source: &'static str,
    pub cwd: String,
    pub title: String,
    pub updated: u64,
    pub messages: usize,
    pub imported: bool,
    /// Listed but cannot be imported: the file is broken or holds no messages.
    pub unreadable: bool,
    pub reason: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FoundSkill {
    pub source: &'static str,
    pub name: String,
    pub path: String,
    pub description: String,
    /// Neru already has a personal skill by this name; importing replaces it.
    pub conflict: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FoundServer {
    pub key: String,
    pub source: &'static str,
    pub name: String,
    /// The command or URL, for recognising it; secrets stay on the Rust side.
    pub target: String,
    pub conflict: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FoundRules {
    pub source: &'static str,
    pub path: String,
    /// Where it goes: Neru's own AGENTS.md, or the project's .neru/instructions.md.
    pub scope: &'static str,
    pub size: u64,
    pub imported: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub sources: Vec<Source>,
    pub chats: Vec<Chat>,
    pub skills: Vec<FoundSkill>,
    pub servers: Vec<FoundServer>,
    pub rules: Vec<FoundRules>,
}

/// Where a listed chat lives, so `import_chats` can read it in full.
#[derive(Clone, Debug)]
enum Located {
    File(PathBuf),
    OpenCode(String),
    Cursor(PathBuf),
}

/// One conversation read from another tool.
#[derive(Debug, Default)]
struct Convo {
    cwd: String,
    title: String,
    /// The CLI session to resume, when the member's CLI can.
    upstream: Option<String>,
    /// (from the user, text), consecutive messages from one side merged.
    messages: Vec<(bool, String)>,
}

static FOUND: LazyLock<Mutex<HashMap<String, (Located, Chat)>>> = LazyLock::new(Default::default);
static SERVERS: LazyLock<Mutex<HashMap<String, mcp::ServerConfig>>> = LazyLock::new(Default::default);

#[derive(Serialize, Deserialize, Default)]
struct Ledger {
    #[serde(default)]
    chats: HashSet<String>,
    #[serde(default)]
    rules: HashSet<String>,
}

fn ledger_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("imports.json"))
}

fn ledger() -> Ledger {
    ledger_path().ok().and_then(|path| fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn save_ledger(ledger: &Ledger) -> Result<(), String> {
    fs::write(ledger_path()?, serde_json::to_vec_pretty(ledger).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// The per-user settings folder apps keep their data in: %APPDATA%, ~/Library/Application Support
/// or ~/.config.
fn app_dir(name: &str) -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| home().join(".config")).join(name)
}

fn modified(path: &Path) -> u64 {
    fs::metadata(path).and_then(|meta| meta.modified()).ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map_or(0, |time| time.as_millis() as u64)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Files under `dir` (up to `depth` folders down) that `keep` accepts.
fn files(dir: &Path, depth: usize, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), 0)];
    while let Some((dir, level)) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if level < depth {
                    pending.push((path, level + 1));
                }
            } else if keep(&path) {
                found.push(path);
            }
        }
    }
    found
}

fn ext(path: &Path, wanted: &str) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(wanted))
}

fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The first line of text, without the tags tools wrap pasted content in.
fn title_from(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    clip(plain.lines().find(|line| !line.trim().is_empty()).unwrap_or("Untitled").trim(), 80)
}

/// Text of a message's content: a string, or the text blocks of a content array.
fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| matches!(block["type"].as_str(), Some("text" | "input_text" | "output_text")))
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Wrappers the tools add around the user's words: command echoes, environment context, injected
/// instructions. They are not part of the conversation.
fn is_noise(text: &str) -> bool {
    let text = text.trim_start();
    text.is_empty()
        || ["<command-", "<local-command", "Caveat:", "<environment_context>", "<user_instructions>", "<permissions", "# AGENTS.md", "<INSTRUCTIONS>", "<system-reminder>", "[Request interrupted"]
            .iter()
            .any(|marker| text.starts_with(marker))
}

/// Blocks the tools inject into a message (time stamps, rules, file lists, reminders); dropped whole.
const INJECTED: &[&str] = &[
    "timestamp", "system-reminder", "system_reminder", "environment_context", "user_info", "user_instructions", "rules", "user_rules",
    "project_layout", "git_status", "attached_files", "open_and_recently_viewed_files", "recently_viewed_files", "additional_data",
    "agent_requestable_workspace_rules", "cursor_commands", "mode_specific_instructions", "command-name", "command-message",
    "command-args", "local-command-stdout", "local-command-stderr", "local-command-caveat", "available_skills", "INSTRUCTIONS",
];
/// Tags wrapped around the user's own words; only the words are kept.
const WRAPPERS: &[&str] = &["user_query", "user_request", "user_message", "pasted_content", "query"];

/// The message as the person wrote it, without the tags tools add. Empty when the whole message
/// was generated (Cursor's task notifications).
fn clean(text: &str) -> String {
    static PATTERNS: LazyLock<(regex::Regex, regex::Regex)> = LazyLock::new(|| {
        let injected = INJECTED.iter().map(|tag| regex::escape(tag)).collect::<Vec<_>>().join("|");
        let wrappers = WRAPPERS.iter().map(|tag| regex::escape(tag)).collect::<Vec<_>>().join("|");
        (
            regex::Regex::new(&format!(r"(?s)<({injected})\b[^>]*>.*?</({injected})>")).expect("valid pattern"),
            regex::Regex::new(&format!(r"</?({wrappers})\b[^>]*>")).expect("valid pattern"),
        )
    });
    if text.contains("<system_notification>") {
        return String::new();
    }
    let (injected, wrappers) = &*PATTERNS;
    let text = injected.replace_all(text, "");
    wrappers.replace_all(&text, "").trim().to_string()
}

impl Convo {
    fn push(&mut self, user: bool, text: &str) {
        let text = &clean(text);
        if is_noise(text) {
            return;
        }
        match self.messages.last_mut() {
            Some((last, previous)) if *last == user => {
                previous.push_str("\n\n");
                previous.push_str(text.trim());
            }
            _ => self.messages.push((user, text.trim().to_string())),
        }
        if self.title.is_empty() && user {
            self.title = title_from(text);
        }
    }
}

/// A JSON-lines file, read as it goes so a listing can stop early.
fn lines(path: &Path) -> impl Iterator<Item = Value> {
    let reader = fs::File::open(path).ok().map(std::io::BufReader::new);
    reader.into_iter().flat_map(std::io::BufRead::lines).map_while(Result::ok).filter_map(|line| serde_json::from_str::<Value>(&line).ok())
}

/// ~/.claude/projects/<folder>/<session>.jsonl. The file name is the session `claude --resume` takes.
/// With `quick`, reading stops once the title and folder are known: enough for a listing.
fn read_claude(path: &Path, quick: bool) -> Convo {
    let mut convo = Convo { upstream: path.file_stem().map(|stem| stem.to_string_lossy().into_owned()), ..Convo::default() };
    let mut summary = None;
    for value in lines(path) {
        match value["type"].as_str() {
            Some("summary") => summary = value["summary"].as_str().map(String::from),
            Some(kind @ ("user" | "assistant")) if value["isMeta"] != true && value["isSidechain"] != true => {
                if convo.cwd.is_empty() {
                    convo.cwd = value["cwd"].as_str().unwrap_or_default().into();
                }
                convo.push(kind == "user", &content_text(&value["message"]["content"]));
            }
            _ => {}
        }
        if quick && !convo.title.is_empty() && !convo.cwd.is_empty() {
            break;
        }
    }
    if let Some(summary) = summary {
        convo.title = summary;
    }
    convo
}

/// ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl, shared by the Codex CLI, desktop app and IDE extension.
fn read_codex(path: &Path, quick: bool) -> Convo {
    let mut convo = Convo::default();
    for value in lines(path) {
        let payload = &value["payload"];
        match value["type"].as_str() {
            Some("session_meta") => {
                convo.upstream = payload["id"].as_str().map(String::from);
                convo.cwd = payload["cwd"].as_str().unwrap_or_default().into();
            }
            Some("response_item") if payload["type"] == "message" => {
                let role = payload["role"].as_str().unwrap_or_default();
                if role == "user" || role == "assistant" {
                    convo.push(role == "user", &content_text(&payload["content"]));
                }
            }
            _ => {}
        }
        if quick && !convo.title.is_empty() && !convo.cwd.is_empty() {
            break;
        }
    }
    convo
}

/// ~/.gemini/tmp/<project hash>/chats/session-*.json. Gemini CLI cannot resume headless.
fn read_gemini(path: &Path, _quick: bool) -> Convo {
    let mut convo = Convo::default();
    let value: Value = fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    for message in value["messages"].as_array().into_iter().flatten() {
        let text = content_text(&message["content"]);
        match message["type"].as_str() {
            Some("user") => convo.push(true, &text),
            Some("gemini" | "model" | "assistant") => convo.push(false, &text),
            _ => {}
        }
    }
    convo
}

/// VS Code Copilot Chat: workspaceStorage/<hash>/chatSessions/<id>.json.
fn read_copilot(path: &Path, _quick: bool) -> Convo {
    let mut convo = Convo::default();
    let value: Value = fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    for request in value["requests"].as_array().into_iter().flatten() {
        convo.push(true, request["message"]["text"].as_str().unwrap_or_default());
        let reply: Vec<&str> = request["response"].as_array().into_iter().flatten().filter_map(|part| part["value"].as_str()).collect();
        convo.push(false, &reply.join(""));
    }
    if let Some(title) = value["customTitle"].as_str() {
        convo.title = title.into();
    }
    convo.cwd = path.parent().and_then(Path::parent).map(workspace_folder).unwrap_or_default();
    convo
}

/// "file:///d%3A/work/app" → "d:/work/app".
fn file_url_path(url: &str) -> String {
    let raw = url.strip_prefix("file://").unwrap_or(url);
    let mut bytes = Vec::new();
    let mut chars = raw.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let hex: String = chars.by_ref().take(2).map(char::from).collect();
            bytes.push(u8::from_str_radix(&hex, 16).unwrap_or(b'?'));
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8_lossy(&bytes).into_owned();
    // Windows drive paths come as /d:/…
    if path.len() > 2 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b':' { path[1..].to_string() } else { path }
}

/// The folder a VS Code / Cursor workspace storage entry belongs to.
fn workspace_folder(storage: &Path) -> String {
    fs::read(storage.join("workspace.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value["folder"].as_str().map(file_url_path))
        .unwrap_or_default()
}

fn open_readonly(path: &Path) -> Option<rusqlite::Connection> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = rusqlite::Connection::open_with_flags(path, flags).ok()?;
    let _ = connection.busy_timeout(Duration::from_secs(2));
    Some(connection)
}

/// Cursor's composer chats, keyed by composer id, with the folder each was opened in.
fn cursor_chats(since: u64) -> Vec<(String, Chat)> {
    let user = app_dir("Cursor").join("User");
    let mut folders: HashMap<String, String> = HashMap::new();
    for storage in fs::read_dir(user.join("workspaceStorage")).into_iter().flatten().flatten() {
        let folder = workspace_folder(&storage.path());
        let Some(db) = open_readonly(&storage.path().join("state.vscdb")) else { continue };
        let data: Option<String> = db.query_row("SELECT value FROM ItemTable WHERE key = 'composer.composerData'", [], |row| row.get(0)).ok();
        let ids = data.and_then(|text| serde_json::from_str::<Value>(&text).ok()).map(|value| value["allComposers"].clone()).unwrap_or_default();
        for id in ids.as_array().into_iter().flatten().filter_map(|composer| composer["composerId"].as_str()) {
            folders.insert(id.into(), folder.clone());
        }
    }
    let Some(db) = open_readonly(&user.join("globalStorage").join("state.vscdb")) else { return Vec::new() };
    // Cursor 2 lists chats in composerHeaders, with the folder each belongs to.
    if let Ok(mut query) = db.prepare("SELECT composerId, value FROM composerHeaders WHERE isSubagent IS NOT 1") {
        let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)));
        let mut chats = Vec::new();
        for (id, value) in rows.into_iter().flatten().flatten() {
            let Some(value) = value.and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
                chats.extend((since == 0).then(|| broken_cursor(&id)));
                continue;
            };
            let updated = value["lastUpdatedAt"].as_u64().or(value["createdAt"].as_u64()).unwrap_or(0);
            if updated < since || value["isDraft"] == true || value["isArchived"] == true {
                continue;
            }
            let uri = &value["workspaceIdentifier"]["uri"];
            let cwd = uri["fsPath"].as_str().map(String::from).or_else(|| uri["external"].as_str().map(file_url_path)).or_else(|| folders.get(&id).cloned()).unwrap_or_default();
            let title = value["name"].as_str().filter(|name| !name.is_empty()).unwrap_or("Cursor chat").to_string();
            chats.push((id.clone(), Chat { key: format!("cursor:{id}"), source: "cursor", cwd, title, updated, messages: 0, imported: false, unreadable: false, reason: None }));
        }
        if !chats.is_empty() {
            return chats;
        }
    }
    let Ok(mut query) = db.prepare("SELECT key, value FROM cursorDiskKV WHERE key >= 'composerData:' AND key < 'composerData;'") else { return Vec::new() };
    let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)));
    let mut chats = Vec::new();
    for (key, value) in rows.into_iter().flatten().flatten() {
        let id = key.trim_start_matches("composerData:").to_string();
        let Some(value) = value.and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
            chats.extend((since == 0).then(|| broken_cursor(&id)));
            continue;
        };
        let updated = value["lastUpdatedAt"].as_u64().or(value["createdAt"].as_u64()).unwrap_or(0);
        if updated < since || value["isDraft"] == true {
            continue;
        }
        let messages = value["fullConversationHeadersOnly"].as_array().map_or(0, Vec::len);
        let title = value["name"].as_str().map(String::from).unwrap_or_else(|| title_from(value["text"].as_str().unwrap_or("Cursor chat")));
        chats.push((id.clone(), Chat { key: format!("cursor:{id}"), source: "cursor", cwd: folders.get(&id).cloned().unwrap_or_default(), title, updated, messages, imported: false, unreadable: false, reason: None }));
    }
    chats
}

/// A Cursor chat whose stored record is not valid JSON; it has no date, so only "All time" lists it.
fn broken_cursor(id: &str) -> (String, Chat) {
    let chat = Chat { key: format!("cursor:{id}"), source: "cursor", cwd: String::new(), title: "Cursor chat".into(), updated: 0, messages: 0, imported: false, unreadable: true, reason: Some("Chat record could not be parsed".into()) };
    (id.into(), chat)
}

fn read_cursor(db: &Path, id: &str, cwd: &str) -> Convo {
    let mut convo = Convo { cwd: cwd.into(), ..Convo::default() };
    let Some(db) = open_readonly(db) else { return convo };
    let (low, high) = (format!("bubbleId:{id}:"), format!("bubbleId:{id};"));
    let Ok(mut query) = db.prepare("SELECT value FROM cursorDiskKV WHERE key >= ?1 AND key < ?2") else { return convo };
    let mut bubbles: Vec<Value> = query
        .query_map([&low, &high], |row| row.get::<_, Option<String>>(0))
        .into_iter()
        .flatten()
        .flatten()
        .flatten()
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect();
    bubbles.sort_by(|a, b| a["createdAt"].as_str().unwrap_or_default().cmp(b["createdAt"].as_str().unwrap_or_default()));
    for bubble in bubbles {
        let text = bubble["text"].as_str().unwrap_or_default();
        match bubble["type"].as_i64().or_else(|| bubble["type"].as_str().and_then(|kind| kind.parse().ok())) {
            Some(1) => convo.push(true, text),
            Some(2) => convo.push(false, text),
            _ => {}
        }
    }
    convo
}

/// OpenCode keeps sessions in its own database; only their ids, folders and titles are read here.
/// Messages come from `opencode export`, its supported way out.
fn opencode_chats(since: u64) -> Vec<(String, Chat)> {
    let db = dirs::data_local_dir()
        .filter(|_| cfg!(not(windows)))
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("opencode")
        .join("opencode.db");
    let Some(db) = open_readonly(&db) else { return Vec::new() };
    let mut chats = Vec::new();
    for table in ["session_v2", "session"] {
        let sql = format!("SELECT id, directory, title, time_updated FROM {table} WHERE parent_id IS NULL");
        let Ok(mut query) = db.prepare(&sql) else { continue };
        let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)));
        for (id, cwd, title, updated) in rows.into_iter().flatten().flatten() {
            let updated = updated.max(0) as u64;
            if updated >= since && !chats.iter().any(|(known, _): &(String, Chat)| *known == id) {
                chats.push((id.clone(), Chat { key: format!("opencode:{id}"), source: "opencode", cwd, title, updated, messages: 0, imported: false, unreadable: false, reason: None }));
            }
        }
    }
    chats
}

fn read_opencode(id: &str) -> Convo {
    let mut convo = Convo { upstream: Some(id.into()), ..Convo::default() };
    let Some(program) = crate::team_agents::program(crate::team_agents::Kind::OpenCode) else { return convo };
    // OpenCode 2 moved export under `session`; 1.x has it at the top level.
    let value = [&["session", "export", id][..], &["export", id][..]]
        .iter()
        .find_map(|args| {
            let mut command = crate::cli_setup::command(&program);
            command.args(*args);
            let text = crate::cli_setup::output_within(command, Duration::from_secs(30))?;
            serde_json::from_str::<Value>(&text[text.find('{')?..]).ok().filter(|value| value["messages"].is_array())
        })
        .unwrap_or_default();
    opencode_convo(&mut convo, &value);
    convo
}

/// Both export shapes: 2.x `{type, text | content: [parts]}`, 1.x `{info: {role}, parts}`.
fn opencode_convo(convo: &mut Convo, value: &Value) {
    let info = &value["info"];
    convo.cwd = info["location"]["directory"].as_str().or(info["directory"].as_str()).unwrap_or_default().into();
    convo.title = info["title"].as_str().unwrap_or_default().into();
    for message in value["messages"].as_array().into_iter().flatten() {
        let role = message["type"].as_str().or(message["info"]["role"].as_str()).unwrap_or_default();
        let parts = message["content"].as_array().or(message["parts"].as_array());
        let text = match message["text"].as_str() {
            Some(text) => text.to_string(),
            None => parts.into_iter().flatten().filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
        };
        match role {
            "user" => convo.push(true, &text),
            "assistant" => convo.push(false, &text),
            _ => {}
        }
    }
}

/// Why a session file gave no messages: it cannot be read or parsed, or nothing in it is a message.
fn unreadable_reason(path: &Path) -> String {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => return format!("File could not be read: {e}"),
    };
    let error = if ext(path, "jsonl") {
        let results: Vec<_> = text.lines().filter(|line| !line.trim().is_empty()).map(serde_json::from_str::<Value>).collect();
        if results.iter().any(Result::is_ok) { None } else { results.into_iter().find_map(Result::err) }
    } else {
        serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).err()
    };
    error.map_or_else(|| "No messages could be read".into(), |e| format!("File could not be parsed: {e}"))
}

/// One session file as a listed chat, flagged unreadable when no message could be read from it.
fn list_file(source: &'static str, path: &Path, since: u64, read: fn(&Path, bool) -> Convo) -> Option<Chat> {
    let updated = modified(path);
    if updated < since {
        return None;
    }
    let convo = read(path, true);
    // Sessions Team itself started are already in their task.
    if convo.title.starts_with("You are @") && convo.title.contains("on a Neru team") {
        return None;
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let id = convo.upstream.clone().unwrap_or_else(|| stem.clone());
    let reason = convo.messages.is_empty().then(|| unreadable_reason(path));
    let title = if convo.title.is_empty() { stem } else { convo.title };
    Some(Chat { key: format!("{source}:{id}"), source, cwd: convo.cwd, title, updated, messages: 0, imported: false, unreadable: reason.is_some(), reason })
}

/// Every conversation newer than `since` (ms), newest first.
fn find_chats(since: u64) -> Vec<(Located, Chat)> {
    let mut found: Vec<(Located, Chat)> = Vec::new();
    let mut add = |source: &'static str, path: PathBuf, read: fn(&Path, bool) -> Convo| {
        if let Some(chat) = list_file(source, &path, since, read) {
            found.push((Located::File(path), chat));
        }
    };
    for path in files(&home().join(".claude").join("projects"), 1, &|path| ext(path, "jsonl")) {
        add("claude", path, read_claude);
    }
    for path in files(&home().join(".codex").join("sessions"), 4, &|path| ext(path, "jsonl")) {
        add("codex", path, read_codex);
    }
    for path in files(&home().join(".gemini").join("tmp"), 2, &|path| ext(path, "json") && path.parent().is_some_and(|dir| dir.ends_with("chats"))) {
        add("gemini", path, read_gemini);
    }
    for code in ["Code", "Code - Insiders"] {
        for path in files(&app_dir(code).join("User").join("workspaceStorage"), 2, &|path| ext(path, "json") && path.parent().is_some_and(|dir| dir.ends_with("chatSessions"))) {
            add("copilot", path, read_copilot);
        }
    }
    let cursor_db = app_dir("Cursor").join("User").join("globalStorage").join("state.vscdb");
    for (_, chat) in cursor_chats(since) {
        found.push((Located::Cursor(cursor_db.clone()), chat));
    }
    for (id, chat) in opencode_chats(since) {
        found.push((Located::OpenCode(id), chat));
    }
    found.sort_by(|a, b| b.1.updated.cmp(&a.1.updated));
    // A resumed Codex session writes a new log with the same id; list it once, newest log first.
    let mut keys = std::collections::HashSet::new();
    found.retain(|(_, chat)| keys.insert(chat.key.clone()));
    found
}

fn skill_dirs(root: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let home = home();
    let mut dirs = vec![
        ("claude", home.join(".claude").join("skills")),
        ("codex", home.join(".codex").join("skills")),
        ("shared", home.join(".agents").join("skills")),
        ("cursor", home.join(".cursor").join("skills")),
        ("gemini", home.join(".gemini").join("skills")),
        ("opencode", home.join(".config").join("opencode").join("skill")),
        ("opencode", home.join(".config").join("opencode").join("skills")),
    ];
    if let Some(root) = root {
        dirs.push(("codex", root.join(".codex").join("skills")));
        dirs.push(("cursor", root.join(".cursor").join("skills")));
        dirs.push(("shared", root.join(".agents").join("skills")));
    }
    dirs
}

fn find_skills(root: Option<&Path>) -> Vec<FoundSkill> {
    let personal = data_dir().map(|dir| dir.join("skills")).unwrap_or_default();
    let mut found: Vec<FoundSkill> = Vec::new();
    for (source, dir) in skill_dirs(root) {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let folder = entry.path();
            let file = folder.join("SKILL.md");
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            let Ok(text) = fs::read_to_string(&file) else { continue };
            if hidden {
                continue;
            }
            let name = crate::skills::front_name(&text).unwrap_or_else(|| entry.file_name().to_string_lossy().to_lowercase());
            let front = text.trim_start_matches('\u{feff}').strip_prefix("---").and_then(|rest| rest.find("\n---").map(|end| rest[..end].to_string())).unwrap_or_default();
            let description = crate::skills::front_value(&front, "description").unwrap_or_default();
            if found.iter().any(|skill| skill.path == folder.to_string_lossy()) {
                continue;
            }
            found.push(FoundSkill { source, conflict: personal.join(&name).is_dir(), name, path: folder.to_string_lossy().into_owned(), description: clip(&description, 200) });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(b.source)));
    found
}

/// Codex's `[mcp_servers.<name>]` tables from config.toml, read just far enough: strings, string
/// arrays and inline string tables.
fn codex_servers(text: &str) -> Value {
    let mut servers = serde_json::Map::new();
    let mut current: Option<String> = None;
    let string = |raw: &str| raw.trim().trim_matches('"').trim_matches('\'').to_string();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current = line.trim_matches(['[', ']']).strip_prefix("mcp_servers.").map(|name| name.trim_matches('"').to_string());
            if let Some(name) = &current {
                if let Some((name, table)) = name.split_once('.') {
                    // [mcp_servers.x.env]
                    current = Some(format!("{name}\n{table}"));
                } else {
                    servers.insert(name.clone(), json!({}));
                }
            }
            continue;
        }
        let (Some(name), Some((key, value))) = (&current, line.split_once('=')) else { continue };
        let (server, table) = name.split_once('\n').map_or((name.as_str(), None), |(server, table)| (server, Some(table)));
        let Some(entry) = servers.get_mut(server) else { continue };
        let key = key.trim().trim_matches('"').to_string();
        let value = value.trim();
        let parsed = if let Some(inner) = value.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            json!(inner.split(',').map(string).filter(|item| !item.is_empty()).collect::<Vec<_>>())
        } else if let Some(inner) = value.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')) {
            let map: serde_json::Map<String, Value> = inner.split(',').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (string(k), json!(string(v)))).collect();
            Value::Object(map)
        } else {
            json!(string(value))
        };
        match table {
            Some(table) => {
                entry[table][key] = parsed;
            }
            None => {
                let key = if key == "http_headers" { "headers".to_string() } else { key };
                entry[key] = parsed;
            }
        }
    }
    json!({ "mcpServers": servers })
}

/// OpenCode's `"mcp": {name: {type: "local", command: [...], environment} | {type: "remote", url, headers}}`.
fn opencode_servers(value: &Value) -> Value {
    let mut servers = serde_json::Map::new();
    for (name, entry) in value["mcp"].as_object().into_iter().flatten() {
        let command: Vec<&str> = entry["command"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let converted = if entry["type"] == "remote" {
            json!({ "type": "http", "url": entry["url"], "headers": entry["headers"], "disabled": entry["enabled"] == false })
        } else {
            json!({ "command": command.first(), "args": command.get(1..).unwrap_or_default(), "env": entry["environment"], "disabled": entry["enabled"] == false })
        };
        servers.insert(name.clone(), converted);
    }
    json!({ "mcpServers": servers })
}

fn read_json(path: &Path) -> Value {
    fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

fn find_servers() -> Vec<(&'static str, mcp::ServerConfig)> {
    let home = home();
    let mut configs: Vec<(&'static str, Value)> = vec![
        ("claude", read_json(&home.join(".claude.json"))),
        ("claude-desktop", read_json(&app_dir("Claude").join("claude_desktop_config.json"))),
        ("cursor", read_json(&home.join(".cursor").join("mcp.json"))),
        ("gemini", read_json(&home.join(".gemini").join("settings.json"))),
        ("opencode", opencode_servers(&read_json(&home.join(".config").join("opencode").join("opencode.json")))),
        ("codex", codex_servers(&fs::read_to_string(home.join(".codex").join("config.toml")).unwrap_or_default())),
    ];
    for code in ["Code", "Code - Insiders"] {
        let value = read_json(&app_dir(code).join("User").join("mcp.json"));
        configs.push(("vscode", json!({ "mcpServers": value["servers"] })));
    }
    let mut found = Vec::new();
    for (source, mut value) in configs {
        // Gemini names streamable HTTP endpoints httpUrl.
        if let Some(servers) = value["mcpServers"].as_object_mut() {
            for entry in servers.values_mut() {
                if entry.get("url").is_none() {
                    if let Some(url) = entry.get("httpUrl").cloned() {
                        entry["url"] = url;
                    }
                }
            }
        }
        for server in mcp::parse_project_config(&value) {
            found.push((source, server.config));
        }
    }
    found
}

fn find_rules(root: Option<&Path>) -> Vec<(&'static str, PathBuf, &'static str)> {
    let home = home();
    let mut rules = vec![
        ("codex", home.join(".codex").join("AGENTS.md"), "personal"),
        ("gemini", home.join(".gemini").join("GEMINI.md"), "personal"),
        ("opencode", home.join(".config").join("opencode").join("AGENTS.md"), "personal"),
    ];
    if let Some(root) = root {
        rules.push(("cursor", root.join(".cursorrules"), "project"));
        for file in files(&root.join(".cursor").join("rules"), 2, &|path| ext(path, "mdc") || ext(path, "md")) {
            rules.push(("cursor", file, "project"));
        }
        rules.push(("gemini", root.join("GEMINI.md"), "project"));
        rules.push(("copilot", root.join(".github").join("copilot-instructions.md"), "project"));
    }
    rules.retain(|(_, path, _)| fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0));
    rules
}

fn sources(scan: &Scan) -> Vec<Source> {
    let home = home();
    let count = |source: &str| -> (usize, usize, usize, usize) {
        (
            scan.chats.iter().filter(|chat| chat.source == source).count(),
            scan.skills.iter().filter(|skill| skill.source == source).count(),
            scan.servers.iter().filter(|server| server.source == source).count(),
            scan.rules.iter().filter(|rules| rules.source == source).count(),
        )
    };
    let extensions: Vec<String> = fs::read_dir(home.join(".vscode").join("extensions"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
        .collect();
    let has_extension = |prefix: &str| extensions.iter().any(|name| name.starts_with(prefix));
    let mut vscode_note: Vec<&str> = Vec::new();
    for (prefix, name) in [("anthropic.claude-code", "Claude Code"), ("openai.chatgpt", "Codex"), ("github.copilot-chat", "Copilot Chat")] {
        if has_extension(prefix) {
            vscode_note.push(name);
        }
    }
    let entry = |id: &'static str, name: &'static str, kind: &'static str, path: PathBuf, counts: (usize, usize, usize, usize), note: Option<String>| Source {
        id,
        name,
        kind,
        found: path.exists(),
        path: path.to_string_lossy().into_owned(),
        chats: counts.0,
        skills: counts.1,
        servers: counts.2,
        rules: counts.3,
        note,
    };
    vec![
        entry("claude", "Claude Code", "agent", home.join(".claude"), count("claude"), Some("CLI and IDE extension share this history.".into())),
        entry("claude-desktop", "Claude app", "app", app_dir("Claude"), count("claude-desktop"), Some("Chats live in your Claude account; its MCP servers come over.".into())),
        entry("codex", "Codex", "agent", home.join(".codex"), count("codex"), Some("CLI, desktop app and IDE extension share this history.".into())),
        entry("cursor", "Cursor", "editor", app_dir("Cursor"), count("cursor"), None),
        entry("vscode", "VS Code", "editor", app_dir("Code"), (count("copilot").0, 0, count("vscode").2, count("copilot").3), (!vscode_note.is_empty()).then(|| format!("Extensions: {}.", vscode_note.join(", ")))),
        entry("opencode", "OpenCode", "agent", home.join(".config").join("opencode"), count("opencode"), None),
        entry("gemini", "Gemini CLI", "agent", home.join(".gemini"), count("gemini"), None),
        entry("shared", "Shared skills", "agent", home.join(".agents"), count("shared"), Some("~/.agents/skills, read by several agents.".into())),
    ]
}

fn scan(since_days: u64, root: Option<PathBuf>) -> Scan {
    let since = if since_days == 0 { 0 } else { now().saturating_sub(since_days * 86_400_000) };
    let ledger = ledger();
    let chats = find_chats(since);
    let known: HashSet<String> = mcp::read_config().into_iter().map(|server| server.name).collect();
    let servers = find_servers();
    let mut found_servers = Vec::new();
    {
        let mut cache = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        cache.clear();
        for (source, config) in servers {
            let key = format!("{source}:{}", config.name);
            if cache.contains_key(&key) {
                continue;
            }
            found_servers.push(FoundServer {
                key: key.clone(),
                source,
                name: config.name.clone(),
                target: config.url.clone().unwrap_or_else(|| format!("{} {}", config.command, config.args.join(" ")).trim().to_string()),
                conflict: known.contains(&config.name),
            });
            cache.insert(key, config);
        }
    }
    let mut result = Scan {
        sources: Vec::new(),
        chats: Vec::new(),
        skills: find_skills(root.as_deref()),
        servers: found_servers,
        rules: find_rules(root.as_deref())
            .into_iter()
            .map(|(source, path, scope)| FoundRules {
                source,
                size: fs::metadata(&path).map_or(0, |meta| meta.len()),
                imported: ledger.rules.contains(&path.to_string_lossy().into_owned()),
                path: path.to_string_lossy().into_owned(),
                scope,
            })
            .collect(),
    };
    let mut cache = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    cache.clear();
    for (located, mut chat) in chats {
        chat.imported = ledger.chats.contains(&chat.key);
        cache.insert(chat.key.clone(), (located, chat.clone()));
        result.chats.push(chat);
    }
    result.sources = sources(&result);
    result
}

fn read(located: &Located, chat: &Chat) -> Convo {
    let id = chat.key.split_once(':').map_or("", |(_, id)| id);
    let mut convo = match located {
        Located::File(path) => match chat.source {
            "claude" => read_claude(path, false),
            "codex" => read_codex(path, false),
            "gemini" => read_gemini(path, false),
            _ => read_copilot(path, false),
        },
        Located::OpenCode(id) => read_opencode(id),
        Located::Cursor(db) => read_cursor(db, id, &chat.cwd),
    };
    if convo.cwd.is_empty() {
        convo.cwd = chat.cwd.clone();
    }
    if convo.title.is_empty() {
        convo.title = chat.title.clone();
    }
    convo
}

/// Groups chats by folder into Team tasks. Returns the new task ids.
/// `progress(done, total, title)` is called as each chat is read.
fn import(keys: &[String], progress: &dyn Fn(usize, usize, &str)) -> Result<Vec<String>, String> {
    let found = FOUND.lock().map_err(|e| e.to_string())?.clone();
    let mut groups: Vec<(String, Vec<(Chat, Convo)>)> = Vec::new();
    for (done, key) in keys.iter().enumerate() {
        let Some((located, chat)) = found.get(key).filter(|(_, chat)| !chat.unreadable) else { continue };
        progress(done, keys.len(), &chat.title);
        let convo = read(located, chat);
        if convo.messages.is_empty() {
            continue;
        }
        let folder = convo.cwd.replace('\\', "/").trim_end_matches('/').to_string();
        let group = folder.to_lowercase();
        match groups.iter_mut().find(|(known, _)| known.to_lowercase() == group) {
            Some((_, list)) => list.push((chat.clone(), convo)),
            None => groups.push((folder, vec![(chat.clone(), convo)])),
        }
    }
    if groups.is_empty() {
        return Err("Nothing to import. Scan again and pick some chats.".into());
    }
    let mut ledger = ledger();
    let mut created = Vec::new();
    for (folder, mut list) in groups {
        list.sort_by_key(|(chat, _)| chat.updated);
        let name = Path::new(&folder).file_name().map(|name| name.to_string_lossy().into_owned()).filter(|name| !name.is_empty());
        let project = if !folder.is_empty() && Path::new(&folder).is_dir() { folder.clone() } else { String::new() };
        let title = match &name {
            Some(name) => format!("{name} · imported"),
            None => "Imported chats".to_string(),
        };
        let mut task = team::create(&title, &project, Vec::new(), Vec::new())?;
        task.labels.push("Imported".into());
        for (chat, convo) in list {
            let label = match chat.source {
                "claude" => "Claude Code",
                "codex" => "Codex",
                "opencode" => "OpenCode",
                "gemini" => "Gemini CLI",
                "cursor" => "Cursor",
                _ => "VS Code Copilot",
            };
            let date = chrono_date(chat.updated);
            task.posts.push(team::Post::notice(format!("Imported from {label}: “{}” · {date}", convo.title)));
            // Copilot cannot be driven from outside VS Code, so its replies stay as history only.
            let author = if chat.source == "copilot" { "copilot".to_string() } else { team::adopt(&mut task, chat.source, convo.upstream.clone()) };
            for (user, text) in &convo.messages {
                task.posts.push(team::Post::imported(if *user { "you" } else { &author }, clip(text, MAX_POST)));
            }
            let seen = task.posts.len();
            if let Some(member) = task.members.iter_mut().find(|member| member.handle == author) {
                member.seen = seen;
            }
            ledger.chats.insert(chat.key.clone());
        }
        task.updated_at = now();
        created.push(task.id.clone());
        team::store(task)?;
    }
    progress(keys.len(), keys.len(), "");
    save_ledger(&ledger)?;
    Ok(created)
}

/// "2 Oct 2026" without a date library.
fn chrono_date(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

/// Looks through the machine for other agents' chats, skills, MCP servers and instructions.
#[tauri::command]
pub async fn scan_imports(since_days: Option<u64>, app: AppHandle) -> Result<Scan, String> {
    let root = app.state::<AppState>().root.lock().ok().and_then(|root| root.clone());
    tokio::task::spawn_blocking(move || scan(since_days.unwrap_or(30), root)).await.map_err(|e| e.to_string())
}

/// Imports the chats off the UI thread, reporting `imports://progress` {done, total, title} as it reads them.
#[tauri::command]
pub async fn import_chats(keys: Vec<String>, app: AppHandle) -> Result<Vec<String>, String> {
    let emit = move |done: usize, total: usize, title: &str| {
        let _ = app.emit("imports://progress", json!({ "done": done, "total": total, "title": title }));
    };
    tokio::task::spawn_blocking(move || import(&keys, &emit)).await.map_err(|e| e.to_string())?
}

/// Adds found MCP servers to Neru's connectors and starts them.
#[tauri::command]
pub async fn import_servers(keys: Vec<String>, app: AppHandle) -> Result<usize, String> {
    let chosen: Vec<mcp::ServerConfig> = {
        let cache = SERVERS.lock().map_err(|e| e.to_string())?;
        keys.iter().filter_map(|key| cache.get(key).cloned()).collect()
    };
    let mut servers = mcp::read_config();
    for server in &chosen {
        servers.retain(|known| known.name != server.name);
        servers.push(server.clone());
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    mcp::write_config(&servers)?;
    let state = app.state::<AppState>();
    for server in chosen.iter().filter(|server| server.enabled) {
        state.mcp.start(server).await;
    }
    Ok(chosen.len())
}

/// Appends other agents' instruction files to Neru's: personal ones to the data folder's
/// AGENTS.md (read in every project), project ones to the project's `.neru/instructions.md`.
#[tauri::command]
pub fn import_rules(paths: Vec<String>, app: AppHandle) -> Result<usize, String> {
    let root = app.state::<AppState>().root.lock().ok().and_then(|root| root.clone());
    let found = find_rules(root.as_deref());
    let mut ledger = ledger();
    let mut count = 0;
    for path in paths {
        let Some((source, file, scope)) = found.iter().find(|(_, file, _)| file.to_string_lossy() == path) else { continue };
        let target = match (*scope, &root) {
            ("project", Some(root)) => root.join(".neru").join("instructions.md"),
            _ => data_dir()?.join("AGENTS.md"),
        };
        let marker = format!("<!-- Imported from {} -->", file.display());
        let existing = fs::read_to_string(&target).unwrap_or_default();
        if !existing.contains(&marker) {
            let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let body = if ext(file, "mdc") { strip_front(&text) } else { text };
            fs::create_dir_all(target.parent().ok_or("Invalid folder")?).map_err(|e| e.to_string())?;
            let joined = format!("{}{}{marker}\n## From {source}\n\n{}\n", existing, if existing.is_empty() || existing.ends_with("\n\n") { "" } else { "\n\n" }, body.trim());
            fs::write(&target, joined).map_err(|e| e.to_string())?;
            count += 1;
        }
        ledger.rules.insert(path);
    }
    save_ledger(&ledger)?;
    Ok(count)
}

/// Cursor `.mdc` rules start with front matter (description, globs) that is not instruction text.
fn strip_front(text: &str) -> String {
    text.trim_start_matches('\u{feff}')
        .strip_prefix("---")
        .and_then(|rest| rest.find("\n---").map(|end| rest[end + 4..].to_string()))
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("neru-import-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn claude_transcripts_skip_tool_noise_and_keep_the_session() {
        let dir = temp("claude");
        let file = dir.join("df94f3b5-e665-4ace-8e5f-bca4f248c9fb.jsonl");
        fs::write(
            &file,
            [
                r#"{"type":"user","cwd":"D:\\work\\app","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
                r#"{"type":"user","cwd":"D:\\work\\app","message":{"role":"user","content":"Fix the login bug"}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking."}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"file"}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed it."}]}}"#,
                r#"{"type":"summary","summary":"Login bug fix"}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let convo = read_claude(&file, false);
        assert_eq!(convo.upstream.as_deref(), Some("df94f3b5-e665-4ace-8e5f-bca4f248c9fb"));
        assert_eq!(convo.cwd, "D:\\work\\app");
        assert_eq!(convo.title, "Login bug fix");
        assert_eq!(convo.messages, vec![(true, "Fix the login bug".to_string()), (false, "Looking.\n\nFixed it.".to_string())]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_sessions_are_listed_with_a_reason() {
        let dir = temp("unreadable");
        let garbage = dir.join("0b1d.jsonl");
        fs::write(&garbage, "not json\n{broken").unwrap();
        let chat = list_file("claude", &garbage, 0, read_claude).unwrap();
        assert!(chat.unreadable);
        assert!(chat.reason.as_deref().unwrap().starts_with("File could not be parsed"), "{:?}", chat.reason);
        assert_eq!((chat.key.as_str(), chat.title.as_str()), ("claude:0b1d", "0b1d"));
        let empty = dir.join("meta.jsonl");
        fs::write(&empty, r#"{"type":"summary","summary":"x"}"#).unwrap();
        assert_eq!(list_file("claude", &empty, 0, read_claude).unwrap().reason.as_deref(), Some("No messages could be read"));
        let good = dir.join("ok.jsonl");
        fs::write(&good, r#"{"type":"user","cwd":"D:\\w","message":{"content":"Hi"}}"#).unwrap();
        assert!(!list_file("claude", &good, 0, read_claude).unwrap().unreadable);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_rollouts_give_thread_cwd_and_messages() {
        let dir = temp("codex");
        let file = dir.join("rollout-2026-07-18T23-12-04-019f.jsonl");
        fs::write(
            &file,
            [
                r#"{"type":"session_meta","payload":{"id":"019f","cwd":"F:\\Epos"}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add tests"}]}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Added."}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let convo = read_codex(&file, false);
        assert_eq!((convo.upstream.as_deref(), convo.cwd.as_str(), convo.title.as_str()), (Some("019f"), "F:\\Epos", "Add tests"));
        assert_eq!(convo.messages.len(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_config_servers_are_read() {
        let toml = "model = \"o3\"\n[mcp_servers.github]\ncommand = \"npx\"\nargs = [\"-y\", \"@modelcontextprotocol/server-github\"]\nenv = { GITHUB_TOKEN = \"t\" }\n[mcp_servers.docs]\nurl = \"https://docs.example/mcp\"\n[mcp_servers.docs.http_headers]\nX-Key = \"k\"\n";
        let servers = mcp::parse_project_config(&codex_servers(toml));
        let github = servers.iter().find(|server| server.config.name == "github").unwrap();
        assert_eq!(github.config.command, "npx");
        assert_eq!(github.config.args, vec!["-y", "@modelcontextprotocol/server-github"]);
        assert_eq!(github.config.env.get("GITHUB_TOKEN").map(String::as_str), Some("t"));
        let docs = servers.iter().find(|server| server.config.name == "docs").unwrap();
        assert_eq!(docs.config.url.as_deref(), Some("https://docs.example/mcp"));
    }

    #[test]
    fn opencode_servers_are_converted() {
        let value = json!({"mcp": {"fs": {"type": "local", "command": ["npx", "fs-server", "/w"], "environment": {"A": "1"}}, "web": {"type": "remote", "url": "https://x.example/mcp"}}});
        let servers = mcp::parse_project_config(&opencode_servers(&value));
        assert_eq!(servers.len(), 2);
        let fs_server = servers.iter().find(|server| server.config.name == "fs").unwrap();
        assert_eq!((fs_server.config.command.as_str(), fs_server.config.args.len()), ("npx", 2));
    }

    #[test]
    fn opencode_exports_of_both_versions_are_read() {
        let v2 = json!({"info": {"title": "T", "location": {"directory": "D:\\w"}}, "messages": [
            {"type": "user", "text": "Hi"},
            {"type": "assistant", "content": [{"type": "reasoning", "text": "hmm"}, {"type": "text", "text": "Hello"}]},
            {"type": "idle"}
        ]});
        let mut convo = Convo::default();
        opencode_convo(&mut convo, &v2);
        assert_eq!((convo.cwd.as_str(), convo.messages.len()), ("D:\\w", 2));
        assert_eq!(convo.messages[1], (false, "Hello".to_string()));
        let v1 = json!({"info": {"directory": "/w"}, "messages": [{"info": {"role": "user"}, "parts": [{"type": "text", "text": "Yo"}]}]});
        let mut convo = Convo::default();
        opencode_convo(&mut convo, &v1);
        assert_eq!((convo.cwd.as_str(), convo.title.as_str()), ("/w", "Yo"));
    }

    #[test]
    fn injected_tags_are_removed_and_the_words_kept() {
        assert_eq!(clean("<timestamp>Friday, 4:16 PM</timestamp>\n<user_query>Fix the header</user_query>"), "Fix the header");
        assert_eq!(clean("<system_notification>\nThe following task has finished.\n<task>x</task>\n</system_notification>\n<user_query>Briefly inform the user</user_query>"), "");
        assert_eq!(clean("Look at this <system-reminder>\nnoise\n</system-reminder> please"), "Look at this  please");
        assert_eq!(clean("<command-name>/clear</command-name>\n<command-message>clear</command-message>"), "");
        // Markup the person wrote stays.
        assert_eq!(clean("Why does <div> collapse?"), "Why does <div> collapse?");
    }

    #[test]
    fn small_helpers_behave() {
        assert_eq!(title_from("<pasted_content id=\"1\">Fix the build</pasted_content>"), "Fix the build");
        assert_eq!(file_url_path("file:///d%3A/work/my%20app"), "d:/work/my app");
        assert_eq!(file_url_path("file:///home/me/app"), "/home/me/app");
        assert_eq!(chrono_date(1_790_956_969_667), "2 Oct 2026");
        assert_eq!(strip_front("---\ndescription: x\nglobs: *.ts\n---\nUse tabs."), "\nUse tabs.");
        assert!(is_noise("<environment_context>"));
        assert!(!is_noise("Fix it"));
    }
}
