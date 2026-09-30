//! Loopback proxy behind the in-app browser. Local pages (dev servers and Neru's static server)
//! are loaded through it so Neru can inject its page bridge into their HTML: element picking,
//! annotations, console and network capture, and navigation state. Everything else passes
//! through untouched, including WebSocket upgrades (hot reload), so dev servers behave normally.

use std::{collections::HashMap, io, sync::Mutex, time::Duration};

use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub const BRIDGE_PATH: &str = "/__neru/bridge.js";
const BRIDGE_JS: &str = include_str!("preview_bridge.js");
const BRIDGE_TAG: &str = "<script src=\"/__neru/bridge.js\" data-neru=\"bridge\"></script>";
const MAX_HEAD: usize = 64 * 1024;
const MAX_HTML: usize = 24 * 1024 * 1024;

/// Where the in-app browser should load a URL from, and how to translate back.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// URL to give the frame.
    pub frame_url: String,
    /// The address the person sees.
    pub real_url: String,
    /// Origin the frame is served from, when proxied.
    pub proxy_origin: Option<String>,
    /// Origin of the real server, when proxied.
    pub real_origin: Option<String>,
    /// Whether the page bridge will be present.
    pub bridged: bool,
}

#[derive(Clone)]
struct Upstream {
    /// Host as the person typed it: `localhost`, `127.0.0.1` or `[::1]`.
    host: String,
    port: u16,
    proxy_port: u16,
}

impl Upstream {
    fn real_origin(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
    fn proxy_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.proxy_port)
    }
    /// Every spelling of the real server that may appear in a redirect.
    fn spellings(&self) -> Vec<String> {
        ["localhost", "127.0.0.1", "[::1]"].iter().map(|host| format!("http://{host}:{}", self.port)).collect()
    }
    fn addresses(&self) -> Vec<&'static str> {
        match self.host.as_str() {
            "localhost" => vec!["127.0.0.1", "::1"],
            "[::1]" => vec!["::1"],
            _ => vec!["127.0.0.1"],
        }
    }
}

#[derive(Default)]
pub struct ProxyManager {
    /// `host:port` of the real server -> the proxy port fronting it.
    routes: Mutex<HashMap<String, (u16, JoinHandle<()>)>>,
}

impl ProxyManager {
    pub async fn route(&self, url: &str) -> Result<Route, String> {
        let parsed = reqwest::Url::parse(url).map_err(|_| "That is not a valid address".to_string())?;
        if parsed.scheme() == "https" {
            return Ok(Route { frame_url: url.to_string(), real_url: url.to_string(), proxy_origin: None, real_origin: None, bridged: false });
        }
        if parsed.scheme() != "http" || !matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]" | "::1")) {
            return Err("The preview opens http://localhost, http://127.0.0.1 or https pages".into());
        }
        let host = match parsed.host_str() {
            Some("::1") => "[::1]".to_string(),
            Some(host) => host.to_string(),
            None => return Err("That address has no host".into()),
        };
        let port = parsed.port_or_known_default().unwrap_or(80);
        let key = format!("{host}:{port}");
        let existing = self.routes.lock().map_err(|e| e.to_string())?.get(&key).map(|(port, _)| *port);
        let proxy_port = match existing {
            Some(port) => port,
            None => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Could not open a preview port: {e}"))?;
                let proxy_port = listener.local_addr().map_err(|e| e.to_string())?.port();
                let upstream = Upstream { host: host.clone(), port, proxy_port };
                let task = tokio::spawn(accept_loop(listener, upstream));
                self.routes.lock().map_err(|e| e.to_string())?.insert(key, (proxy_port, task));
                proxy_port
            }
        };
        let mut framed = parsed.clone();
        framed.set_host(Some("127.0.0.1")).map_err(|e| e.to_string())?;
        framed.set_port(Some(proxy_port)).map_err(|_| "Could not build the preview address".to_string())?;
        Ok(Route {
            frame_url: framed.to_string(),
            real_url: url.to_string(),
            proxy_origin: Some(format!("http://127.0.0.1:{proxy_port}")),
            real_origin: Some(format!("http://{host}:{port}")),
            bridged: true,
        })
    }

    pub fn shutdown(&self) {
        if let Ok(mut routes) = self.routes.lock() {
            for (_, (_, task)) in routes.drain() {
                task.abort();
            }
        }
    }
}

async fn accept_loop(listener: TcpListener, upstream: Upstream) {
    while let Ok((stream, _)) = listener.accept().await {
        let upstream = upstream.clone();
        tokio::spawn(async move {
            let _ = handle(stream, upstream).await;
        });
    }
}

struct Head {
    first: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn parse_head(bytes: &[u8]) -> Option<Head> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split("\r\n");
    let first = lines.next()?.to_string();
    if first.is_empty() {
        return None;
    }
    let headers = lines.filter_map(|line| line.split_once(':').map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))).collect();
    Some(Head { first, headers })
}

/// Reads up to the blank line ending an HTTP head; returns the head and any bytes read past it.
async fn read_head<R: AsyncReadExt + Unpin>(stream: &mut R) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(end) = find(&buffer, b"\r\n\r\n") {
            let rest = buffer.split_off(end + 4);
            buffer.truncate(end);
            return Ok(Some((buffer, rest)));
        }
        if buffer.len() > MAX_HEAD {
            return Ok(None);
        }
        let read = tokio::time::timeout(Duration::from_secs(30), stream.read(&mut chunk)).await.map_err(|_| io::ErrorKind::TimedOut)??;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

async fn respond<W: AsyncWriteExt + Unpin>(stream: &mut W, status: &str, kind: &str, body: &[u8]) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nCross-Origin-Resource-Policy: cross-origin\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unreachable_page(upstream: &Upstream) -> String {
    let address = escape_html(&upstream.real_origin());
    format!(
        "<!doctype html><meta charset=utf-8><title>Cannot reach {address}</title><style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#151515;color:#f2f0e9;font:15px/1.6 Inter,system-ui,sans-serif}}main{{max-width:420px;padding:32px;text-align:center}}h1{{margin:0 0 8px;font:600 20px Inter,system-ui,sans-serif}}p{{margin:0;color:#b3b3aa}}code{{color:#8ea291}}</style><main><h1>Nothing is running at <code>{address}</code></h1><p>Start the app with the Run preview button, or check that the server is listening on that port, then reload.</p></main>"
    )
}

async fn connect(upstream: &Upstream) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::ConnectionRefused, "no address");
    for address in upstream.addresses() {
        match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect((address, upstream.port))).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = error,
            Err(_) => last = io::Error::new(io::ErrorKind::TimedOut, "timed out"),
        }
    }
    Err(last)
}

fn request_head(head: &Head, upstream: &Upstream, upgrade: bool) -> String {
    let (proxy, real) = (upstream.proxy_origin(), upstream.real_origin());
    let mut out = format!("{}\r\n", head.first);
    for (key, value) in &head.headers {
        match key.to_ascii_lowercase().as_str() {
            "host" | "accept-encoding" | "if-none-match" | "if-modified-since" | "proxy-connection" | "keep-alive" => {}
            "connection" if !upgrade => {}
            "origin" | "referer" => out.push_str(&format!("{key}: {}\r\n", value.replace(&proxy, &real))),
            _ => out.push_str(&format!("{key}: {value}\r\n")),
        }
    }
    out.push_str(&format!("Host: {}:{}\r\nAccept-Encoding: identity\r\n", upstream.host, upstream.port));
    if !upgrade {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    out
}

/// Response headers for the frame: framing and embedding restrictions dropped (this is a local
/// preview), redirects kept inside the proxy, connection closed after the response.
fn response_head(head: &Head, upstream: &Upstream, body_length: Option<usize>) -> String {
    let mut out = format!("{}\r\n", head.first);
    for (key, value) in &head.headers {
        match key.to_ascii_lowercase().as_str() {
            "x-frame-options" | "content-security-policy" | "content-security-policy-report-only" | "cross-origin-resource-policy" | "connection" | "keep-alive" => {}
            "content-length" | "transfer-encoding" if body_length.is_some() => {}
            "location" => {
                let mut value = value.clone();
                for spelling in upstream.spellings() {
                    value = value.replace(&spelling, &upstream.proxy_origin());
                }
                out.push_str(&format!("{key}: {value}\r\n"));
            }
            _ => out.push_str(&format!("{key}: {value}\r\n")),
        }
    }
    if let Some(length) = body_length {
        out.push_str(&format!("Content-Length: {length}\r\n"));
    }
    out.push_str("Cross-Origin-Resource-Policy: cross-origin\r\nConnection: close\r\n\r\n");
    out
}

/// Decodes a chunked body; the flag says whether the final chunk has arrived.
fn dechunk(raw: &[u8]) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    loop {
        let Some(line_end) = find(&raw[at..], b"\r\n") else { return (out, false) };
        let size_text = String::from_utf8_lossy(&raw[at..at + line_end]);
        let Ok(size) = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16) else { return (out, false) };
        at += line_end + 2;
        if size == 0 {
            return (out, true);
        }
        if raw.len() < at + size + 2 {
            return (out, false);
        }
        out.extend_from_slice(&raw[at..at + size]);
        at += size + 2;
    }
}

/// Places the bridge script at the top of `<head>` so it sees errors from every later script.
fn inject(html: &[u8]) -> Vec<u8> {
    let window = &html[..html.len().min(64 * 1024)];
    let lower = window.to_ascii_lowercase();
    let after_tag = |name: &[u8]| -> Option<usize> {
        let start = find(&lower, name)?;
        let end = lower[start..].iter().position(|byte| *byte == b'>')?;
        Some(start + end + 1)
    };
    let at = after_tag(b"<head").or_else(|| after_tag(b"<html")).or_else(|| after_tag(b"<!doctype")).unwrap_or(0);
    let mut out = Vec::with_capacity(html.len() + BRIDGE_TAG.len());
    out.extend_from_slice(&html[..at]);
    out.extend_from_slice(BRIDGE_TAG.as_bytes());
    out.extend_from_slice(&html[at..]);
    out
}

async fn handle(mut client: TcpStream, upstream: Upstream) -> io::Result<()> {
    let Some((raw_head, leftover)) = read_head(&mut client).await? else { return Ok(()) };
    let Some(request) = parse_head(&raw_head) else { return Ok(()) };
    let mut parts = request.first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_ascii_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    if target.split('?').next() == Some(BRIDGE_PATH) {
        return respond(&mut client, "200 OK", "text/javascript; charset=utf-8", BRIDGE_JS.as_bytes()).await;
    }
    let mut server = match connect(&upstream).await {
        Ok(stream) => stream,
        Err(_) => return respond(&mut client, "502 Bad Gateway", "text/html; charset=utf-8", unreachable_page(&upstream).as_bytes()).await,
    };
    let upgrade = request.header("upgrade").is_some();
    server.write_all(request_head(&request, &upstream, upgrade).as_bytes()).await?;
    if !leftover.is_empty() {
        server.write_all(&leftover).await?;
    }
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    // Request bodies (and WebSocket frames) flow on their own while the response is read.
    let upload = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
        let _ = server_write.shutdown().await;
    });
    let result = relay_response(&mut server_read, &mut client_write, &upstream, &method).await;
    upload.abort();
    let _ = client_write.shutdown().await;
    result
}

async fn relay_response<R, W>(server: &mut R, client: &mut W, upstream: &Upstream, method: &str) -> io::Result<()>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let Some((raw_head, mut body)) = read_head(server).await? else {
        return respond(client, "502 Bad Gateway", "text/html; charset=utf-8", unreachable_page(upstream).as_bytes()).await;
    };
    let Some(head) = parse_head(&raw_head) else { return Ok(()) };
    let status: u16 = head.first.split_whitespace().nth(1).and_then(|code| code.parse().ok()).unwrap_or(200);
    if status == 101 {
        // WebSocket and other upgrades: hand the connection over untouched.
        client.write_all(&raw_head).await?;
        client.write_all(b"\r\n\r\n").await?;
        client.write_all(&body).await?;
        client.flush().await?;
        tokio::io::copy(server, client).await?;
        return Ok(());
    }
    let is_html = head.header("content-type").is_some_and(|kind| kind.to_ascii_lowercase().contains("text/html"))
        && method != "HEAD"
        && !matches!(status, 100..=199 | 204 | 304)
        && head.header("content-encoding").is_none_or(|encoding| encoding.eq_ignore_ascii_case("identity"));
    if !is_html {
        client.write_all(response_head(&head, upstream, None).as_bytes()).await?;
        client.write_all(&body).await?;
        client.flush().await?;
        tokio::io::copy(server, client).await?;
        return Ok(());
    }
    let chunked = head.header("transfer-encoding").is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    let length = head.header("content-length").and_then(|value| value.parse::<usize>().ok()).filter(|_| !chunked);
    let mut chunk = vec![0u8; 32 * 1024];
    let decoded = loop {
        if chunked {
            let (decoded, complete) = dechunk(&body);
            if complete {
                break Some(decoded);
            }
        } else if length.is_some_and(|expected| body.len() >= expected) {
            break Some(body[..length.unwrap_or(body.len())].to_vec());
        }
        if body.len() > MAX_HTML {
            break None;
        }
        let read = tokio::time::timeout(Duration::from_secs(60), server.read(&mut chunk)).await.map_err(|_| io::ErrorKind::TimedOut)??;
        if read == 0 {
            // Closed: with no length, the body ends here.
            break if chunked { Some(dechunk(&body).0) } else { Some(body.clone()) };
        }
        body.extend_from_slice(&chunk[..read]);
    };
    match decoded {
        Some(html) => {
            let page = inject(&html);
            client.write_all(response_head(&head, upstream, Some(page.len())).as_bytes()).await?;
            client.write_all(&page).await?;
            client.flush().await
        }
        None => {
            client.write_all(response_head(&head, upstream, None).as_bytes()).await?;
            client.write_all(&body).await?;
            tokio::io::copy(server, client).await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_after_the_head_tag() {
        let page = inject(b"<!doctype html><html><HEAD lang=x><title>a</title></head><body></body></html>");
        let text = String::from_utf8(page).unwrap();
        assert!(text.contains("<HEAD lang=x><script src=\"/__neru/bridge.js\""), "{text}");
        assert!(inject(b"<p>fragment</p>").starts_with(BRIDGE_TAG.as_bytes()));
        assert!(String::from_utf8(inject(b"<html><body>x</body></html>")).unwrap().starts_with("<html><script"));
    }

    #[test]
    fn decodes_chunked_bodies() {
        assert_eq!(dechunk(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"), (b"hello world".to_vec(), true));
        assert!(!dechunk(b"5\r\nhel").1);
        assert!(!dechunk(b"5\r\nhello\r\n").1);
    }

    #[test]
    fn rewrites_response_headers() {
        let upstream = Upstream { host: "localhost".into(), port: 5173, proxy_port: 9000 };
        let head = parse_head(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:5173/login\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\nContent-Length: 0").unwrap();
        let out = response_head(&head, &upstream, None);
        assert!(out.contains("Location: http://127.0.0.1:9000/login"));
        assert!(!out.to_ascii_lowercase().contains("x-frame-options") && !out.to_ascii_lowercase().contains("content-security-policy"));
        assert!(out.contains("Cross-Origin-Resource-Policy: cross-origin"));
    }

    /// A tiny HTTP server: serves one canned response per connection.
    async fn upstream_server(response: Vec<u8>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let response = response.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer).await;
                    let _ = stream.write_all(&response).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        port
    }

    async fn get(url: &str) -> String {
        let route_url = reqwest::Url::parse(url).unwrap();
        let mut stream = TcpStream::connect((route_url.host_str().unwrap(), route_url.port().unwrap())).await.unwrap();
        stream.write_all(format!("GET {} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", route_url.path()).as_bytes()).await.unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn proxies_html_with_the_bridge_and_passes_other_files() {
        let html = "<html><head><title>t</title></head><body>hi</body></html>";
        let chunked = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nX-Frame-Options: DENY\r\n\r\n{:x}\r\n{html}\r\n0\r\n\r\n", html.len());
        let page_port = upstream_server(chunked.into_bytes()).await;
        let manager = ProxyManager::default();
        let route = manager.route(&format!("http://127.0.0.1:{page_port}/index.html")).await.unwrap();
        assert!(route.bridged);
        let body = get(&route.frame_url).await;
        assert!(body.contains("<head><script src=\"/__neru/bridge.js\""), "{body}");
        assert!(body.contains("hi</body>"));
        assert!(body.contains("Content-Length:") && !body.to_ascii_lowercase().contains("transfer-encoding") && !body.to_ascii_lowercase().contains("x-frame-options"));

        let script = get(&format!("{}/__neru/bridge.js", route.proxy_origin.clone().unwrap())).await;
        assert!(script.contains("__neruBridge"));

        let css_port = upstream_server(b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: 6\r\n\r\na{b:c}".to_vec()).await;
        let css = manager.route(&format!("http://127.0.0.1:{css_port}/a.css")).await.unwrap();
        assert!(get(&css.frame_url).await.ends_with("a{b:c}"));
        manager.shutdown();
    }

    #[tokio::test]
    async fn passes_websocket_upgrades_through() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer).await;
                    let _ = stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n").await;
                    while let Ok(read) = stream.read(&mut buffer).await {
                        if read == 0 {
                            break;
                        }
                        let _ = stream.write_all(&buffer[..read].to_ascii_uppercase()).await;
                    }
                });
            }
        });
        let manager = ProxyManager::default();
        let route = manager.route(&format!("http://127.0.0.1:{port}/")).await.unwrap();
        let framed = reqwest::Url::parse(&route.frame_url).unwrap();
        let mut socket = TcpStream::connect((framed.host_str().unwrap(), framed.port().unwrap())).await.unwrap();
        socket.write_all(b"GET /ws HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n").await.unwrap();
        let mut head = vec![0u8; 512];
        let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut head)).await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&head[..read]).starts_with("HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head[..read]));
        socket.write_all(b"hello").await.unwrap();
        let mut echo = [0u8; 5];
        tokio::time::timeout(Duration::from_secs(3), socket.read_exact(&mut echo)).await.unwrap().unwrap();
        assert_eq!(&echo, b"HELLO");
        manager.shutdown();
    }

    #[tokio::test]
    async fn explains_when_nothing_is_listening() {
        let free = TcpListener::bind(("127.0.0.1", 0)).await.unwrap().local_addr().unwrap().port();
        let manager = ProxyManager::default();
        let route = manager.route(&format!("http://127.0.0.1:{free}/")).await.unwrap();
        assert!(get(&route.frame_url).await.contains("Nothing is running"));
        manager.shutdown();
    }

    #[tokio::test]
    async fn routes_only_local_http_and_https() {
        let manager = ProxyManager::default();
        assert!(!manager.route("https://docs.rs/tokio").await.unwrap().bridged);
        assert!(manager.route("http://example.com/").await.is_err());
        assert!(manager.route("file:///C:/Windows").await.is_err());
        assert!(manager.route("http://192.168.1.4:3000").await.is_err());
    }
}
