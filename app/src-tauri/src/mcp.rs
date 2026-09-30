//! Model Context Protocol connectors over stdio. Each configured server is a local process that
//! speaks JSON-RPC 2.0, one message per line. Neru lists its tools, offers them to the model as
//! `mcp__<server>__<tool>`, and runs a call only after the user approves it (or always allows it).

use std::{
    collections::{HashMap, VecDeque},
    fs,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::oneshot,
};

use crate::{AppState, oauth, workspace::data_dir};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn enabled_default() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    /// Local servers: the program to start. Empty for hosted servers.
    #[serde(default)]
    pub command: String,
    /// Hosted servers: the Streamable HTTP endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Hosted servers: extra request headers, such as `Authorization: Bearer …`.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    #[serde(skip)]
    pub schema: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub config: ServerConfig,
    /// "connected", "starting", "error", "off", "needs_auth", or "needs_trust" for a project
    /// connector whose folder the user has not trusted yet (it is not started).
    pub status: String,
    /// True when a saved OAuth sign-in exists for this hosted connector.
    pub signed_in: bool,
    pub error: Option<String>,
    pub tools: Vec<ToolInfo>,
    /// "user" for connectors saved in Settings, "project" for the open project's `.mcp.json`.
    pub source: String,
}

/// A connector from the open project's `.mcp.json`. `legacy_sse` marks `"type": "sse"` entries,
/// which are tried as Streamable HTTP.
#[derive(Clone, PartialEq, Debug)]
pub struct ProjectServer {
    pub config: ServerConfig,
    pub legacy_sse: bool,
}

type Reply = oneshot::Sender<Result<Value, String>>;

struct Client {
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Mutex<HashMap<u64, Reply>>,
    next: AtomicU64,
    tools: Mutex<Vec<ToolInfo>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    alive: Arc<std::sync::atomic::AtomicBool>,
    _child: Mutex<Child>,
}

#[derive(Default)]
pub struct McpManager {
    clients: Mutex<HashMap<String, Connection>>,
    /// Hosted connectors that answered 401, with their WWW-Authenticate header.
    auth_needed: Mutex<HashMap<String, Option<String>>>,
    errors: Mutex<HashMap<String, String>>,
    starting: Mutex<Vec<String>>,
    /// Connectors from the open project's `.mcp.json`, as last synced; they win over saved ones
    /// with the same name and are never written to Neru's own `mcp.json`.
    project: Mutex<Vec<ProjectServer>>,
    /// Whether the user trusts the project those came from; untrusted ones are never started.
    project_trusted: std::sync::atomic::AtomicBool,
}

fn config_path() -> Result<std::path::PathBuf, String> {
    Ok(data_dir()?.join("mcp.json"))
}

/// Environment values often hold tokens, so they are stored encrypted with a `dpapi:` prefix.
const SEALED: &str = "dpapi:";

pub fn read_config() -> Vec<ServerConfig> {
    let mut servers: Vec<ServerConfig> = config_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| serde_json::from_value(value["servers"].clone()).ok())
        .unwrap_or_default();
    for server in &mut servers {
        for value in server.env.values_mut().chain(server.headers.values_mut()) {
            if let Some(sealed) = value.strip_prefix(SEALED) {
                *value = crate::settings::open_secret(sealed).unwrap_or_default();
            }
        }
    }
    servers
}

fn write_config(servers: &[ServerConfig]) -> Result<(), String> {
    let stored: Vec<ServerConfig> = servers
        .iter()
        .cloned()
        .map(|mut server| {
            for value in server.env.values_mut().chain(server.headers.values_mut()) {
                if let Some(sealed) = crate::settings::seal_secret(value) {
                    *value = format!("{SEALED}{sealed}");
                }
            }
            server
        })
        .collect();
    let text = serde_json::to_string_pretty(&json!({ "servers": stored })).map_err(|e| e.to_string())?;
    fs::write(config_path()?, text).map_err(|e| e.to_string())
}

/// `${VAR}` and `${VAR:-default}` from the environment, as Claude Code expands them in `.mcp.json`.
fn expand_env(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &rest[start + 2..start + end];
        let (name, default) = match inner.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (inner, None),
        };
        match std::env::var(name).ok().filter(|value| !value.is_empty()).or(default.map(String::from)) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..start + end + 1]),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

/// Reads Claude Code's project format: `{"mcpServers": {name: {command, args, env}` or
/// `{type: "http" | "sse", url, headers}}}`.
fn parse_project_config(value: &Value) -> Vec<ProjectServer> {
    let Some(servers) = value["mcpServers"].as_object() else { return Vec::new() };
    let strings = |value: &Value| -> HashMap<String, String> {
        value
            .as_object()
            .map(|map| map.iter().filter_map(|(key, value)| Some((key.clone(), expand_env(value.as_str()?)))).collect())
            .unwrap_or_default()
    };
    let mut list: Vec<ProjectServer> = servers
        .iter()
        .filter(|(name, _)| valid_name(name))
        .filter_map(|(name, entry)| {
            let kind = entry["type"].as_str().unwrap_or(if entry.get("url").is_some() { "http" } else { "stdio" });
            let remote = matches!(kind, "http" | "sse" | "streamable-http");
            let config = ServerConfig {
                name: name.clone(),
                command: if remote { String::new() } else { expand_env(entry["command"].as_str()?.trim()) },
                url: if remote { Some(expand_env(entry["url"].as_str()?.trim())) } else { None },
                headers: strings(&entry["headers"]),
                args: entry["args"].as_array().map(|args| args.iter().filter_map(Value::as_str).map(expand_env).collect()).unwrap_or_default(),
                env: strings(&entry["env"]),
                enabled: entry["disabled"].as_bool() != Some(true),
            };
            (!config.command.is_empty() || config.url.as_deref().is_some_and(|url| !url.is_empty())).then_some(ProjectServer { config, legacy_sse: kind == "sse" })
        })
        .collect();
    list.sort_by(|a, b| a.config.name.cmp(&b.config.name));
    list
}

/// Connectors the project at `root` declares in `.mcp.json`.
pub fn read_project_config(root: &std::path::Path) -> Vec<ProjectServer> {
    fs::read_to_string(root.join(".mcp.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok())
        .map(|value| parse_project_config(&value))
        .unwrap_or_default()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The function name the model sees for a server's tool.
pub fn function_name(server: &str, tool: &str) -> String {
    let tool: String = tool
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    format!("mcp__{server}__{tool}").chars().take(64).collect()
}

impl Client {
    async fn start(config: &ServerConfig) -> Result<Arc<Client>, String> {
        #[cfg(windows)]
        let mut command = {
            // npx, uvx and friends are .cmd shims on Windows, so go through cmd.
            let mut command = Command::new("cmd");
            command.arg("/C").arg(&config.command).args(&config.args);
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new(&config.command);
            command.args(&config.args);
            command
        };
        // Keep the server's package caches inside Neru's storage, like every other Neru tool.
        let local = std::path::Path::new(r"D:\Neru\.local");
        if local.is_dir() {
            for (key, sub) in [
                ("NPM_CONFIG_CACHE", r"cache\npm"),
                ("UV_CACHE_DIR", r"cache\uv"),
                ("PIP_CACHE_DIR", r"cache\pip"),
                ("XDG_CACHE_HOME", "cache"),
                ("TEMP", "tmp"),
                ("TMP", "tmp"),
            ] {
                command.env(key, local.join(sub));
            }
        }
        command
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", config.command))?;
        let stdin = child.stdin.take().ok_or("No stdin")?;
        let stdout = child.stdout.take().ok_or("No stdout")?;
        let stderr = child.stderr.take().ok_or("No stderr")?;
        let log = Arc::new(Mutex::new(VecDeque::new()));
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let client = Arc::new(Client {
            stdin: tokio::sync::Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            tools: Mutex::new(Vec::new()),
            stderr: log.clone(),
            alive: alive.clone(),
            _child: Mutex::new(child),
        });
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut log) = log.lock() {
                    log.push_back(line.chars().take(400).collect());
                    while log.len() > 12 {
                        log.pop_front();
                    }
                }
            }
        });
        let reader = Arc::downgrade(&client);
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Some(client) = reader.upgrade() else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                client.handle(message).await;
            }
            alive.store(false, Ordering::SeqCst);
            if let Some(client) = reader.upgrade() {
                let waiting: Vec<Reply> = client
                    .pending
                    .lock()
                    .map(|mut map| map.drain().map(|(_, reply)| reply).collect())
                    .unwrap_or_default();
                let reason = client.last_error();
                for reply in waiting {
                    let _ = reply.send(Err(format!("The server stopped. {reason}")));
                }
            }
        });
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "Neru", "version": env!("CARGO_PKG_VERSION") }
                }),
                Duration::from_secs(90),
            )
            .await?;
        client.notify("notifications/initialized", json!({})).await?;
        client.refresh_tools().await?;
        Ok(client)
    }

    fn last_error(&self) -> String {
        self.stderr
            .lock()
            .map(|log| log.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    async fn handle(&self, message: Value) {
        let id = message.get("id").cloned();
        if message.get("method").is_some() {
            // A request from the server (ping, roots/list, sampling…): answer so it never hangs.
            if let Some(id) = id {
                let reply = if message["method"] == "ping" {
                    json!({"jsonrpc":"2.0","id":id,"result":{}})
                } else {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Not supported by Neru"}})
                };
                let _ = self.send(&reply).await;
            }
            return;
        }
        let Some(id) = id.and_then(|id| id.as_u64()) else { return };
        let Some(reply) = self.pending.lock().ok().and_then(|mut map| map.remove(&id)) else {
            return;
        };
        let result = match message.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or("MCP error").to_string()),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = reply.send(result);
    }

    async fn send(&self, message: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc":"2.0","method":method,"params":params})).await
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(format!("The server is not running. {}", self.last_error()));
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|e| e.to_string())?
            .insert(id, sender);
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("The server closed the request".into()),
            Err(_) => {
                if let Ok(mut map) = self.pending.lock() {
                    map.remove(&id);
                }
                Err(format!("{method} timed out after {} seconds", timeout.as_secs()))
            }
        }
    }

    async fn refresh_tools(&self) -> Result<(), String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({ "cursor": cursor }))
                .unwrap_or_else(|| json!({}));
            let page = self.request("tools/list", params, Duration::from_secs(30)).await?;
            tools.extend(parse_tools(&page));
            cursor = page["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        *self.tools.lock().map_err(|e| e.to_string())? = tools;
        Ok(())
    }
}

fn parse_tools(page: &Value) -> Vec<ToolInfo> {
    page["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tool| {
            Some(ToolInfo {
                name: tool["name"].as_str()?.to_string(),
                description: tool["description"].as_str().unwrap_or("").chars().take(1000).collect(),
                read_only: tool["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                schema: tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}})),
            })
        })
        .collect()
}

enum HttpError {
    /// 401: the connector wants a sign-in; carries its WWW-Authenticate header.
    NeedsAuth(Option<String>),
    Other(String),
}

/// A hosted connector over Streamable HTTP: JSON-RPC POSTs answered with JSON or an SSE stream.
struct HttpClient {
    name: String,
    url: String,
    headers: HashMap<String, String>,
    http: reqwest::Client,
    session: Mutex<Option<String>>,
    token: tokio::sync::Mutex<Option<oauth::Token>>,
    next: AtomicU64,
    tools: Mutex<Vec<ToolInfo>>,
    alive: std::sync::atomic::AtomicBool,
    last: Mutex<String>,
}

fn event_messages(buffer: &str) -> Vec<Value> {
    buffer
        .split("\n\n")
        .filter_map(|event| {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                .collect();
            (!data.is_empty()).then(|| serde_json::from_str(&data.join("\n")).ok()).flatten()
        })
        .collect()
}

impl HttpClient {
    async fn start(config: &ServerConfig) -> Result<Arc<Self>, HttpError> {
        let client = Arc::new(Self {
            name: config.name.clone(),
            url: config.url.clone().unwrap_or_default(),
            headers: config.headers.clone(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| HttpError::Other(e.to_string()))?,
            session: Mutex::new(None),
            token: tokio::sync::Mutex::new(oauth::load(&config.name)),
            next: AtomicU64::new(1),
            tools: Mutex::new(Vec::new()),
            alive: std::sync::atomic::AtomicBool::new(true),
            last: Mutex::new(String::new()),
        });
        let init = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "Neru", "version": env!("CARGO_PKG_VERSION") }
        }});
        match client.post(&init, Some(0)).await? {
            Some(reply) if reply.get("error").is_some() => {
                return Err(HttpError::Other(reply["error"]["message"].as_str().unwrap_or("initialize failed").into()));
            }
            Some(_) => {}
            None => return Err(HttpError::Other("The connector did not answer initialize".into())),
        }
        client.post(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}), None).await?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = cursor.as_ref().map(|cursor| json!({ "cursor": cursor })).unwrap_or_else(|| json!({}));
            let page = client
                .request("tools/list", params, Duration::from_secs(30))
                .await
                .map_err(HttpError::Other)?;
            tools.extend(parse_tools(&page));
            cursor = page["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        if let Ok(mut slot) = client.tools.lock() {
            *slot = tools;
        }
        Ok(client)
    }

    async fn post(&self, message: &Value, id: Option<u64>) -> Result<Option<Value>, HttpError> {
        for attempt in 0..2 {
            let mut request = self
                .http
                .post(&self.url)
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                .json(message);
            for (key, value) in &self.headers {
                request = request.header(key, value);
            }
            if let Some(session) = self.session.lock().ok().and_then(|slot| slot.clone()) {
                request = request.header("Mcp-Session-Id", session);
            }
            {
                let mut token = self.token.lock().await;
                if let Some(current) = token.as_mut() {
                    if current.expired() || attempt == 1 {
                        if let Ok(fresh) = oauth::refresh(&self.http, current).await {
                            *current = fresh;
                            let _ = oauth::store(&self.name, Some(current));
                        }
                    }
                    request = request.bearer_auth(&current.access_token);
                }
            }
            let response = request.send().await.map_err(|e| HttpError::Other(format!("Could not reach the connector: {e}")))?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                let has_refresh = self.token.lock().await.as_ref().is_some_and(|token| token.refresh_token.is_some());
                if attempt == 0 && has_refresh {
                    continue;
                }
                let www = response
                    .headers()
                    .get(reqwest::header::WWW_AUTHENTICATE)
                    .and_then(|value| value.to_str().ok())
                    .map(String::from);
                return Err(HttpError::NeedsAuth(www));
            }
            if let Some(session) = response.headers().get("mcp-session-id").and_then(|value| value.to_str().ok()) {
                if let Ok(mut slot) = self.session.lock() {
                    *slot = Some(session.to_string());
                }
            }
            let status = response.status();
            if status == reqwest::StatusCode::ACCEPTED || status == reqwest::StatusCode::NO_CONTENT || id.is_none() {
                return Ok(None);
            }
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(HttpError::Other(format!("The connector answered {status}: {}", text.chars().take(300).collect::<String>())));
            }
            let stream = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("text/event-stream"));
            let matches = |value: &Value| id.is_some_and(|id| value["id"].as_u64() == Some(id));
            if stream {
                let mut response = response;
                let mut buffer = String::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| HttpError::Other(e.to_string()))? {
                    buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
                    if let Some(found) = event_messages(&buffer).into_iter().find(|value| matches(value)) {
                        return Ok(Some(found));
                    }
                }
                return Err(HttpError::Other("The connector closed the stream without an answer".into()));
            }
            let body: Value = response.json().await.map_err(|e| HttpError::Other(e.to_string()))?;
            return Ok(match body {
                Value::Array(items) => items.into_iter().find(|value| matches(value)),
                other => Some(other),
            });
        }
        Err(HttpError::NeedsAuth(None))
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let reply = match tokio::time::timeout(timeout, self.post(&message, Some(id))).await {
            Err(_) => return Err(format!("{method} timed out after {} seconds", timeout.as_secs())),
            Ok(Err(HttpError::NeedsAuth(_))) => {
                self.alive.store(false, Ordering::SeqCst);
                return Err(format!("The {} connector needs you to sign in again (Settings → Connectors)", self.name));
            }
            Ok(Err(HttpError::Other(error))) => {
                if let Ok(mut last) = self.last.lock() {
                    *last = error.clone();
                }
                return Err(error);
            }
            Ok(Ok(reply)) => reply.ok_or("The connector sent no answer")?,
        };
        match reply.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or("MCP error").to_string()),
            None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
        }
    }
}

/// A running connector, local (stdio) or hosted (HTTP).
#[derive(Clone)]
enum Connection {
    Stdio(Arc<Client>),
    Http(Arc<HttpClient>),
}

impl Connection {
    fn alive(&self) -> bool {
        match self {
            Self::Stdio(client) => client.alive.load(Ordering::SeqCst),
            Self::Http(client) => client.alive.load(Ordering::SeqCst),
        }
    }

    fn tools(&self) -> Vec<ToolInfo> {
        let tools = match self {
            Self::Stdio(client) => client.tools.lock().map(|tools| tools.clone()),
            Self::Http(client) => client.tools.lock().map(|tools| tools.clone()),
        };
        tools.unwrap_or_default()
    }

    fn last_error(&self) -> String {
        match self {
            Self::Stdio(client) => client.last_error(),
            Self::Http(client) => client.last.lock().map(|last| last.clone()).unwrap_or_default(),
        }
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        match self {
            Self::Stdio(client) => client.request(method, params, timeout).await,
            Self::Http(client) => client.request(method, params, timeout).await,
        }
    }
}

impl McpManager {
    fn client(&self, name: &str) -> Option<Connection> {
        self.clients.lock().ok()?.get(name).cloned()
    }

    fn stop(&self, name: &str) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(name);
        }
        if let Ok(mut errors) = self.errors.lock() {
            errors.remove(name);
        }
        if let Ok(mut auth) = self.auth_needed.lock() {
            auth.remove(name);
        }
    }

    async fn start(&self, config: &ServerConfig) {
        self.stop(&config.name);
        if let Ok(mut starting) = self.starting.lock() {
            starting.push(config.name.clone());
        }
        let result = if config.url.is_some() {
            match HttpClient::start(config).await {
                Ok(client) => Ok(Connection::Http(client)),
                Err(HttpError::NeedsAuth(www)) => {
                    if let Ok(mut auth) = self.auth_needed.lock() {
                        auth.insert(config.name.clone(), www);
                    }
                    Err("Sign in to connect this account".to_string())
                }
                Err(HttpError::Other(error)) if self.is_legacy_sse(&config.name) => Err(format!(
                    "{error}. This connector is declared with \"type\": \"sse\" in .mcp.json; Neru only speaks Streamable HTTP, not the legacy SSE transport. Point it at the server's Streamable HTTP endpoint (often /mcp) and set \"type\": \"http\"."
                )),
                Err(HttpError::Other(error)) => Err(error),
            }
        } else {
            Client::start(config).await.map(Connection::Stdio)
        };
        if let Ok(mut starting) = self.starting.lock() {
            starting.retain(|name| name != &config.name);
        }
        match result {
            Ok(client) => {
                if let Ok(mut clients) = self.clients.lock() {
                    clients.insert(config.name.clone(), client);
                }
            }
            Err(error) => {
                if let Ok(mut errors) = self.errors.lock() {
                    errors.insert(config.name.clone(), error);
                }
            }
        }
    }

    fn is_legacy_sse(&self, name: &str) -> bool {
        self.project.lock().is_ok_and(|project| project.iter().any(|server| server.config.name == name && server.legacy_sse))
    }

    fn project_configs(&self) -> Vec<ServerConfig> {
        self.project.lock().map(|project| project.iter().map(|server| server.config.clone()).collect()).unwrap_or_default()
    }

    /// Saved connectors and the open project's `.mcp.json` ones (which win on a name clash),
    /// each with its source: "user" or "project".
    fn configs(&self) -> Vec<(ServerConfig, &'static str)> {
        let project = self.project_configs();
        let mut all: Vec<(ServerConfig, &'static str)> = read_config()
            .into_iter()
            .filter(|config| !project.iter().any(|item| item.name == config.name))
            .map(|config| (config, "user"))
            .collect();
        all.extend(project.into_iter().map(|config| (config, "project")));
        all.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        all
    }

    fn find_config(&self, name: &str) -> Option<ServerConfig> {
        self.configs().into_iter().map(|(config, _)| config).find(|config| config.name == name)
    }

    fn is_project(&self, name: &str) -> bool {
        self.project_configs().iter().any(|config| config.name == name)
    }

    /// Switches the project connectors to those in `root`'s `.mcp.json` (none for `None`): stops
    /// ones that went away or changed, brings back a saved connector a removed one had shadowed,
    /// and starts the new ones. An untrusted project's connectors are listed but not started.
    pub async fn sync_project(&self, root: Option<&std::path::Path>) {
        let next = root.map(read_project_config).unwrap_or_default();
        let trusted = !next.is_empty() && root.is_some_and(crate::trust::is_trusted);
        let was_trusted = self.project_trusted.swap(trusted, Ordering::SeqCst);
        let previous = match self.project.lock() {
            Ok(mut project) => std::mem::replace(&mut *project, next.clone()),
            Err(_) => return,
        };
        let saved = read_config();
        for old in &previous {
            if next.iter().any(|server| server.config == old.config) {
                continue;
            }
            self.stop(&old.config.name);
            if !next.iter().any(|server| server.config.name == old.config.name) {
                if let Some(config) = saved.iter().find(|config| config.name == old.config.name && config.enabled) {
                    self.start(config).await;
                }
            }
        }
        for server in &next {
            let running = self.client(&server.config.name).is_some_and(|client| client.alive());
            if !server.config.enabled || !trusted {
                self.stop(&server.config.name);
            } else if !(running && was_trusted && previous.iter().any(|old| old.config == server.config)) {
                self.start(&server.config).await;
            }
        }
    }

    /// A connector from a project the user has not trusted, which must not start.
    fn needs_trust(&self, name: &str) -> bool {
        self.is_project(name) && !self.project_trusted.load(Ordering::SeqCst)
    }

    fn views(&self) -> Vec<ServerView> {
        self.configs()
            .into_iter()
            .map(|(config, source)| {
                if source == "project" && config.enabled && !self.project_trusted.load(Ordering::SeqCst) {
                    let error = Some("Trust this project to start the connectors in its .mcp.json".to_string());
                    return ServerView { config, status: "needs_trust".into(), signed_in: false, error, tools: vec![], source: source.into() };
                }
                let client = self.client(&config.name).filter(|client| client.alive());
                let needs_auth = self.auth_needed.lock().map(|auth| auth.contains_key(&config.name)).unwrap_or(false);
                let signed_in = config.url.is_some() && oauth::load(&config.name).is_some();
                let starting = self
                    .starting
                    .lock()
                    .map(|list| list.contains(&config.name))
                    .unwrap_or(false);
                let error = self.errors.lock().ok().and_then(|errors| errors.get(&config.name).cloned());
                let (status, tools) = match (&client, starting, config.enabled) {
                    (_, true, _) => ("starting", vec![]),
                    (Some(client), _, _) => ("connected", client.tools()),
                    (None, _, true) if needs_auth => ("needs_auth", vec![]),
                    (None, _, false) => ("off", vec![]),
                    (None, _, true) => ("error", vec![]),
                };
                let error = match (status, error) {
                    ("error", None) => Some(self.client(&config.name).map(|client| client.last_error()).filter(|text| !text.is_empty()).unwrap_or_else(|| "The server stopped".into())),
                    (_, error) => error,
                };
                ServerView { config, status: status.into(), signed_in, error, tools, source: source.into() }
            })
            .collect()
    }

    /// Tool definitions for the model plus a map from function name back to (server, tool).
    pub fn tools(&self, read_only: bool) -> (Vec<Value>, HashMap<String, (String, String, bool)>) {
        let mut specs = Vec::new();
        let mut names = HashMap::new();
        let Ok(clients) = self.clients.lock() else {
            return (specs, names);
        };
        for (server, client) in clients.iter() {
            if !client.alive() {
                continue;
            }
            for tool in client.tools() {
                if read_only && !tool.read_only {
                    continue;
                }
                let function = function_name(server, &tool.name);
                specs.push(json!({"type":"function","function":{
                    "name": function,
                    "description": format!("[{server} MCP] {}", tool.description),
                    "parameters": tool.schema,
                }}));
                names.insert(function, (server.clone(), tool.name.clone(), tool.read_only));
            }
        }
        (specs, names)
    }

    /// Calls a tool and flattens its content to text for the model.
    pub async fn call(&self, server: &str, tool: &str, arguments: Value) -> Result<String, String> {
        let client = self
            .client(server)
            .ok_or_else(|| format!("The {server} connector is not running"))?;
        let result = client
            .request(
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
                Duration::from_secs(180),
            )
            .await?;
        let mut text = String::new();
        for part in result["content"].as_array().cloned().unwrap_or_default() {
            match part["type"].as_str() {
                Some("text") => text.push_str(part["text"].as_str().unwrap_or("")),
                Some("resource") => text.push_str(
                    part["resource"]["text"]
                        .as_str()
                        .unwrap_or("[binary resource]"),
                ),
                Some(other) => text.push_str(&format!("[{other} content]")),
                None => {}
            }
            text.push('\n');
        }
        if text.trim().is_empty() {
            if let Some(structured) = result.get("structuredContent") {
                text = structured.to_string();
            }
        }
        if text.len() > 60_000 {
            text.truncate(60_000);
            text.push_str("\n[output truncated]");
        }
        if result["isError"].as_bool() == Some(true) {
            return Ok(format!("Error from {server}: {text}"));
        }
        Ok(text)
    }
}

/// Starts every enabled connector, including the open project's `.mcp.json` ones; called once
/// when Neru opens.
pub async fn start_enabled(app: AppHandle) {
    let state = app.state::<AppState>();
    let root = state.root.lock().ok().and_then(|root| root.clone());
    let project = root.as_deref().map(read_project_config).unwrap_or_default();
    for config in read_config().into_iter().filter(|config| config.enabled) {
        if !project.iter().any(|server| server.config.name == config.name) {
            state.mcp.start(&config).await;
        }
    }
    state.mcp.sync_project(root.as_deref()).await;
}

#[tauri::command]
pub fn mcp_servers(app: AppHandle) -> Vec<ServerView> {
    app.state::<AppState>().mcp.views()
}

/// Brings the project connectors in line with the open project's `.mcp.json`; the window calls
/// this after opening a project.
#[tauri::command]
pub async fn mcp_sync_project(app: AppHandle) -> Result<Vec<ServerView>, String> {
    let state = app.state::<AppState>();
    let root = state.root.lock().map_err(|e| e.to_string())?.clone();
    state.mcp.sync_project(root.as_deref()).await;
    Ok(state.mcp.views())
}

fn untrusted(name: &str) -> String {
    format!("{name} comes from this project's .mcp.json. Trust the project folder to start it.")
}

fn project_owned(name: &str) -> String {
    format!("{name} comes from this project's .mcp.json. Change it in that file instead.")
}

/// Adds or updates a connector (renaming when `previous` differs) and (re)starts it if enabled.
#[tauri::command]
pub async fn mcp_save_server(
    server: ServerConfig,
    previous: Option<String>,
    app: AppHandle,
) -> Result<Vec<ServerView>, String> {
    let server = ServerConfig {
        name: server.name.trim().to_string(),
        command: server.command.trim().to_string(),
        args: server.args.into_iter().map(|arg| arg.trim().to_string()).filter(|arg| !arg.is_empty()).collect(),
        ..server
    };
    if !valid_name(&server.name) {
        return Err("Name the connector with 1–32 letters, digits, - or _".into());
    }
    let server = ServerConfig {
        url: server.url.map(|url| url.trim().to_string()).filter(|url| !url.is_empty()),
        headers: server
            .headers
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .filter(|(key, _)| !key.is_empty())
            .collect(),
        ..server
    };
    match &server.url {
        Some(url) => {
            let parsed = reqwest::Url::parse(url).map_err(|_| "Enter the connector's full URL")?;
            let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
                return Err("Hosted connectors must use HTTPS".into());
            }
        }
        None if server.command.is_empty() => return Err("Enter the command that starts the server, or a URL".into()),
        None => {}
    }
    let state = app.state::<AppState>();
    if state.mcp.is_project(&server.name) {
        return Err(project_owned(&server.name));
    }
    let mut servers = read_config();
    let previous = previous.unwrap_or_else(|| server.name.clone());
    if server.name != previous && servers.iter().any(|item| item.name == server.name) {
        return Err("A connector with that name already exists".into());
    }
    servers.retain(|item| item.name != previous && item.name != server.name);
    servers.push(server.clone());
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    write_config(&servers)?;
    state.mcp.stop(&previous);
    if server.enabled {
        state.mcp.start(&server).await;
    } else {
        state.mcp.stop(&server.name);
    }
    Ok(state.mcp.views())
}

#[tauri::command]
pub fn mcp_remove_server(name: String, app: AppHandle) -> Result<Vec<ServerView>, String> {
    if app.state::<AppState>().mcp.is_project(&name) {
        return Err(project_owned(&name));
    }
    let mut servers = read_config();
    servers.retain(|item| item.name != name);
    write_config(&servers)?;
    let _ = oauth::store(&name, None);
    let state = app.state::<AppState>();
    state.mcp.stop(&name);
    Ok(state.mcp.views())
}

/// Opens the browser to sign in to a hosted connector, then connects it.
#[tauri::command]
pub async fn mcp_sign_in(name: String, app: AppHandle) -> Result<Vec<ServerView>, String> {
    let state = app.state::<AppState>();
    if state.mcp.needs_trust(&name) {
        return Err(untrusted(&name));
    }
    let config = state.mcp.find_config(&name).ok_or("Unknown connector")?;
    let url = config.url.clone().ok_or("Only hosted connectors sign in")?;
    let hint = state.mcp.auth_needed.lock().ok().and_then(|auth| auth.get(&name).cloned()).flatten();
    oauth::sign_in(&name, &url, hint.as_deref()).await?;
    state.mcp.start(&config).await;
    Ok(state.mcp.views())
}

#[tauri::command]
pub async fn mcp_sign_out(name: String, app: AppHandle) -> Result<Vec<ServerView>, String> {
    oauth::store(&name, None)?;
    let state = app.state::<AppState>();
    if let Some(config) = state.mcp.find_config(&name).filter(|item| item.enabled && !state.mcp.needs_trust(&item.name)) {
        state.mcp.start(&config).await;
    } else {
        state.mcp.stop(&name);
    }
    Ok(state.mcp.views())
}

#[tauri::command]
pub async fn mcp_restart(name: String, app: AppHandle) -> Result<Vec<ServerView>, String> {
    let state = app.state::<AppState>();
    if state.mcp.needs_trust(&name) {
        return Err(untrusted(&name));
    }
    let config = state.mcp.find_config(&name).ok_or("Unknown connector")?;
    state.mcp.start(&config).await;
    Ok(state.mcp.views())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_names_are_safe_and_short() {
        assert_eq!(function_name("files", "read_file"), "mcp__files__read_file");
        assert_eq!(function_name("gh", "search.repos"), "mcp__gh__search_repos");
        assert!(function_name("server", &"x".repeat(100)).len() <= 64);
        assert!(valid_name("my-server_1"));
        assert!(!valid_name("bad name"));
    }

    #[test]
    fn reads_claude_project_mcp_json() {
        // SAFETY: tests in this module do not read this variable concurrently.
        unsafe { std::env::set_var("NERU_TEST_MCP_TOKEN", "secret") };
        let value = json!({"mcpServers": {
            "files": {"command": "npx", "args": ["-y", "@mcp/fs", "${NERU_TEST_MCP_ROOT:-.}"], "env": {"TOKEN": "${NERU_TEST_MCP_TOKEN}"}},
            "docs": {"type": "http", "url": "https://docs.example.com/mcp", "headers": {"Authorization": "Bearer ${NERU_TEST_MCP_TOKEN}"}},
            "old": {"type": "sse", "url": "https://old.example.com/sse"},
            "off": {"command": "x", "disabled": true},
            "bad name": {"command": "x"},
            "empty": {"type": "stdio"}
        }});
        let servers = parse_project_config(&value);
        let names: Vec<&str> = servers.iter().map(|server| server.config.name.as_str()).collect();
        assert_eq!(names, vec!["docs", "files", "off", "old"]);
        let files = &servers[1].config;
        assert_eq!(files.command, "npx");
        assert_eq!(files.args, vec!["-y", "@mcp/fs", "."]);
        assert_eq!(files.env["TOKEN"], "secret");
        assert_eq!(servers[0].config.url.as_deref(), Some("https://docs.example.com/mcp"));
        assert_eq!(servers[0].config.headers["Authorization"], "Bearer secret");
        assert!(servers[3].legacy_sse && !servers[0].legacy_sse);
        assert!(!servers[2].config.enabled);
        assert_eq!(expand_env("${NERU_TEST_UNSET_VAR}"), "${NERU_TEST_UNSET_VAR}");
    }

    #[test]
    fn reads_json_rpc_from_event_streams() {
        let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"ok\":true}}\n\n";
        let messages = event_messages(stream);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["id"], 3);
    }
}
