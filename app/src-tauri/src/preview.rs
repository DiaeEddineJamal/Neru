//! In-app preview of a project dev server. The command is started in the project folder and the
//! guessed URL is shown in the preview pane.

use std::{
    fs,
    path::Path,
    process::Stdio,
    sync::Mutex,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{AppState, workspace::project_root};

pub struct PreviewManager {
    child: Mutex<Option<tokio::process::Child>>,
}

impl Default for PreviewManager {
    fn default() -> Self {
        Self { child: Mutex::new(None) }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewHint {
    pub command: String,
    pub url: String,
}

pub fn hint(root: &Path) -> Result<PreviewHint, String> {
    let path = root.join("package.json");
    if !path.is_file() {
        return Err("This project has no package.json dev server".into());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let (name, script) = if let Some(script) = json["scripts"]["dev"].as_str() {
        ("dev", script)
    } else if let Some(script) = json["scripts"]["start"].as_str() {
        ("start", script)
    } else {
        return Err("package.json has no dev or start script".into());
    };
    let url = guess_url(script);
    Ok(PreviewHint { command: format!("npm run {name}"), url })
}

fn guess_url(script: &str) -> String {
    if let Some(port) = script.split("--port").nth(1).and_then(|rest| rest.split_whitespace().next()).and_then(|value| value.trim_start_matches('=').parse::<u16>().ok()) {
        return format!("http://127.0.0.1:{port}");
    }
    let lower = script.to_lowercase();
    let port = if lower.contains("vite") || lower.contains("1420") {
        if lower.contains("1420") { 1420 } else { 5173 }
    } else if lower.contains("next") {
        3000
    } else {
        3000
    };
    format!("http://127.0.0.1:{port}")
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

impl PreviewManager {
    pub async fn start(&self, root: &Path) -> Result<PreviewHint, String> {
        let hint = hint(root)?;
        self.stop();
        #[cfg(windows)]
        let mut process = {
            let mut process = tokio::process::Command::new("powershell.exe");
            process.args(["-NoProfile", "-Command", &hint.command]);
            process
        };
        #[cfg(not(windows))]
        let mut process = {
            let mut process = tokio::process::Command::new("sh");
            process.args(["-lc", &hint.command]);
            process
        };
        let child = process
            .current_dir(root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Could not start the dev server: {e}"))?;
        *self.child.lock().map_err(|e| e.to_string())? = Some(child);
        Ok(hint)
    }

    pub fn stop(&self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.start_kill();
            }
            *child = None;
        }
    }
}

#[tauri::command]
pub fn preview_hint(state: State<'_, AppState>) -> Result<PreviewHint, String> {
    hint(&project_root(&state)?)
}

#[tauri::command]
pub async fn preview_start(app: AppHandle) -> Result<PreviewHint, String> {
    let state = app.state::<AppState>();
    let root = project_root(&state)?;
    let hint = state.preview.start(&root).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_a_port_and_rejects_file_urls() {
        assert_eq!(guess_url("vite --port 1420"), "http://127.0.0.1:1420");
        assert_eq!(guess_url("vite"), "http://127.0.0.1:5173");
        assert!(allowed_url("http://127.0.0.1:5173"));
        assert!(allowed_url("https://docs.example.com/guide"));
        assert!(!allowed_url("file:///C:/Windows"));
        assert!(!allowed_url("http://192.168.0.8"));
    }
}
