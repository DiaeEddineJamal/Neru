//! OAuth 2.1 sign-in for hosted MCP connectors, following the MCP authorization spec: protected
//! resource metadata → authorization server metadata → dynamic client registration → browser
//! authorization with PKCE (S256) → a one-shot loopback callback on 127.0.0.1 → token exchange.
//! Tokens are stored encrypted (see `settings::seal_secret`) and refreshed when they expire.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::workspace::data_dir;

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds; 0 when the server did not say.
    #[serde(default)]
    pub expires_at: u64,
    pub client_id: String,
    pub token_endpoint: String,
    pub resource: String,
}

impl Token {
    pub fn expired(&self) -> bool {
        self.expires_at != 0 && now() + 60 >= self.expires_at
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn tokens_path() -> Result<std::path::PathBuf, String> {
    Ok(data_dir()?.join("mcp-tokens.json"))
}

fn read_all() -> HashMap<String, String> {
    tokens_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn load(server: &str) -> Option<Token> {
    let sealed = read_all().remove(server)?;
    serde_json::from_str(&crate::settings::open_secret(&sealed)?).ok()
}

pub fn store(server: &str, token: Option<&Token>) -> Result<(), String> {
    let mut all = read_all();
    match token {
        Some(token) => {
            let text = serde_json::to_string(token).map_err(|e| e.to_string())?;
            let sealed = crate::settings::seal_secret(&text)
                .ok_or("This system has no secure key store, so the sign-in cannot be saved")?;
            all.insert(server.to_string(), sealed);
        }
        None => {
            all.remove(server);
        }
    }
    fs::write(tokens_path()?, serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

// ---- SHA-256 and base64url, for the PKCE challenge ----

pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut out = [0u8; 32];
    for (i, value) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    out
}

pub fn base64url(bytes: &[u8]) -> String {
    crate::documents::base64(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("http://form.invalid/").expect("static url");
    url.query_pairs_mut().extend_pairs(pairs);
    url.query().unwrap_or("").to_string()
}

// ---- Discovery ----

#[derive(Debug, Clone)]
pub struct AuthServer {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub scopes: Vec<String>,
}

/// Pulls `resource_metadata="…"` out of a `WWW-Authenticate: Bearer …` header.
pub fn resource_metadata_hint(header: &str) -> Option<String> {
    let start = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = header[start..].trim_start_matches('"');
    let end = rest.find(['"', ',', ' ']).unwrap_or(rest.len());
    Some(rest[..end].to_string()).filter(|value| value.starts_with("https://") || value.starts_with("http://"))
}

async fn get_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    let response = http.get(url).header("Accept", "application/json").send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

fn well_known(base: &reqwest::Url, suffix: &str) -> Vec<String> {
    let origin = format!("{}://{}", base.scheme(), base.host_str().unwrap_or(""))
        + &base.port().map(|port| format!(":{port}")).unwrap_or_default();
    let path = base.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(format!("{origin}/.well-known/{suffix}{path}"));
    }
    urls.push(format!("{origin}/.well-known/{suffix}"));
    urls
}

pub async fn discover(http: &reqwest::Client, resource: &str, www_authenticate: Option<&str>) -> Result<AuthServer, String> {
    let resource_url = reqwest::Url::parse(resource).map_err(|_| "Invalid connector URL")?;
    let mut candidates: Vec<String> = www_authenticate.and_then(resource_metadata_hint).into_iter().collect();
    candidates.extend(well_known(&resource_url, "oauth-protected-resource"));
    let mut issuer = None;
    for url in candidates {
        if let Some(meta) = get_json(http, &url).await {
            issuer = meta["authorization_servers"][0].as_str().map(String::from);
            if issuer.is_some() {
                break;
            }
        }
    }
    let issuer = issuer.unwrap_or_else(|| resource_url.origin().ascii_serialization());
    let issuer_url = reqwest::Url::parse(&issuer).map_err(|_| "Invalid authorization server")?;
    let mut candidates = well_known(&issuer_url, "oauth-authorization-server");
    candidates.extend(well_known(&issuer_url, "openid-configuration"));
    for url in candidates {
        let Some(meta) = get_json(http, &url).await else { continue };
        let (Some(authorization), Some(token)) = (meta["authorization_endpoint"].as_str(), meta["token_endpoint"].as_str()) else {
            continue;
        };
        return Ok(AuthServer {
            authorization_endpoint: authorization.into(),
            token_endpoint: token.into(),
            registration_endpoint: meta["registration_endpoint"].as_str().map(String::from),
            scopes: meta["scopes_supported"]
                .as_array()
                .map(|list| list.iter().filter_map(|scope| scope.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        });
    }
    Err("This connector does not publish OAuth metadata. Add an Authorization header with a token instead.".into())
}

async fn register(http: &reqwest::Client, server: &AuthServer, redirect: &str) -> Result<String, String> {
    let endpoint = server.registration_endpoint.as_deref().ok_or(
        "This connector does not allow automatic app registration. Add an Authorization header with a token instead.",
    )?;
    let response = http
        .post(endpoint)
        .json(&json!({
            "client_name": "Neru",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await
        .map_err(|e| format!("Registration failed: {e}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|e| format!("Registration failed: {e}"))?;
    body["client_id"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("Registration failed ({status}): {}", body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("no client id")))
}

fn token_from(body: &Value, previous: Option<&Token>, client_id: &str, token_endpoint: &str, resource: &str) -> Result<Token, String> {
    let access = body["access_token"].as_str().ok_or_else(|| {
        format!("Sign-in failed: {}", body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("no access token"))
    })?;
    Ok(Token {
        access_token: access.into(),
        refresh_token: body["refresh_token"].as_str().map(String::from).or_else(|| previous.and_then(|token| token.refresh_token.clone())),
        expires_at: body["expires_in"].as_u64().map(|seconds| now() + seconds).unwrap_or(0),
        client_id: client_id.into(),
        token_endpoint: token_endpoint.into(),
        resource: resource.into(),
    })
}

pub async fn refresh(http: &reqwest::Client, token: &Token) -> Result<Token, String> {
    let refresh = token.refresh_token.as_deref().ok_or("The sign-in expired; sign in again")?;
    let body = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", &token.client_id),
        ("resource", &token.resource),
    ]);
    let response = http
        .post(&token.token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Refreshing the sign-in failed: {e}"))?;
    let json: Value = response.json().await.map_err(|e| e.to_string())?;
    token_from(&json, Some(token), &token.client_id, &token.token_endpoint, &token.resource)
}

/// Waits for the browser to come back to the loopback redirect; returns the query parameters.
fn wait_for_callback(listener: TcpListener, timeout: Duration) -> Result<HashMap<String, String>, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let target = request.split_whitespace().nth(1).unwrap_or("/");
                if !target.starts_with("/callback") {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|e| e.to_string())?;
                let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
                let ok = params.contains_key("code");
                let page = format!(
                    "<!doctype html><meta charset=utf-8><title>Neru</title><body style=\"font:16px system-ui;background:#f4f3ee;color:#1f2a1f;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><h1 style=\"font-weight:500\">{}</h1><p>You can close this tab and return to Neru.</p></div>",
                    if ok { "Connected to Neru" } else { "Sign-in was not completed" }
                );
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
                return Ok(params);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() > deadline {
                    return Err("Sign-in timed out after five minutes".into());
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Runs the whole browser sign-in for a hosted connector and saves the token.
pub async fn sign_in(server_name: &str, resource: &str, www_authenticate: Option<&str>) -> Result<Token, String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let server = discover(&http, resource, www_authenticate).await?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Could not open the sign-in callback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let client_id = register(&http, &server, &redirect).await?;
    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = base64url(&sha256(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().simple().to_string();
    let mut authorize = reqwest::Url::parse(&server.authorization_endpoint).map_err(|_| "Invalid authorization endpoint")?;
    {
        let mut query = authorize.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("resource", resource);
        if !server.scopes.is_empty() {
            query.append_pair("scope", &server.scopes.join(" "));
        }
    }
    crate::web::open_url(authorize.to_string())?;
    let params = tauri::async_runtime::spawn_blocking(move || wait_for_callback(listener, Duration::from_secs(300)))
        .await
        .map_err(|e| e.to_string())??;
    if params.get("state") != Some(&state) {
        return Err("Sign-in was rejected: the response did not match this request".into());
    }
    if let Some(error) = params.get("error") {
        return Err(format!("Sign-in was declined: {}", params.get("error_description").unwrap_or(error)));
    }
    let code = params.get("code").ok_or("Sign-in returned no code")?;
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect),
        ("client_id", &client_id),
        ("code_verifier", &verifier),
        ("resource", resource),
    ]);
    let response = http
        .post(&server.token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?;
    let json: Value = response.json().await.map_err(|e| format!("Token exchange failed: {e}"))?;
    let token = token_from(&json, None, &client_id, &server.token_endpoint, resource)?;
    store(server_name, Some(&token))?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // RFC 7636 appendix B PKCE example.
        assert_eq!(
            base64url(&sha256(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn reads_resource_metadata_hint() {
        let header = r#"Bearer error="invalid_token", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource""#;
        assert_eq!(
            resource_metadata_hint(header).as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );
        assert_eq!(resource_metadata_hint("Bearer"), None);
        let base = reqwest::Url::parse("https://mcp.example.com/v1/mcp").unwrap();
        assert_eq!(
            well_known(&base, "oauth-protected-resource"),
            vec![
                "https://mcp.example.com/.well-known/oauth-protected-resource/v1/mcp".to_string(),
                "https://mcp.example.com/.well-known/oauth-protected-resource".to_string()
            ]
        );
    }
}
