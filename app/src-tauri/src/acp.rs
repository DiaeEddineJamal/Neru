//! `neru acp`: Neru as an Agent Client Protocol agent over stdio, so editors such as Zed and
//! JetBrains IDEs can drive it. Newline-delimited JSON-RPC 2.0:
//!
//! - `initialize` → protocol version 1 and what Neru accepts in prompts.
//! - `session/new` {cwd} → opens that folder as the project and starts a session.
//! - `session/prompt` {sessionId, prompt} → runs the agent; text, thoughts and tool calls stream to
//!   the client as `session/update` notifications; edits and commands that need approval become
//!   `session/request_permission` requests; the reply ends with a `stopReason`.
//! - `session/cancel` {sessionId} → stops the running turn (`stopReason: "cancelled"`).
//!
//! The protocol core (`serve_with`) is independent of the agent behind it (`Backend`), so the
//! handshake is tested with a scripted client and a fake agent.

use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use serde_json::{Value, json};

pub const PROTOCOL_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    Cancelled,
}

impl StopReason {
    fn name(self) -> &'static str {
        match self {
            StopReason::EndTurn => "end_turn",
            StopReason::Cancelled => "cancelled",
        }
    }
}

/// What a running turn can tell or ask the client.
pub trait Client {
    /// A `session/update` payload (`{"sessionUpdate": …}`).
    fn update(&self, update: Value);
    /// Asks to run a tool call; true when the user allowed it.
    fn permission(&self, tool_call: Value) -> bool;
}

/// The agent behind the protocol.
pub trait Backend: Send + Sync + 'static {
    fn new_session(&self, cwd: &str) -> Result<String, String>;
    fn prompt(&self, session: &str, text: &str, client: &dyn Client) -> Result<StopReason, String>;
    fn cancel(&self, session: &str);
}

/// The connection: one writer, and the client's answers to our own requests by id.
struct Connection {
    out: Mutex<Box<dyn Write + Send>>,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Value>>>,
    next_id: AtomicU64,
}

impl Connection {
    fn send(&self, message: Value) {
        // One write per message, so a line never interleaves with another thread's.
        let line = format!("{message}\n");
        if let Ok(mut out) = self.out.lock() {
            let _ = out.write_all(line.as_bytes());
            let _ = out.flush();
        }
    }

    fn reply(&self, id: &Value, result: Result<Value, (i64, String)>) {
        self.send(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        });
    }

    /// Sends a request to the client and waits for its answer; None when the connection closed.
    fn request(&self, method: &str, params: Value) -> Option<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        self.waiting.lock().ok()?.insert(id, sender);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        receiver.recv().ok()
    }
}

struct Turn<'a> {
    connection: &'a Connection,
    session: String,
}

impl Client for Turn<'_> {
    fn update(&self, update: Value) {
        self.connection.send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": self.session, "update": update}}));
    }

    fn permission(&self, tool_call: Value) -> bool {
        let options = json!([
            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
            {"optionId": "reject", "name": "Reject", "kind": "reject_once"}
        ]);
        let Some(answer) = self.connection.request("session/request_permission", json!({"sessionId": self.session, "toolCall": tool_call, "options": options})) else {
            return false;
        };
        let outcome = &answer["result"]["outcome"];
        outcome["outcome"] == "selected" && outcome["optionId"] == "allow"
    }
}

/// The text of a prompt's content blocks: text as is, resources and links as references.
pub fn prompt_text(blocks: &Value) -> String {
    let mut parts = Vec::new();
    for block in blocks.as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => parts.push(block["text"].as_str().unwrap_or("").to_string()),
            Some("resource") => {
                let resource = &block["resource"];
                let uri = resource["uri"].as_str().unwrap_or("");
                match resource["text"].as_str() {
                    Some(text) => parts.push(format!("<context uri=\"{uri}\">\n{text}\n</context>")),
                    None => parts.push(format!("(attached: {uri})")),
                }
            }
            Some("resource_link") => parts.push(format!("(see {})", block["uri"].as_str().unwrap_or(""))),
            _ => {}
        }
    }
    parts.join("\n\n")
}

/// Serves the protocol on `input`/`output` until the client closes the input.
pub fn serve_with<B: Backend>(backend: Arc<B>, input: impl BufRead, output: impl Write + Send + 'static) {
    let connection = Arc::new(Connection { out: Mutex::new(Box::new(output)), waiting: Mutex::new(HashMap::new()), next_id: AtomicU64::new(1) });
    let sessions: Arc<Mutex<HashSet<String>>> = Arc::default();
    let mut turns = Vec::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                connection.reply(&Value::Null, Err((-32700, format!("Parse error: {error}"))));
                continue;
            }
        };
        let id = message.get("id").cloned();
        let Some(method) = message["method"].as_str().map(str::to_string) else {
            // The client's answer to one of our requests.
            if let Some(sender) = id.as_ref().and_then(Value::as_u64).and_then(|id| connection.waiting.lock().ok()?.remove(&id)) {
                let _ = sender.send(message);
            }
            continue;
        };
        let params = message["params"].clone();
        let session_of = |params: &Value| params["sessionId"].as_str().unwrap_or("").to_string();
        let known = |session: &str| sessions.lock().is_ok_and(|sessions| sessions.contains(session));
        let result: Result<Value, (i64, String)> = match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": {"image": false, "audio": false, "embeddedContext": true},
                    "mcpCapabilities": {"http": false, "sse": false}
                },
                "agentInfo": {"name": "neru", "title": "Neru", "version": env!("CARGO_PKG_VERSION")},
                "authMethods": []
            })),
            "authenticate" => Ok(json!({})),
            "session/new" => {
                let cwd = params["cwd"].as_str().unwrap_or("");
                backend.new_session(cwd).map(|session| {
                    if let Ok(mut sessions) = sessions.lock() {
                        sessions.insert(session.clone());
                    }
                    json!({"sessionId": session})
                }).map_err(|error| (-32603, error))
            }
            "session/cancel" => {
                let session = session_of(&params);
                if known(&session) {
                    backend.cancel(&session);
                }
                continue;
            }
            "session/prompt" => {
                let session = session_of(&params);
                if !known(&session) {
                    Err((-32602, format!("Unknown session {session}")))
                } else {
                    // A turn runs on its own thread, so cancels and permission answers keep flowing.
                    let (backend, connection, text, id) = (backend.clone(), connection.clone(), prompt_text(&params["prompt"]), id.clone().unwrap_or(Value::Null));
                    turns.push(std::thread::spawn(move || {
                        let turn = Turn { connection: &connection, session: session.clone() };
                        let result = backend.prompt(&session, &text, &turn).map(|reason| json!({"stopReason": reason.name()})).map_err(|error| (-32603, error));
                        connection.reply(&id, result);
                    }));
                    continue;
                }
            }
            _ => Err((-32601, format!("Method not found: {method}"))),
        };
        // Notifications get no answer.
        if let Some(id) = id {
            connection.reply(&id, result);
        }
    }
    // The client is gone: unblock turns waiting on it, then let them finish.
    if let Ok(mut waiting) = connection.waiting.lock() {
        waiting.clear();
    }
    for session in sessions.lock().map(|sessions| sessions.clone()).unwrap_or_default() {
        backend.cancel(&session);
    }
    for turn in turns {
        let _ = turn.join();
    }
}

// ---------------------------------------------------------------------------------------------
// Neru's agent behind the protocol

struct Neru {
    app: tauri::AppHandle,
    events: Mutex<mpsc::Receiver<Value>>,
    cancelled: Mutex<HashSet<String>>,
}

/// The ACP tool kind for a Neru activity label.
fn tool_kind(label: &str) -> &'static str {
    let lower = label.to_lowercase();
    if lower.starts_with("read") || lower.starts_with("list") || lower.starts_with("open") {
        "read"
    } else if lower.contains("edit") || lower.contains("write") || lower.contains("creat") {
        "edit"
    } else if lower.contains("delete") {
        "delete"
    } else if lower.contains("search") || lower.contains("find") {
        "search"
    } else if lower.contains("ran") || lower.contains("run") || lower.contains("command") {
        "execute"
    } else if lower.contains("fetch") || lower.contains("web") {
        "fetch"
    } else {
        "other"
    }
}

/// A Neru agent event as an ACP `session/update`, if the client should see it.
pub fn event_update(event: &Value, started: &mut HashSet<String>) -> Option<Value> {
    match event["type"].as_str()? {
        "delta" => Some(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": event["text"].as_str()?}})),
        "reasoning" => Some(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": event["text"].as_str().filter(|text| !text.is_empty())?}})),
        "tool" => {
            let id = event["id"].as_str()?.to_string();
            let label = event["label"].as_str().unwrap_or("tool");
            let status = match event["status"].as_str() {
                Some("running") => "in_progress",
                Some("error") => "failed",
                Some("pending") => "pending",
                _ => "completed",
            };
            if started.insert(id.clone()) {
                Some(json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": label, "kind": tool_kind(label), "status": status}))
            } else {
                Some(json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": status}))
            }
        }
        _ => None,
    }
}

impl Backend for Neru {
    fn new_session(&self, cwd: &str) -> Result<String, String> {
        use tauri::Manager;
        if !cwd.is_empty() {
            crate::workspace::open_project(cwd.to_string(), self.app.state())?;
        }
        Ok(crate::sessions::create_session(Some(false), self.app.state())?.session.id)
    }

    fn prompt(&self, session: &str, text: &str, client: &dyn Client) -> Result<StopReason, String> {
        use tauri::Manager;
        crate::sessions::select_session(session.to_string(), self.app.state())?;
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.remove(session);
        }
        let events = self.events.lock().map_err(|e| e.to_string())?;
        while events.try_recv().is_ok() {}
        let mut started = HashSet::new();
        let mut prompt = text.to_string();
        loop {
            let (done_tx, done_rx) = mpsc::channel();
            let (app, request, id) = (self.app.clone(), std::mem::take(&mut prompt), session.to_string());
            tauri::async_runtime::spawn(async move {
                let _ = done_tx.send(crate::agent::ai_chat(request, vec![], "manual".into(), Some(true), None, None, None, Some(id), None, Some("code".into()), None, app).await);
            });
            let mut streamed = false;
            let response = loop {
                let finished = done_rx.recv_timeout(Duration::from_millis(50));
                while let Ok(event) = events.try_recv() {
                    if event["sessionId"].as_str() != Some(session) {
                        continue;
                    }
                    if let Some(update) = event_update(&event, &mut started) {
                        streamed |= update["sessionUpdate"] == "agent_message_chunk";
                        client.update(update);
                    }
                }
                match finished {
                    Ok(response) => break response,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break Err("The agent stopped unexpectedly".into()),
                }
            };
            if self.cancelled.lock().is_ok_and(|cancelled| cancelled.contains(session)) {
                let _ = crate::workspace::reject_pending(self.app.state());
                return Ok(StopReason::Cancelled);
            }
            let response = response?;
            if !streamed && !response.content.trim().is_empty() {
                client.update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": response.content}}));
            }
            let Some(pending) = response.pending else {
                return Ok(StopReason::EndTurn);
            };
            if matches!(pending.kind.as_str(), "plan" | "question") {
                let _ = crate::workspace::deny_pending(&self.app.state(), "This client cannot show plans or questions; decide yourself and continue, or say what you need in your reply.");
                continue;
            }
            let call_id = format!("approval-{}", uuid::Uuid::new_v4());
            let tool_call = json!({
                "toolCallId": call_id,
                "title": pending.label,
                "kind": if pending.kind == "task" { "execute" } else if pending.kind == "delete" { "delete" } else { "edit" },
                "status": "pending",
                "rawInput": {"kind": pending.kind, "label": pending.label, "diff": pending.diff},
            });
            client.update(json!({"sessionUpdate": "tool_call", "toolCallId": call_id, "title": pending.label, "kind": tool_call["kind"], "status": "pending"}));
            if !client.permission(tool_call) {
                let _ = crate::workspace::reject_pending(self.app.state());
                client.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call_id, "status": "failed"}));
                if self.cancelled.lock().is_ok_and(|cancelled| cancelled.contains(session)) {
                    return Ok(StopReason::Cancelled);
                }
                continue;
            }
            let outcome = if pending.kind == "task" {
                tauri::async_runtime::block_on(crate::agent::run_pending_task(self.app.clone()))
            } else {
                crate::workspace::apply_pending(self.app.state())
            };
            if outcome.is_err() {
                let _ = crate::workspace::reject_pending(self.app.state());
            }
            client.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call_id, "status": if outcome.is_ok() { "completed" } else { "failed" }}));
        }
    }

    fn cancel(&self, session: &str) {
        use tauri::Manager;
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.insert(session.to_string());
        }
        let _ = crate::agent::stop_chat(Some(session.to_string()), self.app.state());
    }
}

/// `neru acp`: serves stdin/stdout until the client hangs up.
pub fn serve(app: &tauri::AppHandle) -> i32 {
    use tauri::Listener;
    let (sender, events) = mpsc::channel();
    app.listen("agent://event", move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            let _ = sender.send(value);
        }
    });
    tauri::async_runtime::spawn(crate::mcp::start_enabled(app.clone()));
    let backend = Arc::new(Neru { app: app.clone(), events: Mutex::new(events), cancelled: Mutex::default() });
    serve_with(backend, std::io::stdin().lock(), std::io::stdout());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake agent: streams two chunks and a tool call, asks once for permission, and reports
    /// whether it was allowed; "wait" blocks until cancelled.
    #[derive(Default)]
    struct Fake {
        cancelled: Mutex<bool>,
    }

    impl Backend for Fake {
        fn new_session(&self, cwd: &str) -> Result<String, String> {
            if cwd.is_empty() { Err("cwd is required".into()) } else { Ok("s1".into()) }
        }

        fn prompt(&self, _session: &str, text: &str, client: &dyn Client) -> Result<StopReason, String> {
            if text == "wait" {
                for _ in 0..200 {
                    if *self.cancelled.lock().unwrap() {
                        return Ok(StopReason::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                return Ok(StopReason::EndTurn);
            }
            client.update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": format!("echo: {text}")}}));
            client.update(json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Run cargo test", "kind": "execute", "status": "pending"}));
            let allowed = client.permission(json!({"toolCallId": "t1", "title": "Run cargo test"}));
            client.update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": if allowed { "ran" } else { "skipped" }}}));
            Ok(StopReason::EndTurn)
        }

        fn cancel(&self, _session: &str) {
            *self.cancelled.lock().unwrap() = true;
        }
    }

    /// A pipe the test writes the client's side into, line by line, as answers come back.
    struct Pipe(mpsc::Receiver<String>);

    impl std::io::Read for Pipe {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            unreachable!("read through BufRead")
        }
    }

    impl BufRead for Pipe {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            unreachable!()
        }
        fn consume(&mut self, _: usize) {}
        fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
            match self.0.recv() {
                Ok(line) => {
                    buf.push_str(&line);
                    buf.push('\n');
                    Ok(line.len() + 1)
                }
                Err(_) => Ok(0),
            }
        }
    }

    #[derive(Clone)]
    struct Sink(mpsc::Sender<Value>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            for line in String::from_utf8_lossy(buf).lines().filter(|line| !line.trim().is_empty()) {
                let _ = self.0.send(serde_json::from_str(line).expect("the agent writes JSON lines"));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Runs a scripted client against the fake agent.
    fn client() -> (mpsc::Sender<String>, mpsc::Receiver<Value>, std::thread::JoinHandle<()>) {
        let (to_agent, input) = mpsc::channel();
        let (output, from_agent) = mpsc::channel();
        let server = std::thread::spawn(move || serve_with(Arc::new(Fake::default()), Pipe(input), Sink(output)));
        (to_agent, from_agent, server)
    }

    fn next(from: &mpsc::Receiver<Value>) -> Value {
        from.recv_timeout(Duration::from_secs(5)).expect("the agent answers")
    }

    #[test]
    fn handshake_prompt_and_permission() {
        let (to, from, server) = client();
        to.send(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false}}}}"#.into()).unwrap();
        let init = next(&from);
        assert_eq!(init["id"], 0);
        assert_eq!(init["result"]["protocolVersion"], 1);
        assert_eq!(init["result"]["agentCapabilities"]["promptCapabilities"]["embeddedContext"], true);

        to.send(r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/work/app","mcpServers":[]}}"#.into()).unwrap();
        assert_eq!(next(&from)["result"]["sessionId"], "s1");

        to.send(r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"hello"}]}}"#.into()).unwrap();
        let chunk = next(&from);
        assert_eq!(chunk["method"], "session/update");
        assert_eq!(chunk["params"]["sessionId"], "s1");
        assert_eq!(chunk["params"]["update"]["content"]["text"], "echo: hello");
        assert_eq!(next(&from)["params"]["update"]["sessionUpdate"], "tool_call");
        let ask = next(&from);
        assert_eq!(ask["method"], "session/request_permission");
        assert_eq!(ask["params"]["toolCall"]["toolCallId"], "t1");
        assert_eq!(ask["params"]["options"][0]["kind"], "allow_once");
        to.send(json!({"jsonrpc": "2.0", "id": ask["id"], "result": {"outcome": {"outcome": "selected", "optionId": "allow"}}}).to_string()).unwrap();
        assert_eq!(next(&from)["params"]["update"]["content"]["text"], "ran");
        let done = next(&from);
        assert_eq!((done["id"].clone(), done["result"]["stopReason"].clone()), (json!(2), json!("end_turn")));

        to.send(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#.into()).unwrap();
        assert_eq!(next(&from)["error"]["code"], -32601);
        to.send(r#"{"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"other","prompt":[]}}"#.into()).unwrap();
        assert_eq!(next(&from)["error"]["code"], -32602);
        drop(to);
        server.join().unwrap();
    }

    #[test]
    fn cancel_stops_a_running_turn_and_rejection_is_reported() {
        let (to, from, server) = client();
        to.send(r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/w"}}"#.into()).unwrap();
        next(&from);
        to.send(r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"x"}]}}"#.into()).unwrap();
        next(&from);
        next(&from);
        let ask = next(&from);
        to.send(json!({"jsonrpc": "2.0", "id": ask["id"], "result": {"outcome": {"outcome": "cancelled"}}}).to_string()).unwrap();
        assert_eq!(next(&from)["params"]["update"]["content"]["text"], "skipped");
        next(&from);
        to.send(r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"s1","prompt":[{"type":"text","text":"wait"}]}}"#.into()).unwrap();
        to.send(r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"s1"}}"#.into()).unwrap();
        let done = next(&from);
        assert_eq!((done["id"].clone(), done["result"]["stopReason"].clone()), (json!(3), json!("cancelled")));
        drop(to);
        server.join().unwrap();
    }

    #[test]
    fn prompt_blocks_and_events_translate() {
        let text = prompt_text(&json!([
            {"type": "text", "text": "Fix this"},
            {"type": "resource", "resource": {"uri": "file:///a.rs", "text": "fn main() {}"}},
            {"type": "resource_link", "uri": "file:///b.rs", "name": "b.rs"},
            {"type": "image", "data": "…"}
        ]));
        assert_eq!(text, "Fix this\n\n<context uri=\"file:///a.rs\">\nfn main() {}\n</context>\n\n(see file:///b.rs)");
        let mut started = HashSet::new();
        let first = event_update(&json!({"type": "tool", "id": "c1", "label": "Read src/main.rs", "status": "running"}), &mut started).unwrap();
        assert_eq!((first["sessionUpdate"].as_str(), first["kind"].as_str(), first["status"].as_str()), (Some("tool_call"), Some("read"), Some("in_progress")));
        let second = event_update(&json!({"type": "tool", "id": "c1", "label": "Read src/main.rs", "status": "done"}), &mut started).unwrap();
        assert_eq!((second["sessionUpdate"].as_str(), second["status"].as_str()), (Some("tool_call_update"), Some("completed")));
        assert_eq!(event_update(&json!({"type": "delta", "text": "hi"}), &mut started).unwrap()["content"]["text"], "hi");
        assert!(event_update(&json!({"type": "context"}), &mut started).is_none());
    }
}
