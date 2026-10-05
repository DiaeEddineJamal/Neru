//! Managed outbound tunnel: carries only Neru's encrypted WebSocket frames.
//! Quick URLs last for this desktop run; a stable WSS endpoint can be configured separately.
use std::{fs, path::PathBuf, process::Stdio, sync::Mutex, time::Duration};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use crate::{Hidden, workspace::data_dir};

static TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);
static URL: Mutex<String> = Mutex::new(String::new());
static ERROR: Mutex<String> = Mutex::new(String::new());
pub fn url() -> String { URL.lock().map(|s| s.clone()).unwrap_or_default() }
pub fn error() -> String { ERROR.lock().map(|s| s.clone()).unwrap_or_default() }
pub fn stop() {
    if let Ok(mut task) = TASK.lock() { if let Some(task) = task.take() { task.abort(); } }
    if let Ok(mut url) = URL.lock() { url.clear(); }
    if let Ok(mut error) = ERROR.lock() { error.clear(); }
}

async fn binary() -> Result<PathBuf, String> {
    // Pin version and digest; never execute an unverified downloaded executable.
    #[cfg(all(windows, target_arch = "x86_64"))]
    let (asset, hash) = ("cloudflared-windows-amd64.exe", "f096265ec2fcbe9bb6e2d64268db167ced3fcbb83d894bdb9e2fcdb26f2ea7e2");
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    return Err("Automatic internet access currently supports Windows x64. Set a stable WSS endpoint for other desktops.".into());
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let directory = data_dir()?.join("remote-tools");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!("2026.9.3-{asset}"));
        if fs::read(&path).ok().is_some_and(|b| format!("{:x}", Sha256::digest(b)) == hash) { return Ok(path); }
        let client = reqwest::Client::builder().timeout(Duration::from_secs(180)).build().map_err(|e| e.to_string())?;
        let response = client.get(format!("https://github.com/cloudflare/cloudflared/releases/download/2026.9.3/{asset}"))
            .send().await.map_err(|e| format!("Could not download the internet connector: {e}"))?
            .error_for_status().map_err(|e| e.to_string())?;
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(&bytes)) != hash { return Err("The internet connector failed its integrity check.".into()); }
        let partial = path.with_extension("partial");
        fs::write(&partial, &bytes).map_err(|e| e.to_string())?;
        if path.exists() { fs::remove_file(&path).map_err(|e| e.to_string())?; }
        fs::rename(partial, &path).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

pub fn start(app: AppHandle, port: u16) {
    stop();
    let task = tauri::async_runtime::spawn(async move {
        let result = run(&app, port).await;
        if let Ok(mut url) = URL.lock() { url.clear(); }
        if let Ok(mut error) = ERROR.lock() { *error = result.err().unwrap_or_else(|| "Internet connector stopped. Turn internet access off and on to reconnect.".into()); }
        let _ = app.emit("remote://tunnel", ());
    });
    if let Ok(mut current) = TASK.lock() { *current = Some(task); }
}

async fn run(app: &AppHandle, port: u16) -> Result<(), String> {
    let path = binary().await?;
    let mut command = tokio::process::Command::new(&path).hidden();
    command.kill_on_drop(true).current_dir(path.parent().unwrap())
        .args(["tunnel", "--no-autoupdate", "--protocol", "http2", "--url", &format!("http://127.0.0.1:{port}")])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("Could not start internet access: {e}"))?;
    let stderr = child.stderr.take().ok_or("Internet connector did not provide output")?;
    let mut lines = BufReader::new(stderr).lines();
    let mut announced = None;
    // Drain output for the process lifetime, so logging never blocks its connection.
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        if let Some(endpoint) = endpoint(&line) { announced = Some(endpoint); }
        if line.contains("Registered tunnel connection") {
            if let Some(endpoint) = announced.take() {
                if let Ok(mut url) = URL.lock() { *url = endpoint; }
                let _ = app.emit("remote://tunnel", ());
            }
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    Err(format!("Internet connector exited ({status}). Turn internet access off and on to retry."))
}

fn endpoint(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let candidate = line[start..].split_whitespace().next()?.trim_end_matches('|');
    let parsed = reqwest::Url::parse(candidate).ok()?;
    let host = parsed.host_str()?;
    if !host.ends_with(".trycloudflare.com") || parsed.path() != "/" { return None; }
    Some(format!("wss://{host}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_accepts_cloudflare_endpoint() {
        assert_eq!(endpoint(" | https://sage-quiet-wave.trycloudflare.com |"), Some("wss://sage-quiet-wave.trycloudflare.com".into()));
        assert!(endpoint("https://trycloudflare.com.evil.test").is_none());
        assert!(endpoint("https://other.test").is_none());
    }
}
