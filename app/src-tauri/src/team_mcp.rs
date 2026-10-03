//! Neru's browser as an MCP server for Team members. Claude Code, Codex and OpenCode connect to it
//! over streamable HTTP on 127.0.0.1 with a per-run bearer token, and get three tools: start the
//! project's dev server, open a page in Neru's browser pane (the user watches), and read a page's
//! title, text and errors. The URL path names the task, so the tools work in its project.
use std::{
    path::PathBuf,
    sync::OnceLock,
};

use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::AppState;

/// The server's port and token, once started.
static SERVER: OnceLock<(u16, String)> = OnceLock::new();

/// Starts the server once and returns its base URL (without the task) and token.
pub async fn ensure(app: &AppHandle) -> Result<(String, String), String> {
    if let Some((port, token)) = SERVER.get() {
        return Ok((format!("http://127.0.0.1:{port}/mcp"), token.clone()));
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    if SERVER.set((port, token.clone())).is_err() {
        // Another turn started it first; use that one.
        return Box::pin(ensure(app)).await;
    }
    let app = app.clone();
    let expected = token.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (app, expected) = (app.clone(), expected.clone());
            tauri::async_runtime::spawn(async move { let _ = serve(stream, &app, &expected).await; });
        }
    });
    Ok((format!("http://127.0.0.1:{port}/mcp"), token))
}

/// One HTTP request: the header block, then a body of Content-Length bytes.
async fn serve(mut stream: TcpStream, app: &AppHandle, token: &str) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if buffer.len() > 64 * 1024 {
            return reply(&mut stream, 431, "", "").await;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.lines();
    let request = lines.next().unwrap_or_default().to_string();
    let header = |name: &str| head.lines().find_map(|line| line.split_once(':').filter(|(key, _)| key.trim().eq_ignore_ascii_case(name)).map(|(_, value)| value.trim().to_string()));
    let length: usize = header("content-length").and_then(|value| value.parse().ok()).unwrap_or(0).min(4 * 1024 * 1024);
    while buffer.len() < head_end + length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    if header("authorization").as_deref() != Some(&format!("Bearer {token}")) {
        return reply(&mut stream, 401, "application/json", r#"{"error":"unauthorized"}"#).await;
    }
    let mut parts = request.split_whitespace();
    let (method, path) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    let Some(task) = path.strip_prefix("/mcp/").map(|task| task.trim_end_matches('/').to_string()) else {
        return reply(&mut stream, 404, "", "").await;
    };
    if method != "POST" {
        // No server-initiated stream: clients fall back to plain request/response.
        return reply(&mut stream, 405, "", "").await;
    }
    let body = &buffer[head_end..(head_end + length).min(buffer.len())];
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return reply(&mut stream, 400, "application/json", &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } }).to_string()).await;
    };
    let answers: Vec<Value> = match &message {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for item in batch {
                if let Some(answer) = handle(app, &task, item).await {
                    out.push(answer);
                }
            }
            out
        }
        single => handle(app, &task, single).await.into_iter().collect(),
    };
    match (answers.len(), message.is_array()) {
        (0, _) => reply(&mut stream, 202, "", "").await,
        (_, true) => reply(&mut stream, 200, "application/json", &Value::Array(answers).to_string()).await,
        _ => reply(&mut stream, 200, "application/json", &answers[0].to_string()).await,
    }
}

async fn reply(stream: &mut TcpStream, status: u16, kind: &str, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let kind = if kind.is_empty() { String::new() } else { format!("Content-Type: {kind}\r\n") };
    let text = format!("HTTP/1.1 {status} {reason}\r\n{kind}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(text.as_bytes()).await?;
    stream.shutdown().await
}

/// The tools, in MCP's `tools/list` shape.
pub fn tools() -> Value {
    json!([
        { "name": "preview_start", "description": "Start the project's dev server (npm run dev and the like) and open it in Neru's browser pane, where the user can see it. Returns its URL.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "browser_open", "description": "Open a page in Neru's browser pane so the user sees it. http://localhost, http://127.0.0.1 or https URLs.", "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] } },
        { "name": "browser_read", "description": "Load a local page (http://localhost or http://127.0.0.1) in a hidden browser and return its title, visible text, console errors and broken images.", "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] } },
    ])
}

/// Answers one JSON-RPC message; notifications get no answer.
async fn handle(app: &AppHandle, task: &str, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "neru", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Neru's browser: start the dev server, show pages to the user, and read a page's text and errors."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let args = &message["params"]["arguments"];
            let outcome = call(app, task, message["params"]["name"].as_str().unwrap_or_default(), args).await;
            Ok(match outcome {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                Err(error) => json!({ "content": [{ "type": "text", "text": error }], "isError": true }),
            })
        }
        _ => Err(json!({ "code": -32601, "message": format!("Unknown method {method}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

async fn call(app: &AppHandle, task: &str, name: &str, args: &Value) -> Result<String, String> {
    let url = args["url"].as_str().unwrap_or_default().trim().to_string();
    match name {
        "preview_start" => {
            let root = task_root(task)?;
            let hint = app.state::<AppState>().preview.start(app, &root).await?;
            crate::preview::open(app, &hint.url)?;
            Ok(format!("The dev server is running at {} (started with `{}`) and is open in Neru's browser.", hint.url, hint.command))
        }
        "browser_open" => {
            crate::preview::open(app, &url)?;
            Ok(format!("Opened {url} in Neru's browser pane."))
        }
        "browser_read" => crate::preview::inspect(app, &url).await,
        _ => Err(format!("Unknown tool {name}")),
    }
}

/// The task's project folder.
fn task_root(task: &str) -> Result<PathBuf, String> {
    let shared = crate::team::load(task)?;
    let task = crate::team::lock(&shared)?;
    if task.project_path.is_empty() {
        return Err("This task has no project folder".into());
    }
    Ok(PathBuf::from(&task.project_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_have_schemas() {
        let tools = tools();
        let names: Vec<&str> = tools.as_array().unwrap().iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert_eq!(names, ["preview_start", "browser_open", "browser_read"]);
        assert!(tools.as_array().unwrap().iter().all(|tool| tool["inputSchema"]["type"] == "object"));
    }
}
