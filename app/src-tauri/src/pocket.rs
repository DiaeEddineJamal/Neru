//! Pocket Lab: pinned Gallery downloads and isolated local LiteRT / MediaPipe inference.
use std::{fs, io::Read, path::PathBuf, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, sync::Mutex};
use crate::{Hidden, workspace::data_dir};

static DOWNLOAD: Mutex<()> = Mutex::const_new(());
static GENERATION: Mutex<()> = Mutex::const_new(());
static SETUP: Mutex<()> = Mutex::const_new(());
static PAUSE: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
const CATALOG: &str = include_str!("../../src/lib/pocket/gallery-models.json");
const WORKER: &str = include_str!("pocket_runtime.py");

fn root() -> Result<PathBuf, String> { let path = data_dir()?.join("pocket-lab"); fs::create_dir_all(&path).map_err(|e| e.to_string())?; Ok(path) }
fn python() -> Result<PathBuf, String> { Ok(root()?.join(if cfg!(windows) { "runtime/Scripts/python.exe" } else { "runtime/bin/python" })) }
fn catalog() -> Vec<Value> { serde_json::from_str::<Value>(CATALOG).unwrap()["models"].as_array().unwrap().clone() }
fn id(model: &Value) -> String {
    match model["name"].as_str().unwrap() {
        "Gemma-4-E2B-it" => "gemma-4-e2b".into(), "Gemma-4-E4B-it" => "gemma-4-e4b".into(), "Gemma3-1B-IT" => "gemma-3-1b".into(),
        "Gemma-3n-E2B-it" => "gemma-3n-e2b".into(), "Gemma-3n-E4B-it" => "gemma-3n-e4b".into(),
        name => name.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect(),
    }
}
fn model(id_value: &str) -> Result<Value, String> { catalog().into_iter().find(|m| id(m) == id_value).ok_or("Unknown Pocket Lab model.".into()) }
fn model_info(m: &Value) -> (String, u64, Option<&str>) {
    match m["name"].as_str().unwrap() {
        "Gemma-4-E2B-it" => ("7fa1d78473894f7e736a21d920c3aa80f950c0db".into(), 2583085056, Some("ab7838cdfc8f77e54d8ca45eadceb20452d9f01e4bfade03e5dce27911b27e42")),
        "Gemma-4-E4B-it" => ("9695417f248178c63a9f318c6e0c56cb917cb837".into(), 3654467584, Some("f335f2bfd1b758dc6476db16c0f41854bd6237e2658d604cbe566bcefd00a7bc")),
        "Magic touch" => (m["commitHash"].as_str().unwrap().into(), 30525312, Some("38431bc66b883404e8397f74c3579404315b9b52b04a46c6346fe906a7309b03")),
        _ => (m["commitHash"].as_str().unwrap().into(), m["sizeInBytes"].as_u64().unwrap(), None),
    }
}
fn path(m: &Value) -> Result<PathBuf, String> { let directory = root()?.join("models"); fs::create_dir_all(&directory).map_err(|e| e.to_string())?; Ok(directory.join(m["modelFile"].as_str().unwrap())) }
fn verify(path: &PathBuf, hash: &str) -> Result<bool, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); let mut buffer = vec![0u8; 1024 * 1024];
    loop { let n = file.read(&mut buffer).map_err(|e| e.to_string())?; if n == 0 { break; } digest.update(&buffer[..n]); }
    Ok(format!("{:x}", digest.finalize()) == hash)
}
fn env(command: &mut tokio::process::Command, directory: &PathBuf) {
    for key in ["TEMP", "TMP", "TMPDIR", "UV_CACHE_DIR", "UV_PYTHON_INSTALL_DIR", "PIP_CACHE_DIR", "PYTHONPYCACHEPREFIX", "MPLCONFIGDIR", "HF_HOME"] {
        let location = directory.join(key.to_ascii_lowercase()); let _ = fs::create_dir_all(&location); command.env(key, location);
    }
    command.env("UV_LINK_MODE", "copy").env("PYTHONUTF8", "1").current_dir(directory);
}

#[tauri::command]
pub fn pocket_status() -> Result<Value, String> {
    let rows = catalog().iter().map(|m| { let p = path(m)?; let (_, bytes, _) = model_info(m); Ok(json!({ "id": id(m), "ready": fs::metadata(&p).is_ok_and(|s| s.len() == bytes), "partial": fs::metadata(p.with_extension("partial")).map(|s| s.len()).unwrap_or(0) })) }).collect::<Result<Vec<_>, String>>()?;
    Ok(json!({ "runtime": python()?.exists() && root()?.join("runtime-ready").exists(), "models": rows }))
}

#[tauri::command]
pub async fn pocket_setup(app: AppHandle) -> Result<Value, String> {
    let _guard = SETUP.try_lock().map_err(|_| "Runtime setup is already running.")?;
    if !cfg!(all(windows, target_arch = "x86_64")) { return Err("Automatic runtime setup currently supports Windows x64.".into()); }
    let directory = root()?;
    let _ = app.emit("pocket://setup", "Preparing the local runtime…");
    let uv = directory.join("uv.exe");
    if !uv.exists() {
        let bytes = reqwest::Client::new().get("https://github.com/astral-sh/uv/releases/download/0.12.23/uv-x86_64-pc-windows-msvc.zip").send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(&bytes)) != "75d05de6762778c31ee183398de7dd15093fad0ed90b1f236d8205ea5ec00c90" { return Err("Runtime installer checksum failed.".into()); }
        let archive = directory.join("uv.zip"); fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let script = format!("Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force", archive.display().to_string().replace('\'', "''"), directory.display().to_string().replace('\'', "''"));
        let output = tokio::process::Command::new("powershell.exe").hidden().args(["-NoProfile", "-NonInteractive", "-Command", &script]).output().await.map_err(|e| e.to_string())?;
        if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into()); }
    }
    for arguments in [vec!["venv".to_string(), directory.join("runtime").display().to_string(), "--python".into(), "3.13".into(), "--managed-python".into()], vec!["pip".into(), "install".into(), "--python".into(), python()?.display().to_string(), "litert-lm-api==0.17.1".into()]] {
        let mut command = tokio::process::Command::new(&uv).hidden(); env(&mut command, &directory);
        let output = command.args(arguments).output().await.map_err(|e| e.to_string())?;
        if !output.status.success() { return Err(format!("Runtime setup failed: {}", String::from_utf8_lossy(&output.stderr))); }
    }
    fs::write(directory.join("runtime-ready"), "LiteRT-LM 0.17.1 · MediaPipe 1.0.1").map_err(|e| e.to_string())?;
    let _ = app.emit("pocket://setup", "Runtime ready");
    pocket_status()
}

#[tauri::command]
pub async fn pocket_download(model_id: String, token: String, app: AppHandle) -> Result<(), String> {
    let _guard = DOWNLOAD.try_lock().map_err(|_| "Finish or pause the current download first.")?;
    PAUSE.store(false, Ordering::SeqCst);
    let m = model(&model_id)?; let (revision, bytes, pinned) = model_info(&m);
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let url = m["url"].as_str().map(str::to_string).unwrap_or_else(|| format!("https://huggingface.co/{}/resolve/{revision}/{}", m["modelId"].as_str().unwrap(), m["modelFile"].as_str().unwrap()));
    let hash = if let Some(pinned) = pinned { pinned.to_string() } else {
        let mut req = client.get(format!("https://huggingface.co/api/models/{}/revision/{revision}?blobs=true", m["modelId"].as_str().unwrap()));
        if !token.trim().is_empty() { req = req.bearer_auth(token.trim()); }
        let body = req.send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| format!("Accept this model's Hugging Face terms and add a read token. {e}"))?.json::<Value>().await.map_err(|e| e.to_string())?;
        body["siblings"].as_array().and_then(|rows| rows.iter().find(|r| r["rfilename"] == m["modelFile"])).and_then(|r| r["lfs"]["sha256"].as_str()).ok_or("Model checksum unavailable.")?.to_string()
    };
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) { return Err("Invalid model checksum.".into()); }
    let target = path(&m)?; let partial = target.with_extension("partial");
    let mut loaded = fs::metadata(&partial).map(|s| s.len()).unwrap_or(0);
    if loaded == bytes && verify(&partial, &hash)? { fs::rename(&partial, &target).map_err(|e| e.to_string())?; return Ok(()); }
    if loaded >= bytes { fs::remove_file(&partial).map_err(|e| e.to_string())?; loaded = 0; }
    let mut request = client.get(url);
    if !token.trim().is_empty() && m["url"].is_null() { request = request.bearer_auth(token.trim()); }
    if loaded > 0 { request = request.header(reqwest::header::RANGE, format!("bytes={loaded}-")); }
    let mut response = request.send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
    let append = loaded > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !append { loaded = 0; }
    let mut file = tokio::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&partial).await.map_err(|e| e.to_string())?;
    let mut last = std::time::Instant::now();
    loop {
        if PAUSE.load(Ordering::SeqCst) { file.flush().await.map_err(|e| e.to_string())?; return Ok(()); }
        let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), response.chunk()).await.map_err(|_| "Download timed out. Resume to continue.")?.map_err(|e| e.to_string())? else { break; };
        loaded += chunk.len() as u64; if loaded > bytes { return Err("Unexpected model download size.".into()); }
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        if last.elapsed() > Duration::from_millis(150) { let _ = app.emit("pocket://download", json!({ "id": model_id, "loaded": loaded, "total": bytes, "phase": "downloading" })); last = std::time::Instant::now(); }
    }
    file.flush().await.map_err(|e| e.to_string())?; drop(file);
    let _ = app.emit("pocket://download", json!({ "id": model_id, "loaded": loaded, "total": bytes, "phase": "verifying" }));
    let check = partial.clone();
    if loaded != bytes || !tokio::task::spawn_blocking(move || verify(&check, &hash)).await.map_err(|e| e.to_string())?? { fs::remove_file(&partial).map_err(|e| e.to_string())?; return Err("The download failed its integrity check. Download it again.".into()); }
    fs::rename(partial, target).map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn pocket_vision_model() -> Result<tauri::ipc::Response, String> {
    let m = model("magic-touch")?; let p = path(&m)?;
    let (_, size, hash) = model_info(&m);
    if fs::metadata(&p).map(|s| s.len()).unwrap_or(0) != size || !verify(&p, hash.unwrap())? { return Err("Download Magic Touch first.".into()); }
    Ok(tauri::ipc::Response::new(fs::read(p).map_err(|e| e.to_string())?))
}
#[tauri::command]
pub fn pocket_pause() { PAUSE.store(true, Ordering::SeqCst); }
#[tauri::command]
pub fn pocket_cancel() { CANCEL.store(true, Ordering::SeqCst); }
#[tauri::command]
pub fn pocket_remove(model_id: String) -> Result<(), String> {
    let _download = DOWNLOAD.try_lock().map_err(|_| "Pause the download first.")?;
    let _generation = GENERATION.try_lock().map_err(|_| "Stop generation first.")?;
    let p = path(&model(&model_id)?)?;
    for p in [p.clone(), p.with_extension("partial")] { if p.exists() { fs::remove_file(p).map_err(|e| e.to_string())?; } }
    Ok(())
}

fn validate_config(m: &Value, config: &Value) -> Result<(), String> {
    let max = m["defaultConfig"]["maxContextLength"].as_u64().or_else(|| m["defaultConfig"]["maxTokens"].as_u64()).unwrap_or(4096);
    let integer = |key: &str, low: u64, high: u64| config[key].as_u64().is_some_and(|v| v >= low && v <= high);
    let number = |key: &str, low: f64, high: f64| config[key].as_f64().is_some_and(|v| v.is_finite() && v >= low && v <= high);
    if !integer("contextTokens", 512, max) || !integer("maxTokens", 1, config["contextTokens"].as_u64().unwrap_or(0)) || !integer("topK", 1, 128) || !number("topP", 0.00001, 1.0) || !number("temperature", 0.0, 2.0) || config["systemPrompt"].as_str().is_none_or(|s| s.len() > 50000) { return Err("Invalid model configuration.".into()); }
    let accelerator = config["accelerator"].as_str().ok_or("Choose CPU or GPU.")?;
    let allowed = m["defaultConfig"]["accelerators"].as_str().unwrap_or("cpu,gpu");
    if !allowed.split(',').any(|s| s == accelerator) { return Err("Unsupported accelerator.".into()); }
    for (setting, capability) in [("thinking", "llm_thinking"), ("speculative", "speculative_decoding")] { if config[setting].as_bool().is_none() || config[setting] == true && !m["capabilities"].as_array().is_some_and(|c| c.contains(&json!(capability))) { return Err(format!("Unsupported {setting} setting.")); } }
    Ok(())
}

#[tauri::command]
pub async fn pocket_run(model_id: String, request_id: String, mut request: Value, app: AppHandle) -> Result<Value, String> {
    let _guard = GENERATION.try_lock().map_err(|_| "Stop the current model first.")?;
    CANCEL.store(false, Ordering::SeqCst);
    let m = model(&model_id)?; validate_config(&m, &request["config"])?;
    let model_path = path(&m)?;
    if fs::metadata(&model_path).map(|s| s.len()).unwrap_or(0) != model_info(&m).1 { return Err("Download this model first.".into()); }
    if !python()?.exists() { return Err("Set up the Pocket Lab runtime first.".into()); }
    let directory = root()?;
    let worker = directory.join("worker.py"); fs::write(&worker, WORKER).map_err(|e| e.to_string())?;
    let cache = directory.join("cache"); fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    request["modelPath"] = json!(model_path); request["cachePath"] = json!(cache);
    if m["modelId"] == "magic_touch" { return Err("Open Magic Touch in the vision lab.".into()); }
    let messages = request["messages"].as_array().filter(|m| !m.is_empty() && m.len() <= 64).ok_or("A user message is required.")?;
    if messages.last().unwrap()["role"] != "user" || messages.iter().any(|m| ![json!("user"), json!("assistant")].contains(&m["role"]) || m["text"].as_str().is_none_or(|s| s.len() > 100000)) { return Err("Invalid conversation messages.".into()); }
    request["mode"] = json!("chat");
    let result = async {
        let mut command = tokio::process::Command::new(python()?).hidden(); env(&mut command, &directory);
        let mut child = command.kill_on_drop(true).args(["-u", &worker.display().to_string()]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(|e| e.to_string())?;
        let mut stdin = child.stdin.take().unwrap(); stdin.write_all(format!("{}\n", request).as_bytes()).await.map_err(|e| e.to_string())?; drop(stdin);
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines(); let mut completed = false; let started = std::time::Instant::now();
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Some(line) = line.map_err(|e| e.to_string())? else { break; };
                    if let Ok(chunk) = serde_json::from_str::<Value>(&line) {
                        if let Some(error) = chunk["error"].as_str() { return Err(error.to_string()); }
                        if chunk["done"] == true || chunk["result"].is_string() { completed = true; }
                        if chunk["text"].is_string() { let _ = app.emit("pocket://token", json!({ "requestId": request_id, "text": chunk["text"], "reasoning": chunk["reasoning"] })); }
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if CANCEL.load(Ordering::SeqCst) { return Ok(json!({ "cancelled": true })); }
                    if started.elapsed() > Duration::from_secs(600) { return Err("The model timed out. Try a smaller model or shorter context.".into()); }
                }
            }
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() || !completed { return Err("The local runtime stopped unexpectedly. Try CPU or a smaller context.".into()); }
        Ok(json!({ "done": true }))
    }.await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_and_config_boundaries() {
        let models = catalog(); assert_eq!(models.len(), 10);
        let names = models.iter().map(id).collect::<std::collections::HashSet<_>>(); assert_eq!(names.len(), 10);
        assert!(model("../../outside").is_err());
        let gemma = model("gemma-4-e2b").unwrap(); let config = json!({ "systemPrompt": "You are helpful.", "maxTokens": 128, "contextTokens": 1024, "topK": 64, "topP": 0.95, "temperature": 0.7, "accelerator": "cpu", "thinking": true, "speculative": true });
        assert!(validate_config(&gemma, &config).is_ok());
        let mut bad = config.clone(); bad["accelerator"] = json!("shell"); assert!(validate_config(&gemma, &bad).is_err());
        bad = config.clone(); bad["maxTokens"] = json!(4096); assert!(validate_config(&gemma, &bad).is_err());
        assert!(validate_config(&model("tinygarden-270m").unwrap(), &config).is_err());
    }
}
