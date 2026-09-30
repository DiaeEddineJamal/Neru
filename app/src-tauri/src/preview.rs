//! In-app preview of what the project builds. A project with a dev script gets its own dev server
//! (dependencies installed first when missing), and the URL is read from what the server prints,
//! so Vite, Next.js, Astro, CRA, SvelteKit and the rest all work without guessing ports. A plain
//! HTML/CSS/JS folder is served by a small static server built into Neru.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{AppState, workspace::project_root};

#[derive(Default)]
pub struct PreviewManager {
    child: Mutex<Option<tokio::process::Child>>,
    /// The static server's accept loop, when a plain HTML folder is being served.
    server: Mutex<Option<tokio::task::JoinHandle<()>>>,
    current: Mutex<Option<PreviewHint>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PreviewHint {
    pub command: String,
    pub url: String,
}

enum Plan {
    /// `npm run dev` and friends; `install` is set when node_modules is missing.
    Script { runner: &'static str, script: String, body: String, install: bool },
    /// Serve this folder as static files.
    Static(PathBuf),
}

fn package_runner(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    }
}

/// The folder holding the page to serve for a project without a dev script.
fn static_root(root: &Path) -> Option<PathBuf> {
    ["", "public", "src", "dist", "build", "site", "docs"]
        .iter()
        .map(|folder| root.join(folder))
        .find(|folder| folder.join("index.html").is_file())
        .or_else(|| {
            // Any single top-level .html file (e.g. game.html) still deserves a preview.
            fs::read_dir(root).ok()?.flatten().any(|entry| entry.path().extension().is_some_and(|ext| ext == "html")).then(|| root.to_path_buf())
        })
}

fn plan(root: &Path) -> Result<Plan, String> {
    let package = root.join("package.json");
    if package.is_file() {
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&package).map_err(|e| e.to_string())?)
            .map_err(|e| format!("package.json is not valid JSON: {e}"))?;
        for name in ["dev", "start", "serve", "preview"] {
            if let Some(body) = json["scripts"][name].as_str() {
                return Ok(Plan::Script {
                    runner: package_runner(root),
                    script: name.to_string(),
                    body: body.to_string(),
                    install: !root.join("node_modules").is_dir(),
                });
            }
        }
    }
    static_root(root).map(Plan::Static).ok_or_else(|| {
        "Nothing to preview yet: add an index.html, or a dev script in package.json".to_string()
    })
}

fn command_text(runner: &str, script: &str) -> String {
    if runner == "npm" { format!("npm run {script}") } else { format!("{runner} {script}") }
}

pub fn hint(root: &Path) -> Result<PreviewHint, String> {
    match plan(root)? {
        Plan::Script { runner, script, body, .. } => Ok(PreviewHint { command: command_text(runner, &script), url: guess_url(&body) }),
        Plan::Static(folder) => Ok(PreviewHint {
            command: format!("Neru static server for {}", folder.strip_prefix(root).ok().filter(|p| !p.as_os_str().is_empty()).map_or(".".into(), |p| p.display().to_string())),
            url: String::new(),
        }),
    }
}

fn guess_url(script: &str) -> String {
    if let Some(port) = script.split("--port").nth(1).and_then(|rest| rest.split_whitespace().next()).and_then(|value| value.trim_start_matches('=').parse::<u16>().ok()) {
        return format!("http://127.0.0.1:{port}");
    }
    let lower = script.to_lowercase();
    let port = if lower.contains("1420") {
        1420
    } else if lower.contains("vite") || lower.contains("svelte") {
        5173
    } else if lower.contains("astro") {
        4321
    } else if lower.contains("ng serve") {
        4200
    } else {
        3000
    };
    format!("http://127.0.0.1:{port}")
}

/// Pulls the first local URL out of a line of dev-server output ("Local: http://localhost:5173/").
fn url_in(line: &str) -> Option<String> {
    let clean: String = strip_ansi(line);
    let start = clean.find("http://")?;
    let url: String = clean[start..].chars().take_while(|c| !c.is_whitespace() && *c != ',' && *c != ')').collect();
    let parsed = reqwest::Url::parse(&url).ok()?;
    let host = parsed.host_str()?;
    if !matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]" | "::1") {
        return None;
    }
    parsed.port()?;
    Some(url.replace("0.0.0.0", "127.0.0.1").replace("//localhost", "//127.0.0.1").trim_end_matches('/').to_string())
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip "ESC [ ... letter".
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn shell(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut process = tokio::process::Command::new("powershell.exe");
        process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        process
    }
    #[cfg(not(windows))]
    {
        let mut process = tokio::process::Command::new("sh");
        process.args(["-lc", command]);
        process
    }
}

async fn reachable(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let (Some(host), Some(port)) = (parsed.host_str(), parsed.port_or_known_default()) else { return false };
    let host = if host == "localhost" { "127.0.0.1" } else { host };
    tokio::time::timeout(Duration::from_millis(600), tokio::net::TcpStream::connect((host, port)))
        .await
        .is_ok_and(|result| result.is_ok())
}

fn status(app: &AppHandle, text: &str) {
    let _ = app.emit("preview://status", text.to_string());
}

impl PreviewManager {
    pub fn current(&self) -> Option<PreviewHint> {
        self.current.lock().ok()?.clone()
    }

    pub async fn start(&self, app: &AppHandle, root: &Path) -> Result<PreviewHint, String> {
        // Reuse a preview that is still up rather than restarting it on every click.
        if let Some(current) = self.current() {
            if reachable(&current.url).await {
                return Ok(current);
            }
        }
        self.stop();
        let hint = match plan(root)? {
            Plan::Static(folder) => self.serve_static(folder).await?,
            Plan::Script { runner, script, body, install } => {
                if install {
                    status(app, &format!("Installing dependencies with {runner}…"));
                    let output = tokio::time::timeout(Duration::from_secs(600), shell(&format!("{runner} install")).current_dir(root).kill_on_drop(true).output())
                        .await
                        .map_err(|_| "Installing dependencies took over ten minutes".to_string())?
                        .map_err(|e| format!("Could not run {runner} install: {e}"))?;
                    if !output.status.success() {
                        let text = String::from_utf8_lossy(&output.stderr);
                        return Err(format!("{runner} install failed:\n{}", text.chars().rev().take(1500).collect::<String>().chars().rev().collect::<String>()));
                    }
                }
                let command = command_text(runner, &script);
                status(app, &format!("Starting {command}…"));
                self.run_script(&command, &body, root).await?
            }
        };
        *self.current.lock().map_err(|e| e.to_string())? = Some(hint.clone());
        Ok(hint)
    }

    async fn run_script(&self, command: &str, body: &str, root: &Path) -> Result<PreviewHint, String> {
        let mut child = shell(command)
            .current_dir(root)
            // Keeps frameworks from opening a browser tab of their own.
            .env("BROWSER", "none")
            .env("FORCE_COLOR", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Could not start the dev server: {e}"))?;
        let (sender, mut found) = tokio::sync::mpsc::unbounded_channel::<String>();
        let tail = std::sync::Arc::new(Mutex::new(String::new()));
        for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)].into_iter().flatten() {
            let sender = sender.clone();
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(url) = url_in(&line) {
                        let _ = sender.send(url);
                    }
                    if let Ok(mut tail) = tail.lock() {
                        tail.push_str(&line);
                        tail.push('\n');
                        if tail.len() > 4000 {
                            let cut = tail.len() - 3000;
                            let at = tail.char_indices().find(|(i, _)| *i >= cut).map_or(0, |(i, _)| i);
                            tail.drain(..at);
                        }
                    }
                }
            });
        }
        let fallback = guess_url(body);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        let mut url: Option<String> = None;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let log = tail.lock().map(|t| t.clone()).unwrap_or_default();
                return Err(format!("The dev server stopped ({status}).\n{}", log.trim()));
            }
            while let Ok(printed) = found.try_recv() {
                url.get_or_insert(printed);
            }
            let candidate = url.clone().unwrap_or_else(|| fallback.clone());
            if (url.is_some() || tokio::time::Instant::now() > deadline - Duration::from_secs(150)) && reachable(&candidate).await {
                *self.child.lock().map_err(|e| e.to_string())? = Some(child);
                return Ok(PreviewHint { command: command.to_string(), url: candidate });
            }
            if tokio::time::Instant::now() > deadline {
                return Err("The dev server did not come up within three minutes".into());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    async fn serve_static(&self, folder: PathBuf) -> Result<PreviewHint, String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Could not open a preview port: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let base = folder.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let base = base.clone();
                tokio::spawn(async move {
                    let _ = serve_one(stream, &base).await;
                });
            }
        });
        *self.server.lock().map_err(|e| e.to_string())? = Some(task);
        let page = if folder.join("index.html").is_file() {
            String::new()
        } else {
            fs::read_dir(&folder).ok().and_then(|entries| entries.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|ext| ext == "html")))
                .and_then(|p| p.file_name().map(|n| format!("/{}", n.to_string_lossy())))
                .unwrap_or_default()
        };
        Ok(PreviewHint { command: "Neru static server".into(), url: format!("http://127.0.0.1:{port}{page}") })
    }

    pub fn stop(&self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                // PowerShell wraps the server; kill the whole tree so the port is freed.
                #[cfg(windows)]
                if let Some(pid) = child.id() {
                    let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
                }
                let _ = child.start_kill();
            }
            *child = None;
        }
        if let Ok(mut server) = self.server.lock() {
            if let Some(task) = server.take() {
                task.abort();
            }
        }
        if let Ok(mut current) = self.current.lock() {
            *current = None;
        }
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Answers one HTTP request from `base`, never outside it. Files are read fresh each time, so
/// edits show on reload.
async fn serve_one(mut stream: tokio::net::TcpStream, base: &Path) -> std::io::Result<()> {
    let mut buffer = vec![0u8; 8192];
    let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer)).await.unwrap_or(Ok(0))?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let target = request.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let decoded = percent_decode(path);
    let mut file = base.to_path_buf();
    for part in decoded.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..") {
        file.push(part);
    }
    if file.is_dir() {
        file.push("index.html");
    }
    let (code, kind, body) = match tokio::fs::read(&file).await {
        Ok(body) => ("200 OK", content_type(&file), body),
        Err(_) => ("404 Not Found", "text/plain; charset=utf-8", format!("Not found: {decoded}").into_bytes()),
    };
    let head = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nCross-Origin-Resource-Policy: cross-origin\r\nCross-Origin-Embedder-Policy: credentialless\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Accepts loopback HTTP and public HTTPS pages for the preview pane.
pub fn allowed_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    if url.scheme() == "https" {
        return url.host_str().is_some();
    }
    if url.scheme() != "http" {
        return false;
    }
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

#[tauri::command]
pub fn preview_hint(state: State<'_, AppState>) -> Result<PreviewHint, String> {
    hint(&project_root(&state)?)
}

/// Whether the project has something to preview, and what is running now.
#[tauri::command]
pub fn preview_current(state: State<'_, AppState>) -> Option<PreviewHint> {
    state.preview.current()
}

#[tauri::command]
pub async fn preview_start(app: AppHandle) -> Result<PreviewHint, String> {
    let state = app.state::<AppState>();
    let root = project_root(&state)?;
    let hint = state.preview.start(&app, &root).await?;
    let _ = app.emit("preview://open", hint.url.clone());
    Ok(hint)
}

#[tauri::command]
pub fn preview_stop(state: State<'_, AppState>) {
    state.preview.stop();
}

pub fn open(app: &AppHandle, url: &str) -> Result<(), String> {
    if !allowed_url(url) {
        return Err("Preview URLs must be http://127.0.0.1, http://localhost, or https".into());
    }
    app.emit("preview://open", url.to_string()).map_err(|e| e.to_string())
}

// ---------- checking the page like a browser would ----------

/// Runs in the hidden window before the page's own scripts: records console errors, uncaught
/// exceptions, failed resources and failed requests, then reports once the page settles by
/// navigating to a URL Neru intercepts (the page gets no access to Neru itself).
const INSPECT_SCRIPT: &str = r#"
(() => {
  if (window.__neruInspect) return; window.__neruInspect = true;
  const errors = [];
  const push = (kind, text) => { if (errors.length < 40) errors.push(kind + ': ' + String(text).slice(0, 400)); };
  const original = { error: console.error, warn: console.warn };
  console.error = (...args) => { push('console.error', args.map(a => a && a.stack ? a.stack : (typeof a === 'object' ? JSON.stringify(a) : a)).join(' ')); original.error.apply(console, args); };
  console.warn = (...args) => { push('console.warn', args.join(' ')); original.warn.apply(console, args); };
  window.addEventListener('error', event => {
    const target = event.target;
    if (target && target !== window && (target.src || target.href)) push('failed to load', target.src || target.href);
    else push('uncaught', (event.message || '') + (event.filename ? ' at ' + event.filename + ':' + event.lineno : ''));
  }, true);
  window.addEventListener('unhandledrejection', event => push('unhandled rejection', event.reason && event.reason.stack || event.reason));
  const fetch0 = window.fetch;
  if (fetch0) window.fetch = (...args) => fetch0(...args).then(response => { if (!response.ok) push('request ' + response.status, response.url); return response; }, error => { push('request failed', args[0] + ' ' + error); throw error; });
  const report = () => {
    const body = document.body;
    const data = {
      url: location.href,
      title: document.title,
      text: body ? body.innerText.slice(0, 6000) : '',
      elements: document.getElementsByTagName('*').length,
      images: Array.from(document.images).filter(image => image.complete && image.naturalWidth === 0).map(image => image.src).slice(0, 10),
      errors,
    };
    location.href = 'https://neru-report.invalid/?d=' + encodeURIComponent(JSON.stringify(data));
  };
  const settle = () => setTimeout(report, 1800);
  if (document.readyState === 'complete') settle(); else window.addEventListener('load', settle, { once: true });
})();
"#;

/// Opens `url` in a hidden window, lets it run, and describes what a person would see and what
/// went wrong: title, visible text, console errors, failed loads.
pub async fn inspect(app: &AppHandle, url: &str) -> Result<String, String> {
    if !allowed_url(url) || !url.starts_with("http://") {
        return Err("check_preview only opens the local app (http://127.0.0.1 or localhost)".into());
    }
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let (sender, receiver) = tokio::sync::oneshot::channel::<String>();
    let sender = std::sync::Arc::new(Mutex::new(Some(sender)));
    let label = format!("inspect-{}", uuid::Uuid::new_v4().simple());
    let window = {
        let sender = sender.clone();
        tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::External(parsed))
            .visible(false)
            .inner_size(1280.0, 800.0)
            .initialization_script(INSPECT_SCRIPT)
            .on_navigation(move |target| {
                if target.host_str() != Some("neru-report.invalid") {
                    return true;
                }
                let data = target.query_pairs().find(|(key, _)| key == "d").map(|(_, value)| value.into_owned()).unwrap_or_default();
                if let Some(sender) = sender.lock().ok().and_then(|mut slot| slot.take()) {
                    let _ = sender.send(data);
                }
                false
            })
            .build()
            .map_err(|e| format!("Could not open a browser window: {e}"))?
    };
    let result = tokio::time::timeout(Duration::from_secs(25), receiver).await;
    let _ = window.destroy();
    let data = match result {
        Ok(Ok(data)) => data,
        _ => return Err(format!("{url} did not finish loading within 25 seconds. Is the server running? Start it with the preview first.")),
    };
    Ok(describe(&data, url))
}

/// Turns the page report into text for the model.
fn describe(data: &str, url: &str) -> String {
    let report: serde_json::Value = serde_json::from_str(data).unwrap_or_default();
    let errors: Vec<String> = report["errors"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_string)).collect();
    let broken: Vec<String> = report["images"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_string)).collect();
    let mut text = format!(
        "Checked {} in a browser.\nTitle: {}\nElements on the page: {}\n",
        report["url"].as_str().unwrap_or(url),
        report["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("(none)"),
        report["elements"].as_u64().unwrap_or(0)
    );
    if errors.is_empty() && broken.is_empty() {
        text.push_str("No errors: no console errors, uncaught exceptions or failed loads.\n");
    } else {
        text.push_str(&format!("Problems ({}):\n", errors.len() + broken.len()));
        for error in &errors {
            text.push_str(&format!("- {error}\n"));
        }
        for image in &broken {
            text.push_str(&format!("- broken image: {image}\n"));
        }
    }
    let visible = report["text"].as_str().unwrap_or("").trim();
    text.push_str(&format!("\nVisible text:\n{}", if visible.is_empty() { "(the page shows no text; it may be blank)" } else { visible }));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_a_port_and_rejects_file_urls() {
        assert_eq!(guess_url("vite --port 1420"), "http://127.0.0.1:1420");
        assert_eq!(guess_url("vite"), "http://127.0.0.1:5173");
        assert_eq!(guess_url("next dev"), "http://127.0.0.1:3000");
        assert!(allowed_url("http://127.0.0.1:5173"));
        assert!(allowed_url("https://docs.example.com/guide"));
        assert!(!allowed_url("file:///C:/Windows"));
        assert!(!allowed_url("http://192.168.0.8"));
    }

    #[test]
    fn reads_urls_from_dev_server_output() {
        assert_eq!(url_in("  \x1b[32m➜\x1b[39m  Local:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/\x1b[39m").as_deref(), Some("http://127.0.0.1:5173"));
        assert_eq!(url_in("   - Local:        http://localhost:3000").as_deref(), Some("http://127.0.0.1:3000"));
        assert_eq!(url_in("ready on http://0.0.0.0:4321, press h").as_deref(), Some("http://127.0.0.1:4321"));
        assert_eq!(url_in("Network: http://192.168.1.4:5173/"), None);
        assert_eq!(url_in("see https://nextjs.org/docs"), None);
    }

    #[test]
    fn page_reports_read_well() {
        let text = describe(r#"{"url":"http://127.0.0.1:5173/","title":"Todo","text":"Add task","elements":42,"images":[],"errors":["uncaught: x is not defined at app.js:3"]}"#, "http://127.0.0.1:5173");
        assert!(text.contains("Title: Todo") && text.contains("Problems (1)") && text.contains("x is not defined") && text.contains("Add task"));
        assert!(describe(r#"{"title":"","text":"","errors":[]}"#, "u").contains("No errors"));
    }

    #[test]
    fn decodes_paths() {
        assert_eq!(percent_decode("/my%20page.html"), "/my page.html");
    }
}
