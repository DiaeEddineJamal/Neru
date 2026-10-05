//! Neru Remote: a WebSocket server a paired phone uses to watch and steer this computer's sessions
//! and Team tasks. Off until the user turns it on in Settings → Phone. Every frame is
//! base64url(nonce || XChaCha20-Poly1305 ciphertext) under the pairing key, with no plaintext
//! fallback: a frame that does not open closes the connection, which is how a wrong key is turned
//! away. The message shapes live in src/remoteProtocol.ts, which the phone app imports.

use std::{
    fs,
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, watch},
};
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use crate::{AppState, PendingAction, agent, extras, sessions, settings, subagent, team, workspace::data_dir};

pub const DEFAULT_PORT: u16 = 47613;
/// Largest frame or message a phone may send.
const MAX_FRAME: usize = 8 << 20;
const NONCE: usize = 24;
/// How long a connection may stay open before its first frame opens with the key.
const HELLO_WITHIN: Duration = Duration::from_secs(30);
const FILE: &str = "remote.json";

type Key = [u8; 32];

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    enabled: bool,
    internet: bool,
    endpoint: String,
    /// The pairing key, sealed like the API keys (settings.rs).
    key: String,
    /// The port last bound, tried first so a pairing survives restarts.
    port: u16,
}

fn read_stored() -> Stored {
    data_dir()
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join(FILE)).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_stored(stored: &Stored) -> Result<(), String> {
    let text = serde_json::to_string_pretty(stored).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join(FILE), text).map_err(|e| e.to_string())
}

struct Live {
    port: u16,
    accept: tauri::async_runtime::JoinHandle<()>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static KEY: Mutex<Option<Key>> = Mutex::new(None);
static CLIENTS: AtomicUsize = AtomicUsize::new(0);
/// Bumped to close every connection (reset pairing, turning Remote off).
static EPOCH: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);
/// Plain JSON pushes; each connection seals them with the key it opened with.
static PUSH: LazyLock<broadcast::Sender<Arc<String>>> = LazyLock::new(|| broadcast::channel(4096).0);
static APP: OnceLock<AppHandle> = OnceLock::new();

// ---------- frames and pairing ----------

pub fn seal(key: &Key, plain: &[u8]) -> String {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, plain).expect("XChaCha20-Poly1305 encryption cannot fail"));
    URL_SAFE_NO_PAD.encode(out)
}

/// The plaintext of a frame, or None when it was not sealed with `key`.
pub fn open(key: &Key, frame: &str) -> Option<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD.decode(frame.trim().trim_end_matches('=')).ok()?;
    if bytes.len() < NONCE + 16 {
        return None;
    }
    let (nonce, sealed) = bytes.split_at(NONCE);
    XChaCha20Poly1305::new(key.into()).decrypt(XNonce::from_slice(nonce), sealed).ok()
}

pub fn pairing_url(hosts: &[String], port: u16, key: &Key, name: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    format!(
        "neru://pair?v=1&h={}&p={port}&k={}&n={}",
        hosts.join(","),
        URL_SAFE_NO_PAD.encode(key),
        utf8_percent_encode(name, NON_ALPHANUMERIC)
    )
}

/// This computer's LAN IPv4 addresses, the one on the default route first.
fn lan_hosts() -> Vec<String> {
    let primary = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| socket.connect("8.8.8.8:80").and_then(|_| socket.local_addr()))
        .ok()
        .map(|addr| addr.ip().to_string());
    let mut hosts: Vec<String> = primary.into_iter().collect();
    for interface in if_addrs::get_if_addrs().unwrap_or_default() {
        if let std::net::IpAddr::V4(ip) = interface.ip() {
            let text = ip.to_string();
            if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() && !hosts.contains(&text) {
                hosts.push(text);
            }
        }
    }
    hosts
}

fn computer_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()))
        .or_else(|| fs::read_to_string("/etc/hostname").ok().map(|text| text.trim().to_string()).filter(|text| !text.is_empty()))
        .unwrap_or_else(|| "Neru".into())
}

fn qr_svg(text: &str) -> String {
    use qrcode::render::svg;
    qrcode::QrCode::new(text.as_bytes())
        .map(|code| code.render::<svg::Color>().min_dimensions(220, 220).dark_color(svg::Color("#111111")).light_color(svg::Color("#ffffff")).build())
        .unwrap_or_default()
}

// ---------- settings panel ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    enabled: bool,
    port: u16,
    hosts: Vec<String>,
    name: String,
    /// The pairing link, empty while Remote is off.
    pairing: String,
    qr_svg: String,
    clients: usize,
    internet: bool,
    endpoint: String,
    internet_error: String,
}

fn status() -> RemoteStatus {
    let port = LIVE.lock().ok().and_then(|live| live.as_ref().map(|live| live.port));
    let key = KEY.lock().ok().and_then(|key| *key);
    let (hosts, name) = (lan_hosts(), computer_name());
    let stored = read_stored();
    let endpoint = if stored.endpoint.is_empty() { crate::remote_tunnel::url() } else { stored.endpoint.clone() };
    let mut pairing = match (port, key) {
        (Some(port), Some(key)) => pairing_url(&hosts, port, &key, &name),
        _ => String::new(),
    };
    if stored.internet && !pairing.is_empty() && !endpoint.is_empty() {
        use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
        pairing.push_str(&format!("&u={}", utf8_percent_encode(&endpoint, NON_ALPHANUMERIC)));
    }
    RemoteStatus {
        enabled: port.is_some(),
        port: port.unwrap_or(0),
        qr_svg: if pairing.is_empty() { String::new() } else { qr_svg(&pairing) },
        pairing,
        hosts,
        name,
        clients: CLIENTS.load(Ordering::SeqCst),
        internet: stored.internet,
        endpoint,
        internet_error: crate::remote_tunnel::error(),
    }
}

#[tauri::command]
pub fn remote_status() -> RemoteStatus {
    status()
}

#[tauri::command]
pub async fn remote_set_enabled(enabled: bool, app: AppHandle) -> Result<RemoteStatus, String> {
    let mut stored = read_stored();
    stored.enabled = enabled;
    if enabled {
        start(&app, &mut stored).await?;
    } else {
        stop();
    }
    write_stored(&stored)?;
    Ok(status())
}

/// A new key: every paired phone is disconnected and has to scan again.
#[tauri::command]
pub fn remote_reset_pairing() -> Result<RemoteStatus, String> {
    let mut stored = read_stored();
    set_key(&mut stored, XChaCha20Poly1305::generate_key(&mut OsRng).into());
    EPOCH.send_modify(|epoch| *epoch += 1);
    write_stored(&stored)?;
    Ok(status())
}

fn set_key(stored: &mut Stored, key: Key) {
    // Where the platform cannot protect it, the key lasts until Neru quits.
    stored.key = settings::seal_secret(&URL_SAFE_NO_PAD.encode(key)).unwrap_or_default();
    if let Ok(mut current) = KEY.lock() {
        *current = Some(key);
    }
}

#[tauri::command]
pub fn remote_set_internet(enabled: bool, app: AppHandle) -> Result<RemoteStatus, String> {
    let mut stored = read_stored();
    let port = LIVE.lock().map_err(|e| e.to_string())?.as_ref().map(|l| l.port).ok_or("Turn Neru Remote on first.")?;
    stored.internet = enabled;
    write_stored(&stored)?;
    if enabled && stored.endpoint.is_empty() { crate::remote_tunnel::start(app, port); }
    else { crate::remote_tunnel::stop(); }
    Ok(status())
}

#[tauri::command]
pub fn remote_set_endpoint(endpoint: String, app: AppHandle) -> Result<RemoteStatus, String> {
    let endpoint = endpoint.trim().to_string();
    if !endpoint.is_empty() {
        let url = reqwest::Url::parse(&endpoint).map_err(|_| "Enter a valid secure WebSocket URL.")?;
        if url.scheme() != "wss" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err("Use a wss:// URL without credentials or a fragment.".into());
        }
    }
    let mut stored = read_stored();
    stored.endpoint = endpoint;
    write_stored(&stored)?;
    crate::remote_tunnel::stop();
    if stored.internet && stored.endpoint.is_empty() {
        if let Some(port) = LIVE.lock().map_err(|e| e.to_string())?.as_ref().map(|l| l.port) { crate::remote_tunnel::start(app, port); }
    }
    Ok(status())
}

fn stored_key(stored: &Stored) -> Option<Key> {
    let text = settings::open_secret(&stored.key)?;
    URL_SAFE_NO_PAD.decode(text.trim()).ok()?.try_into().ok()
}

async fn start(app: &AppHandle, stored: &mut Stored) -> Result<(), String> {
    if LIVE.lock().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    let have_key = KEY.lock().map_err(|e| e.to_string())?.is_some();
    if !have_key {
        match stored_key(stored) {
            Some(key) => *KEY.lock().map_err(|e| e.to_string())? = Some(key),
            None => set_key(stored, XChaCha20Poly1305::generate_key(&mut OsRng).into()),
        }
    }
    let wanted = if stored.port == 0 { DEFAULT_PORT } else { stored.port };
    let listener = match TcpListener::bind(("0.0.0.0", wanted)).await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind(("0.0.0.0", 0)).await.map_err(|e| format!("Could not open a port for Neru Remote: {e}"))?,
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    stored.port = port;
    let tunnel_app = app.clone();
    let app = app.clone();
    let accept = tauri::async_runtime::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tauri::async_runtime::spawn(serve(app.clone(), stream));
        }
    });
    *LIVE.lock().map_err(|e| e.to_string())? = Some(Live { port, accept });
    if stored.internet && stored.endpoint.is_empty() { crate::remote_tunnel::start(tunnel_app, port); }
    Ok(())
}

fn stop() {
    crate::remote_tunnel::stop();
    if let Some(live) = LIVE.lock().ok().and_then(|mut live| live.take()) {
        live.accept.abort();
    }
    EPOCH.send_modify(|epoch| *epoch += 1);
}

/// Forwards agent and Team events to connected phones, and starts Remote if it was left on.
/// Desktop window only; the CLI never serves phones.
pub fn init(app: &AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    app.listen("agent://event", |event| {
        if PUSH.receiver_count() == 0 {
            return;
        }
        let payload = event.payload();
        push(format!(r#"{{"event":"agent","payload":{payload}}}"#));
        // A run started, stopped or now waits on an approval: the list changed.
        if payload.contains(r#""type":"status""#) {
            push_sessions();
        }
    });
    app.listen("team://event", |event| {
        if PUSH.receiver_count() > 0 {
            push(format!(r#"{{"event":"team","payload":{}}}"#, event.payload()));
        }
    });
    let mut stored = read_stored();
    if stored.enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if start(&app, &mut stored).await.is_ok() {
                let _ = write_stored(&stored);
            }
        });
    }
}

fn push(text: String) {
    let _ = PUSH.send(Arc::new(text));
}

/// Sends the session list, off the emitting thread (it may hold a session lock).
fn push_sessions() {
    let Some(app) = APP.get().cloned() else { return };
    if PUSH.receiver_count() == 0 {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Ok(list) = phone_sessions(&app) {
            push(json!({"event": "sessions", "payload": list}).to_string());
        }
    });
}

fn clients_changed(app: &AppHandle) {
    let _ = app.emit("remote://clients", CLIENTS.load(Ordering::SeqCst));
}

// ---------- connections ----------

#[derive(Deserialize)]
struct Request {
    id: Value,
    cmd: String,
    #[serde(default)]
    args: Value,
}

async fn serve(app: AppHandle, stream: TcpStream) {
    let Some(key) = KEY.lock().ok().and_then(|key| *key) else { return };
    let changed = app.clone();
    let commands = move |cmd: String, args: Value| {
        let app = app.clone();
        async move { handle(&app, &cmd, &args).await }
    };
    connection(stream, key, commands, move || clients_changed(&changed)).await
}

/// One phone's socket: opens its frames with `key`, runs them through `commands`, seals replies and pushes.
/// `clients_changed` runs when it pairs and when it closes.
async fn connection<C, F>(stream: TcpStream, key: Key, commands: C, clients_changed: impl Fn())
where
    C: Fn(String, Value) -> F,
    F: std::future::Future<Output = Result<Value, String>> + Send + 'static,
{
    let mut config = WebSocketConfig::default();
    config.max_frame_size = Some(MAX_FRAME);
    config.max_message_size = Some(MAX_FRAME);
    let Ok(socket) = tokio_tungstenite::accept_async_with_config(stream, Some(config)).await else { return };
    let mut epoch = EPOCH.subscribe();
    epoch.borrow_and_update();
    let mut pushes = PUSH.subscribe();
    let (replies, mut replied) = mpsc::unbounded_channel::<String>();
    let (mut sink, mut source) = socket.split();
    let deadline = tokio::time::sleep(HELLO_WITHIN);
    tokio::pin!(deadline);
    let mut paired = false;
    loop {
        let out = tokio::select! {
            _ = epoch.changed() => break,
            _ = &mut deadline, if !paired => break,
            message = source.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let Some(plain) = open(&key, text.as_str()) else { break };
                    if !paired {
                        paired = true;
                        CLIENTS.fetch_add(1, Ordering::SeqCst);
                        clients_changed();
                    }
                    let Ok(request) = serde_json::from_slice::<Request>(&plain) else { break };
                    let (running, replies) = (commands(request.cmd, request.args), replies.clone());
                    // Approving can run a command for minutes; the socket keeps flowing meanwhile.
                    tauri::async_runtime::spawn(async move {
                        let reply = match running.await {
                            Ok(data) => json!({"id": request.id, "ok": true, "data": data}),
                            Err(error) => json!({"id": request.id, "ok": false, "error": error}),
                        };
                        let _ = replies.send(reply.to_string());
                    });
                    continue;
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                _ => break,
            },
            pushed = pushes.recv(), if paired => match pushed {
                Ok(text) => text.as_str().to_string(),
                // Missed some events: at least bring the list back in line.
                Err(broadcast::error::RecvError::Lagged(_)) => { push_sessions(); continue }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            Some(reply) = replied.recv() => reply,
        };
        if sink.send(Message::text(seal(&key, out.as_bytes()))).await.is_err() {
            break;
        }
    }
    let _ = sink.close().await;
    if paired {
        CLIENTS.fetch_sub(1, Ordering::SeqCst);
        clients_changed();
    }
}

// ---------- commands ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhoneSession {
    id: String,
    title: String,
    project: String,
    project_path: String,
    updated_at: u64,
    running: bool,
    waiting: bool,
}

fn phone_sessions(app: &AppHandle) -> Result<Vec<PhoneSession>, String> {
    Ok(sessions::all_with_status(&app.state::<AppState>())?
        .into_iter()
        .map(|(summary, waiting)| PhoneSession {
            project: std::path::Path::new(&summary.project_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Chat".into()),
            id: summary.id,
            title: summary.title,
            project_path: summary.project_path,
            updated_at: summary.updated_at,
            running: summary.running,
            waiting,
        })
        .collect())
}

fn value<T: Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result.and_then(|data| serde_json::to_value(data).map_err(|e| e.to_string()))
}

fn text(args: &Value, name: &str) -> Result<String, String> {
    args[name].as_str().map(str::to_string).ok_or_else(|| format!("Missing {name}"))
}

async fn handle(app: &AppHandle, cmd: &str, args: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    match cmd {
        "hello" => Ok(hello()),
        "list_sessions" => value(phone_sessions(app)),
        "session_snapshot" => value(sessions::session_snapshot(text(args, "sessionId")?, state)),
        "send_prompt" => send_prompt(app, text(args, "sessionId")?, text(args, "text")?, mode(args)).await,
        "approve" => approve(app, text(args, "sessionId")?, args["always"].as_bool().unwrap_or(false), mode(args)).await.map(|_| Value::Null),
        "deny" => deny(app, text(args, "sessionId")?).map(|_| Value::Null),
        "answer" => {
            let id = text(args, "sessionId")?;
            let answers = serde_json::from_value::<Vec<String>>(args["answers"].clone()).map_err(|_| "Missing answers")?;
            extras::answer_question(Some(id.clone()), answers, state)?;
            settled(app, &id);
            resume(app, id, mode(args));
            Ok(Value::Null)
        }
        "answer_subagent" => subagent::answer_subagent_approval(text(args, "requestId")?, args["approve"].as_bool().unwrap_or(false)).map(|_| Value::Null),
        "stop" => agent::stop_chat(Some(text(args, "sessionId")?), state).map(|_| Value::Null),
        "list_teams" => value(team::commands::list_team_tasks()),
        "team_snapshot" => value(team::commands::team_snapshot(text(args, "id")?)),
        "send_team_message" => {
            let to = serde_json::from_value::<Vec<String>>(args["to"].clone()).unwrap_or_default();
            value(team::commands::send_team_message(text(args, "id")?, text(args, "text")?, to, app.clone()))
        }
        "stop_team" => team::commands::stop_team(text(args, "id")?, args["handle"].as_str().map(str::to_string)).map(|_| Value::Null),
        "list_team_agents" => value(team::commands::list_team_agents(app.clone(), None).await),
        "list_agent_models" => value(crate::team_more::commands::list_agent_models(text(args, "kind")?).await),
        "create_team_task" => {
            let members = serde_json::from_value::<Vec<team::NewMember>>(args["members"].clone()).map_err(|_| "Missing members")?;
            let created = team::commands::create_team_task(text(args, "title")?, text(args, "projectPath")?, members, None)?;
            // The phone's prompt goes out as the first message, so the team starts right away.
            let prompt = args["text"].as_str().unwrap_or("").trim().to_string();
            if !prompt.is_empty() {
                let to = serde_json::from_value::<Vec<String>>(args["to"].clone()).unwrap_or_default();
                team::commands::send_team_message(created.task.id.clone(), prompt, to, app.clone())?;
            }
            value(Ok(created))
        }
        _ => Err(format!("Unknown command {cmd}")),
    }
}

/// Cargo.toml's version, which releases keep equal to the app's (README, Releasing).
fn hello() -> Value {
    json!({"name": computer_name(), "version": env!("CARGO_PKG_VERSION")})
}

/// The phone picks the mode for runs it starts; Review (ask every time) unless it says otherwise.
fn mode(args: &Value) -> String {
    args["mode"].as_str().unwrap_or("manual").to_string()
}

/// Steers a running session, or starts a reply in an idle one like the desktop's send button.
async fn send_prompt(app: &AppHandle, id: String, prompt: String, mode: String) -> Result<Value, String> {
    if agent::steer_session(id.clone(), prompt.clone(), app.state::<AppState>())? {
        return Ok(json!({"steered": true}));
    }
    let shared = sessions::runtime(&app.state::<AppState>(), &id)?;
    if sessions::lock(&shared)?.pending.is_some() {
        return Err("Answer the waiting approval first".into());
    }
    run(app, id, prompt, mode);
    Ok(json!({"steered": false}))
}

/// Runs a reply in the background; the phone follows it through agent events.
fn run(app: &AppHandle, id: String, prompt: String, mode: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let chat = sessions::runtime(&app.state::<AppState>(), &id)
            .and_then(|shared| sessions::lock(&shared).map(|runtime| runtime.summary.project_path.is_empty()))
            .unwrap_or(false);
        let _ = app.emit("remote://session", json!({"sessionId": id, "phase": "started"}));
        let surface = if chat { "chat" } else { "code" };
        if let Err(error) = agent::ai_chat(prompt, Vec::new(), mode, None, None, None, None, Some(id.clone()), None, Some(surface.into()), app.clone()).await {
            agent::emit(&app, &id, agent::AgentEvent::Notice { text: format!("Stopped: {error}") });
        }
        settled(&app, &id);
    });
}

/// Continues an agent run after its waiting call was answered, as the desktop does.
fn resume(app: &AppHandle, id: String, mode: String) {
    run(app, id, String::new(), mode);
}

/// Tells the window to reload a session the phone changed, and the phones that the list changed.
fn settled(app: &AppHandle, id: &str) {
    let _ = app.emit("remote://session", json!({"sessionId": id, "phase": "settled"}));
    push_sessions();
}

/// The desktop's Approve (and Always allow) button for a session's waiting call.
async fn approve(app: &AppHandle, id: String, always: bool, mode: String) -> Result<(), String> {
    let shared = sessions::runtime(&app.state::<AppState>(), &id)?;
    let (action, root, project) = {
        let runtime = sessions::lock(&shared)?;
        (runtime.pending.clone().ok_or("Nothing is waiting for approval")?, runtime.work_root(), runtime.summary.project_path.clone())
    };
    let from_agent = action.tool_call_id().is_some();
    let mode = match action {
        PendingAction::Question { .. } => return Err("Answer the questions with the answer command".into()),
        PendingAction::Plan { .. } => extras::resolve_plan(Some(id.clone()), true, Some(mode), None, app.state::<AppState>())?,
        PendingAction::Edit { .. } => {
            crate::workspace::apply_pending_in(&shared, &root)?;
            mode
        }
        _ => {
            if always {
                agent::allow_pending_always_in(&shared, std::path::Path::new(&project))?;
            }
            agent::run_pending_in(app, &shared, &root).await?;
            mode
        }
    };
    settled(app, &id);
    if from_agent {
        resume(app, id, mode);
    }
    Ok(())
}

/// The desktop's Deny button: the agent is told and the run does not continue.
fn deny(app: &AppHandle, id: String) -> Result<(), String> {
    let shared = sessions::runtime(&app.state::<AppState>(), &id)?;
    let plan = matches!(sessions::lock(&shared)?.pending, Some(PendingAction::Plan { .. }));
    if plan {
        extras::resolve_plan(Some(id.clone()), false, None, None, app.state::<AppState>())?;
    } else {
        crate::workspace::deny_pending_in(&shared, "User rejected this operation")?;
    }
    settled(app, &id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Key {
        [byte; 32]
    }

    #[test]
    fn frames_round_trip() {
        let plain = br#"{"id":1,"cmd":"hello","args":{}}"#;
        let frame = seal(&key(7), plain);
        assert!(!frame.contains('=') && !frame.contains('+') && !frame.contains('/'));
        assert_eq!(open(&key(7), &frame).as_deref(), Some(&plain[..]));
        // A fresh nonce every time.
        assert_ne!(frame, seal(&key(7), plain));
    }

    #[test]
    fn frames_from_another_key_are_refused() {
        let frame = seal(&key(7), b"hello");
        assert_eq!(open(&key(8), &frame), None);
        assert_eq!(open(&key(7), "not a frame"), None);
        assert_eq!(open(&key(7), &URL_SAFE_NO_PAD.encode([0u8; 30])), None);
        // A flipped byte fails authentication.
        let mut bytes = URL_SAFE_NO_PAD.decode(&frame).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        assert_eq!(open(&key(7), &URL_SAFE_NO_PAD.encode(bytes)), None);
    }

    /// The real connection loop on a real socket: hello is answered under the key, and a frame sealed
    /// with another key closes the connection without a reply.
    #[test]
    fn phone_handshake_over_websocket() {
        tauri::async_runtime::block_on(async {
            let pairing = key(3);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tauri::async_runtime::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    // The desktop's commands need a window; hello is the same function either way.
                    let commands = |cmd: String, _args: Value| async move { if cmd == "hello" { Ok(hello()) } else { Err(format!("Unknown command {cmd}")) } };
                    tauri::async_runtime::spawn(connection(stream, pairing, commands, || {}));
                }
            });
            let connect = || async move {
                let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}/"), stream).await.unwrap().0
            };

            let mut phone = connect().await;
            phone.send(Message::text(seal(&pairing, br#"{"id":1,"cmd":"hello","args":{}}"#))).await.unwrap();
            let frame = match tokio::time::timeout(Duration::from_secs(5), phone.next()).await.unwrap() {
                Some(Ok(Message::Text(frame))) => frame,
                other => panic!("expected a sealed reply, got {other:?}"),
            };
            let reply: Value = serde_json::from_slice(&open(&pairing, frame.as_str()).unwrap()).unwrap();
            assert_eq!(reply["id"], 1);
            assert_eq!(reply["ok"], true);
            assert_eq!(reply["data"]["version"], env!("CARGO_PKG_VERSION"));
            assert!(reply["data"]["name"].as_str().is_some_and(|name| !name.is_empty()));

            let mut stranger = connect().await;
            stranger.send(Message::text(seal(&key(4), br#"{"id":1,"cmd":"hello","args":{}}"#))).await.unwrap();
            match tokio::time::timeout(Duration::from_secs(5), stranger.next()).await.unwrap() {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {}
                other => panic!("a wrong key must close the connection, got {other:?}"),
            }
        });
    }

    /// For scripts/remote-smoke.mjs against a running Neru with Remote on: prints the pairing link and
    /// writes the panel's QR code to NERU_REMOTE_QR (or the temp folder).
    /// `cargo test --lib remote::tests::live_pairing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_pairing() {
        let stored = read_stored();
        let key = stored_key(&stored).expect("Turn Neru Remote on in Settings → Phone first");
        let link = pairing_url(&lan_hosts(), stored.port, &key, &computer_name());
        let path = std::env::var_os("NERU_REMOTE_QR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("neru-pairing.svg"));
        fs::write(&path, qr_svg(&link)).unwrap();
        println!("PAIRING {link}
QR {}", path.display());
    }

    #[test]
    fn pairing_url_round_trips() {
        let hosts = vec!["192.168.1.20".to_string(), "10.0.0.5".to_string()];
        let text = pairing_url(&hosts, 47613, &key(9), "Diae's PC & co");
        assert!(text.starts_with("neru://pair?v=1&h=192.168.1.20,10.0.0.5&p=47613&k="));
        let url = reqwest::Url::parse(&text).unwrap();
        assert_eq!((url.scheme(), url.host_str()), ("neru", Some("pair")));
        let pairs: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["v"], "1");
        assert_eq!(pairs["h"].split(',').collect::<Vec<_>>(), ["192.168.1.20", "10.0.0.5"]);
        assert_eq!(pairs["p"].parse::<u16>().unwrap(), 47613);
        assert_eq!(URL_SAFE_NO_PAD.decode(&pairs["k"]).unwrap(), key(9));
        assert_eq!(pairs["n"], "Diae's PC & co");
    }
}
