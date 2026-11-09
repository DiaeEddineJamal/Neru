//! The local model library: whether Ollama is installed and running, the models it has, and
//! downloading, cancelling and removing them from inside Neru instead of a terminal. Ollama stays the
//! engine; Neru never runs its installer.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

/// Ollama's address as Ollama itself reads it: OLLAMA_HOST, else 127.0.0.1:11434, with the `/v1`
/// suffix Neru's provider settings use.
pub fn base_url() -> String {
    let host = std::env::var("OLLAMA_HOST").ok().map(|host| host.trim().to_string()).filter(|host| !host.is_empty()).unwrap_or_else(|| "127.0.0.1:11434".into());
    let host = if host.contains("://") { host } else { format!("http://{host}") };
    // 0.0.0.0 is how Ollama is told to listen everywhere; on this computer it answers on 127.0.0.1.
    format!("{}/v1", host.trim_end_matches('/').replace("0.0.0.0", "127.0.0.1"))
}

fn root() -> String {
    base_url().trim_end_matches("/v1").to_string()
}

static SEEN: LazyLock<Mutex<Option<(Instant, Option<String>)>>> = LazyLock::new(Default::default);

/// The base URL when Ollama answers on this computer, checked at most every half minute. An Ollama on
/// another machine counts as not running: work sent there would not stay private.
pub async fn running() -> Option<String> {
    if let Some((at, found)) = SEEN.lock().ok().and_then(|seen| seen.clone()) {
        if at.elapsed() < Duration::from_secs(30) {
            return found;
        }
    }
    let base_url = base_url();
    let found = match reqwest::Url::parse(&base_url) {
        Ok(url) if crate::providers::is_local(&url) => {
            let address = format!("{}:{}", url.host_str().unwrap_or("127.0.0.1"), url.port_or_known_default().unwrap_or(11434));
            matches!(tokio::time::timeout(Duration::from_millis(300), tokio::net::TcpStream::connect(address)).await, Ok(Ok(_))).then_some(base_url)
        }
        _ => None,
    };
    if let Ok(mut seen) = SEEN.lock() {
        *seen = Some((Instant::now(), found.clone()));
    }
    found
}

fn forget_seen() {
    if let Ok(mut seen) = SEEN.lock() {
        *seen = None;
    }
}

/// The `ollama` program, from PATH or where the installers put it.
fn program() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "ollama.exe" } else { "ollama" };
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>()).map(|dir| dir.join(name));
    let usual: Vec<std::path::PathBuf> = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|dir| std::path::PathBuf::from(dir).join("Programs").join("Ollama").join(name)).into_iter().collect()
    } else if cfg!(target_os = "macos") {
        vec!["/Applications/Ollama.app/Contents/Resources/ollama".into(), "/usr/local/bin/ollama".into(), "/opt/homebrew/bin/ollama".into()]
    } else {
        vec!["/usr/local/bin/ollama".into(), "/usr/bin/ollama".into()]
    };
    on_path.chain(usual).find(|path| path.is_file())
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub name: String,
    pub size_gb: f32,
    /// "4.3B", as Ollama reports it.
    pub parameters: String,
    pub quantization: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub base_url: String,
    pub models: Vec<InstalledModel>,
    /// Downloads in progress, by tag.
    pub pulling: Vec<String>,
    /// Speeds measured on this computer, by tag.
    pub speeds: HashMap<String, Speed>,
}

/// Installed models from `/api/tags`.
pub fn parse_tags(body: &Value) -> Vec<InstalledModel> {
    let mut models: Vec<InstalledModel> = body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| InstalledModel {
            name: item["name"].as_str().or_else(|| item["model"].as_str()).unwrap_or_default().to_string(),
            size_gb: item["size"].as_f64().unwrap_or(0.0) as f32 / 1_073_741_824.0,
            parameters: item["details"]["parameter_size"].as_str().unwrap_or_default().to_string(),
            quantization: item["details"]["quantization_level"].as_str().unwrap_or_default().to_string(),
        })
        .filter(|model| !model.name.is_empty())
        .collect();
    models.sort_by(|a, b| a.name.cmp(&b.name));
    models
}

#[tauri::command]
pub async fn ollama_status() -> Result<OllamaStatus, String> {
    forget_seen();
    let running = running().await.is_some();
    let client = crate::providers::http();
    let get = |path: &str| {
        let request = client.get(format!("{}{path}", root())).timeout(Duration::from_secs(5));
        async move { request.send().await.ok()?.json::<Value>().await.ok() }
    };
    let (version, models) = if running { (get("/api/version").await.and_then(|body| body["version"].as_str().map(str::to_string)), get("/api/tags").await.map(|body| parse_tags(&body)).unwrap_or_default()) } else { (None, Vec::new()) };
    let pulling = PULLS.lock().map(|pulls| pulls.keys().cloned().collect()).unwrap_or_default();
    Ok(OllamaStatus { installed: running || program().is_some(), running, version, base_url: base_url(), models, pulling, speeds: speeds() })
}

/// Starts Ollama's server when it is installed but not running (the tray app usually does this).
#[tauri::command]
pub async fn ollama_start() -> Result<(), String> {
    use crate::Hidden;
    if running().await.is_some() {
        return Ok(());
    }
    let program = program().ok_or("Ollama is not installed. Get it from ollama.com/download.")?;
    let mut command = std::process::Command::new(program);
    command.arg("serve").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    command.hidden().spawn().map_err(|e| format!("Ollama could not start: {e}"))?;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        forget_seen();
        if running().await.is_some() {
            return Ok(());
        }
    }
    Err("Ollama started but does not answer yet. Try again in a moment.".into())
}

// ---------- downloads ----------

/// Progress of one download, as the window shows it.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    pub tag: String,
    /// Ollama's own words: "pulling manifest", "downloading", "verifying sha256 digest", "success".
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

/// Running downloads, so a second request for the same tag does nothing and Cancel can stop one.
static PULLS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> = LazyLock::new(Default::default);

/// Adds one line of `/api/pull`'s stream to the totals. A model downloads in several layers, each
/// with its own digest, total and completed bytes.
pub fn add_pull_line(line: &Value, layers: &mut HashMap<String, (u64, u64)>, progress: &mut PullProgress) {
    if let Some(error) = line["error"].as_str() {
        progress.error = Some(error.to_string());
        progress.done = true;
        return;
    }
    if let Some(status) = line["status"].as_str() {
        progress.status = if status.starts_with("pulling ") && line["digest"].is_string() { "downloading".into() } else { status.to_string() };
        if status == "success" {
            progress.done = true;
        }
    }
    if let (Some(digest), Some(total)) = (line["digest"].as_str(), line["total"].as_u64()) {
        layers.insert(digest.to_string(), (total, line["completed"].as_u64().unwrap_or(0)));
    }
    progress.total = layers.values().map(|(total, _)| total).sum();
    progress.completed = layers.values().map(|(total, completed)| (*completed).min(*total)).sum();
}

fn report(app: &AppHandle, progress: &PullProgress) {
    let _ = app.emit("ollama://pull", progress);
}

/// Downloads `tag`, reporting progress on `ollama://pull`. Checks the disk first when the catalog knows
/// the size. Ollama keeps what it got, so a cancelled download resumes where it stopped.
#[tauri::command]
pub async fn ollama_pull(tag: String, app: AppHandle) -> Result<(), String> {
    let tag = tag.trim().to_string();
    if tag.is_empty() || tag.contains(char::is_whitespace) {
        return Err("Enter a model name such as gemma3:4b".into());
    }
    if running().await.is_none() {
        return Err("Ollama is not running. Start it, then download again.".into());
    }
    if let Some(size) = crate::registry::get().local_models.iter().find(|model| model.tag == tag).map(|model| model.size_gb) {
        let free = tauri::async_runtime::spawn_blocking(|| crate::hardware::detect().disk_free_gb).await.ok().flatten();
        if let Some(free) = free.filter(|free| *free < size + 2.0) {
            return Err(format!("{tag} needs about {size:.1} GB and the drive that holds Ollama's models has {free:.1} GB free. Free some space or move Ollama's models (OLLAMA_MODELS) to another drive."));
        }
    }
    let cancel = {
        let mut pulls = PULLS.lock().map_err(|e| e.to_string())?;
        if pulls.contains_key(&tag) {
            return Ok(());
        }
        let cancel = Arc::new(tokio::sync::Notify::new());
        pulls.insert(tag.clone(), cancel.clone());
        cancel
    };
    let result = pull(&tag, &app, &cancel).await;
    if let Ok(mut pulls) = PULLS.lock() {
        pulls.remove(&tag);
    }
    crate::fallback::models_changed("ollama");
    result
}

async fn pull(tag: &str, app: &AppHandle, cancel: &tokio::sync::Notify) -> Result<(), String> {
    let mut progress = PullProgress { tag: tag.to_string(), status: "starting".into(), ..PullProgress::default() };
    report(app, &progress);
    let response = crate::providers::http().post(format!("{}/api/pull", root())).json(&json!({"model": tag, "stream": true})).send().await.map_err(|e| format!("Ollama could not be reached: {e}"))?;
    if !response.status().is_success() {
        let error = crate::providers::read_response(response).await.err().unwrap_or_else(|| "Ollama refused the download".into());
        progress.error = Some(error.clone());
        progress.done = true;
        report(app, &progress);
        return Err(error);
    }
    let mut response = response;
    let mut layers = HashMap::new();
    let mut pending = Vec::new();
    let mut last_report = Instant::now();
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|e| format!("The download stopped: {e}"))?,
            _ = cancel.notified() => {
                progress.status = "cancelled".into();
                progress.done = true;
                report(app, &progress);
                return Err("cancelled".into());
            }
        };
        let Some(chunk) = chunk else { break };
        pending.extend_from_slice(&chunk);
        while let Some(at) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=at).collect();
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                add_pull_line(&value, &mut layers, &mut progress);
            }
        }
        // Several lines a second arrive; the window needs a few.
        if progress.done || last_report.elapsed() >= Duration::from_millis(250) {
            report(app, &progress);
            last_report = Instant::now();
        }
        if progress.done {
            break;
        }
    }
    if !progress.done {
        progress.error = Some("The download ended before Ollama said it was complete.".into());
        progress.done = true;
    }
    report(app, &progress);
    progress.error.map_or(Ok(()), Err)
}

#[tauri::command]
pub fn ollama_cancel_pull(tag: String) {
    if let Some(cancel) = PULLS.lock().ok().and_then(|pulls| pulls.get(tag.trim()).cloned()) {
        cancel.notify_one();
    }
}

#[tauri::command]
pub async fn ollama_delete(tag: String) -> Result<(), String> {
    let response = crate::providers::http().delete(format!("{}/api/delete", root())).json(&json!({"model": tag.trim()})).timeout(Duration::from_secs(30)).send().await.map_err(|e| format!("Ollama could not be reached: {e}"))?;
    crate::providers::read_response(response).await.map(|_| ()).or_else(|error| if error.contains("EOF") || error.contains("empty") { Ok(()) } else { Err(error) })?;
    crate::fallback::models_changed("ollama");
    Ok(())
}

// ---------- speed check ----------

/// What a model managed on this computer, measured with Ollama's own timings.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Speed {
    /// Tokens a second reading the prompt: how long a long conversation takes to start answering.
    pub prompt_per_second: f32,
    /// Tokens a second writing.
    pub generate_per_second: f32,
    /// Seconds to load the model into memory the first time.
    pub load_seconds: f32,
    /// "agent", "slow-agent", "helper" or "too-slow".
    pub verdict: String,
    pub sentence: String,
    /// Unix seconds.
    pub measured_at: u64,
}

/// Turns measured speeds into a verdict. A Code session sends prompts of several thousand tokens, so
/// prompt speed matters as much as writing speed; a chore writes a line or two.
pub fn judge(prompt: f32, generate: f32) -> (&'static str, String) {
    // Seconds to read a typical 6,000-token first request.
    let start = 6_000.0 / prompt.max(1.0);
    if generate >= 15.0 && prompt >= 300.0 {
        ("agent", format!("Fast enough for Code: it writes about {generate:.0} tokens a second and starts a long reply in about {start:.0} s."))
    } else if generate >= 6.0 && prompt >= 60.0 {
        ("slow-agent", format!("Works for Code, slowly: about {generate:.0} tokens a second, and a long first request takes about {} to start. Fine as a helper.", human_seconds(start)))
    } else if generate >= 6.0 {
        ("helper", format!("Good for chores (titles, summaries, commit messages) at about {generate:.0} tokens a second, but too slow reading long prompts for Code."))
    } else {
        ("too-slow", format!("Too slow on this computer (about {generate:.1} tokens a second). A smaller model will help more."))
    }
}

fn human_seconds(seconds: f32) -> String {
    if seconds >= 90.0 { format!("{:.0} min", seconds / 60.0) } else { format!("{seconds:.0} s") }
}

/// Reads `/api/generate`'s timings (nanoseconds) into a `Speed`.
pub fn speed_from(body: &Value, now: u64) -> Option<Speed> {
    let per_second = |count: &str, duration: &str| {
        let (count, nanos) = (body[count].as_f64()?, body[duration].as_f64()?);
        (count > 0.0 && nanos > 0.0).then(|| (count / (nanos / 1e9)) as f32)
    };
    let prompt = per_second("prompt_eval_count", "prompt_eval_duration")?;
    let generate = per_second("eval_count", "eval_duration")?;
    let (verdict, sentence) = judge(prompt, generate);
    Some(Speed { prompt_per_second: prompt, generate_per_second: generate, load_seconds: (body["load_duration"].as_f64().unwrap_or(0.0) / 1e9) as f32, verdict: verdict.into(), sentence, measured_at: now })
}

fn speeds_path() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::workspace::data_dir().ok().map(|dir| dir.join("local-speeds.json"))
}

/// Speeds measured earlier, by model tag.
pub fn speeds() -> HashMap<String, Speed> {
    speeds_path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

/// A prompt of about a thousand tokens of ordinary code, so prompt reading is measured on real text.
pub fn speed_prompt() -> String {
    let block = "function totalWithTax(items, rate) {\n  let sum = 0\n  for (const item of items) {\n    sum += item.price * item.quantity\n  }\n  return Math.round(sum * (1 + rate) * 100) / 100\n}\n\n";
    format!("{}Explain in two sentences what the code above does.", block.repeat(18))
}

/// Measures `tag` on this computer and keeps the result. Loading a large model can take a minute.
#[tauri::command]
pub async fn ollama_speed(tag: String) -> Result<Speed, String> {
    let tag = tag.trim().to_string();
    if running().await.is_none() {
        return Err("Ollama is not running.".into());
    }
    // Unloaded again a minute later, so a large model does not keep the memory after the check.
    let payload = json!({"model": tag, "prompt": speed_prompt(), "stream": false, "keep_alive": "1m", "options": {"num_predict": 64, "num_ctx": 4096, "temperature": 0}});
    let response = crate::providers::http().post(format!("{}/api/generate", root())).json(&payload).timeout(Duration::from_secs(600)).send().await.map_err(|e| format!("Ollama could not be reached: {e}"))?;
    let body = crate::providers::read_response(response).await?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let speed = speed_from(&body, now).ok_or("Ollama did not report its timings.")?;
    save_speed(&tag, &speed);
    Ok(speed)
}

/// Keeps a measured speed (the built-in engine's are keyed `builtin:<id>`).
pub fn save_speed(key: &str, speed: &Speed) {
    if let Some(path) = speeds_path() {
        let mut all = speeds();
        all.insert(key.to_string(), speed.clone());
        if let Ok(bytes) = serde_json::to_vec_pretty(&all) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_come_from_ollamas_own_timings() {
        let body = json!({"load_duration": 4_200_000_000u64, "prompt_eval_count": 1_000, "prompt_eval_duration": 2_000_000_000u64, "eval_count": 64, "eval_duration": 5_000_000_000u64});
        let speed = speed_from(&body, 1).unwrap();
        assert_eq!((speed.prompt_per_second.round(), speed.generate_per_second.round(), speed.load_seconds.round()), (500.0, 13.0, 4.0));
        assert_eq!(speed.verdict, "slow-agent");
        assert!(speed_from(&json!({"eval_count": 0}), 1).is_none());
    }

    #[test]
    fn verdicts_weigh_prompt_and_writing_speed() {
        assert_eq!(judge(800.0, 40.0).0, "agent");
        let (verdict, sentence) = judge(60.0, 12.0);
        assert_eq!(verdict, "slow-agent");
        assert!(sentence.contains("2 min"), "6,000 tokens at 60 a second: {sentence}");
        assert_eq!(judge(30.0, 20.0).0, "helper", "writes fast but reads long prompts too slowly");
        assert_eq!(judge(500.0, 3.0).0, "too-slow");
    }

    #[test]
    fn download_progress_adds_up_every_layer() {
        let mut layers = HashMap::new();
        let mut progress = PullProgress { tag: "gemma3:4b".into(), ..PullProgress::default() };
        for line in [
            json!({"status": "pulling manifest"}),
            json!({"status": "pulling aaa", "digest": "sha256:aaa", "total": 3_000, "completed": 1_000}),
            json!({"status": "pulling bbb", "digest": "sha256:bbb", "total": 1_000, "completed": 500}),
            json!({"status": "pulling aaa", "digest": "sha256:aaa", "total": 3_000, "completed": 3_000}),
        ] {
            add_pull_line(&line, &mut layers, &mut progress);
        }
        assert_eq!((progress.completed, progress.total, progress.status.as_str(), progress.done), (3_500, 4_000, "downloading", false));
        add_pull_line(&json!({"status": "verifying sha256 digest"}), &mut layers, &mut progress);
        add_pull_line(&json!({"status": "success"}), &mut layers, &mut progress);
        assert!(progress.done && progress.error.is_none());
        let mut failed = PullProgress::default();
        add_pull_line(&json!({"error": "pull model manifest: file does not exist"}), &mut HashMap::new(), &mut failed);
        assert!(failed.done && failed.error.as_deref() == Some("pull model manifest: file does not exist"));
    }

    #[test]
    fn installed_models_read_from_tags() {
        let body = json!({"models": [
            {"name": "qwen3-coder:30b", "size": 19_000_000_000u64, "details": {"parameter_size": "30.5B", "quantization_level": "Q4_K_M"}},
            {"name": "gemma3:4b", "size": 3_338_801_804u64, "details": {"parameter_size": "4.3B", "quantization_level": "Q4_K_M"}},
        ]});
        let models = parse_tags(&body);
        assert_eq!(models[0].name, "gemma3:4b", "sorted by name");
        assert!((models[0].size_gb - 3.1).abs() < 0.1);
        assert_eq!(models[1].parameters, "30.5B");
    }

    #[test]
    fn the_address_follows_ollama_host() {
        // Read whatever the environment says without changing it (tests run in parallel).
        let url = base_url();
        assert!(url.ends_with("/v1") && url.starts_with("http") && !url.contains("0.0.0.0"), "{url}");
    }
}
