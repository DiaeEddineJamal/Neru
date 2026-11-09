//! The built-in engine: llama.cpp's `llama-server`, downloaded on first use for this computer's
//! graphics (CUDA, Vulkan, Metal or the processor), one process per loaded model, and a small proxy on
//! a fixed local port that the provider layer talks to as an OpenAI-compatible endpoint
//! (`http://127.0.0.1:39281/v1`). The proxy starts the model a request names, sizes its context from
//! the GGUF header and the memory here, lets llama.cpp fit the layers to the graphics card (`--fit`),
//! keeps it warm between turns (the same process keeps its prompt cache) and unloads it when idle.
//! No separate Ollama install is needed; Ollama stays supported as a second backend.
//!
//! Launch defaults follow llama.cpp's own documentation and the ones Atomic Chat (Apache-2.0, a fork of
//! Jan) ships: `--fit on`, `--flash-attn auto` (off on Vulkan), a q8_0 key/value cache, two slots
//! sharing one cache (`-kvu`), and a smaller host prompt cache on machines with little memory. The
//! code here is Neru's own; see THIRD_PARTY_NOTICES.md.

use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The llama.cpp build Neru downloads, and each archive's SHA-256 from its release.
pub const ENGINE_TAG: &str = "b12230";
const ASSETS: &[(&str, &str)] = &[
    ("llama-b12230-bin-win-cpu-x64.zip", "a787fb94d48e6d93aa5809133f5f984360a5713b20d2f9ee1ccc4b22cd9cce56"),
    ("llama-b12230-bin-win-vulkan-x64.zip", "1ae697ed5c08d6f260add738e222f3bce525d79836286575d6a335972d772076"),
    ("llama-b12230-bin-win-cuda-12.4-x64.zip", "29180e373c153ca59e1837872561eeb800f5b60f1e6375653f8b7d3df57ab885"),
    ("cudart-llama-bin-win-cuda-12.4-x64.zip", "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6"),
    ("llama-b12230-bin-win-cpu-arm64.zip", "cefdf6b94ab7cf07c2bcd17401ceb59a5cdaaedc32424f9303c6d4772556f221"),
    ("llama-b12230-bin-win-vulkan-arm64.zip", "76681a30db5a2ce6bbe7a50c13808ec659fef5135cef53487045c1ca227acca2"),
    ("llama-b12230-bin-macos-arm64.tar.gz", "036052dfc498919646abfa3635be73a0e1a5e740d8daf5322c73d2769f89aeff"),
    ("llama-b12230-bin-macos-x64.tar.gz", "89e4520401bb9820b8719cb1c7c7133057bb429c7e894886009fa3b72f558f39"),
    ("llama-b12230-bin-ubuntu-x64.tar.gz", "a57aa1817e135237b3b087a012ccfe17140e440dbe42f51dab1b9dc68499e7c4"),
    ("llama-b12230-bin-ubuntu-vulkan-x64.tar.gz", "21b79a7d9a2ed215a6e75afb2216ef8239152b9a1bfe523805bcd5bfbbd94492"),
    ("llama-b12230-bin-ubuntu-arm64.tar.gz", "4c544397fc4963b0013d23483172b27707631b356f3669a570892c9f8c8033d0"),
    ("llama-b12230-bin-ubuntu-vulkan-arm64.tar.gz", "5750db73e96ab1aa5080baa5cda654941e4eb9c03f779df74c5faf3407ba0506"),
];

/// The proxy's port. Fixed, so a model's base URL never changes; `NERU_BUILTIN_PORT` moves it.
pub fn port() -> u16 {
    std::env::var("NERU_BUILTIN_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(39281)
}

pub fn base_url() -> String {
    format!("http://127.0.0.1:{}/v1", port())
}

// ---------- the runtime ----------

/// The archives for a backend on this platform, the runtime first.
pub fn assets_for(backend: &str, os: &str, arch: &str) -> Vec<String> {
    let arch = if arch == "aarch64" { "arm64" } else { "x64" };
    let tag = ENGINE_TAG;
    match (os, backend) {
        ("windows", "cuda") if arch == "x64" => vec![format!("llama-{tag}-bin-win-cuda-12.4-x64.zip"), "cudart-llama-bin-win-cuda-12.4-x64.zip".into()],
        ("windows", "vulkan") => vec![format!("llama-{tag}-bin-win-vulkan-{arch}.zip")],
        ("windows", _) => vec![format!("llama-{tag}-bin-win-cpu-{arch}.zip")],
        ("macos", _) => vec![format!("llama-{tag}-bin-macos-{arch}.tar.gz")],
        (_, "vulkan" | "cuda") => vec![format!("llama-{tag}-bin-ubuntu-vulkan-{arch}.tar.gz")],
        _ => vec![format!("llama-{tag}-bin-ubuntu-{arch}.tar.gz")],
    }
}

fn checksum(asset: &str) -> Option<&'static str> {
    ASSETS.iter().find(|(name, _)| *name == asset).map(|(_, sha)| *sha)
}

fn engine_root() -> Result<PathBuf, String> {
    let dir = crate::workspace::data_dir()?.join("local").join("engine");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn server_name() -> &'static str {
    if cfg!(windows) { "llama-server.exe" } else { "llama-server" }
}

/// `llama-server` somewhere under `dir` (archives put it at the top or in `build/bin`).
fn find_server(dir: &Path) -> Option<PathBuf> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name() == server_name() {
                return Some(path);
            }
        }
    }
    None
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub tag: String,
    pub backend: String,
    pub path: String,
}

/// The installed runtime: `NERU_LLAMA_SERVER` (a llama-server the user built), else the one Neru
/// downloaded for `backend`, else any it downloaded.
pub fn installed_engine(backend: Option<&str>) -> Option<EngineInfo> {
    if let Some(path) = std::env::var_os("NERU_LLAMA_SERVER").map(PathBuf::from).filter(|path| path.is_file()) {
        return Some(EngineInfo { tag: "custom".into(), backend: "custom".into(), path: path.to_string_lossy().into() });
    }
    let root = engine_root().ok()?;
    let mut found: Vec<EngineInfo> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let (tag, backend) = name.split_once('-')?;
            let path = find_server(&entry.path())?;
            Some(EngineInfo { tag: tag.into(), backend: backend.into(), path: path.to_string_lossy().into() })
        })
        .collect();
    found.sort_by_key(|info| (info.tag != ENGINE_TAG, Some(info.backend.as_str()) != backend));
    found.into_iter().next()
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineProgress {
    pub backend: String,
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

static INSTALLING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

/// Downloads and unpacks the runtime for `backend` (auto: what fits this computer), checking each
/// archive's SHA-256. Progress goes out on `builtin://engine`.
#[tauri::command]
pub async fn builtin_install_engine(backend: Option<String>, app: AppHandle) -> Result<EngineInfo, String> {
    let hardware = tauri::async_runtime::spawn_blocking(crate::hardware::detect).await.map_err(|e| e.to_string())?;
    let backend = backend.filter(|b| matches!(b.as_str(), "cpu" | "vulkan" | "cuda" | "metal")).unwrap_or_else(|| crate::hardware::engine_backend(&hardware).to_string());
    install(&backend, Some(&app)).await
}

pub async fn install(backend: &str, app: Option<&AppHandle>) -> Result<EngineInfo, String> {
    let _guard = INSTALLING.lock().await;
    if let Some(info) = installed_engine(Some(backend)).filter(|info| info.tag == ENGINE_TAG && info.backend == backend) {
        return Ok(info);
    }
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let assets = assets_for(backend, os, std::env::consts::ARCH);
    let dir = engine_root()?.join(format!("{ENGINE_TAG}-{backend}"));
    let staging = engine_root()?.join(format!(".{ENGINE_TAG}-{backend}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let mut progress = EngineProgress { backend: backend.into(), status: "downloading".into(), ..EngineProgress::default() };
    let emit = |progress: &EngineProgress| {
        if let Some(app) = app {
            let _ = app.emit("builtin://engine", progress);
        }
    };
    let result: Result<(), String> = async {
        for asset in &assets {
            let url = format!("https://github.com/ggml-org/llama.cpp/releases/download/{ENGINE_TAG}/{asset}");
            let archive = staging.join(asset);
            let mut response = crate::providers::http().get(&url).header("User-Agent", "Neru").send().await.map_err(|e| format!("GitHub could not be reached: {e}"))?;
            if !response.status().is_success() {
                return Err(format!("Downloading {asset} failed: HTTP {}", response.status()));
            }
            let size = response.content_length().unwrap_or(0);
            progress.total += size;
            let mut file = tokio::fs::File::create(&archive).await.map_err(|e| e.to_string())?;
            let mut last = Instant::now();
            while let Some(chunk) = response.chunk().await.map_err(|e| format!("The download stopped: {e}"))? {
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                progress.completed += chunk.len() as u64;
                if last.elapsed() > Duration::from_millis(250) {
                    emit(&progress);
                    last = Instant::now();
                }
            }
            file.flush().await.map_err(|e| e.to_string())?;
            drop(file);
            progress.status = "verifying".into();
            emit(&progress);
            if let Some(expected) = checksum(asset) {
                let path = archive.clone();
                let actual = tokio::task::spawn_blocking(move || crate::llama_models::sha256_file(&path)).await.map_err(|e| e.to_string())??;
                if actual != expected {
                    return Err(format!("{asset} does not match its published checksum."));
                }
            }
            progress.status = "unpacking".into();
            emit(&progress);
            unpack(&archive, &staging).await?;
            let _ = std::fs::remove_file(&archive);
            progress.status = "downloading".into();
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        progress.error = Some(error.clone());
        progress.done = true;
        emit(&progress);
        return Err(error);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| e.to_string())?;
    let path = find_server(&dir).ok_or("The llama.cpp archive has no llama-server.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    progress.status = "done".into();
    progress.done = true;
    emit(&progress);
    Ok(EngineInfo { tag: ENGINE_TAG.into(), backend: backend.into(), path: path.to_string_lossy().into() })
}

/// Unpacks a .zip or .tar.gz with the system's `tar` (bsdtar on Windows 10 and later reads zip too).
async fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    use crate::Hidden;
    let mut command = std::process::Command::new("tar");
    // -m: no modification times, which exFAT drives refuse to restore.
    command.arg("-xmf").arg(archive).arg("-C").arg(into).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut command = command.hidden();
    let output = tokio::task::spawn_blocking(move || command.output()).await.map_err(|e| e.to_string())?.map_err(|e| format!("tar could not run: {e}"))?;
    if !output.status.success() {
        return Err(format!("Unpacking failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

// ---------- loaded models ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineSettings {
    /// Minutes a model stays loaded after its last request; 0 keeps it loaded until Neru quits.
    pub keep_warm_minutes: u32,
    /// 0 picks the context from memory; otherwise this many tokens (capped by the model).
    pub context: u32,
    /// auto, cpu, vulkan, cuda, metal.
    pub backend: String,
    /// Models loaded at once (an agent model and a helper, say); the least recently used makes room.
    pub max_loaded: u32,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self { keep_warm_minutes: 15, context: 0, backend: "auto".into(), max_loaded: 2 }
    }
}

fn settings_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::workspace::data_dir().ok().map(|dir| dir.join("local").join("engine-settings.json"))
}

pub fn settings() -> EngineSettings {
    settings_path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

struct Loaded {
    id: String,
    port: u16,
    child: Child,
    context: u32,
    backend: String,
    gpu_layers: Option<String>,
    last_used: Instant,
    in_flight: u32,
    log: Arc<Mutex<VecDeque<String>>>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoadedInfo {
    pub id: String,
    pub port: u16,
    pub context: u32,
    pub backend: String,
    /// What llama.cpp reported, such as "offloaded 29/29 layers to GPU".
    pub gpu_layers: Option<String>,
    pub idle_seconds: u64,
    pub busy: bool,
}

static LOADED: LazyLock<tokio::sync::Mutex<HashMap<String, Loaded>>> = LazyLock::new(Default::default);
/// One start at a time, so two requests for the same model do not start it twice.
static STARTING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|listener| listener.local_addr()).map(|address| address.port()).map_err(|e| e.to_string())
}

/// Host prompt-cache size: llama.cpp keeps 8 GB of earlier prompts by default, too much for a small machine.
fn cache_ram_mib(ram_gb: f32) -> Option<u32> {
    if ram_gb <= 8.5 {
        Some(1024)
    } else if ram_gb <= 16.5 {
        Some(2048)
    } else {
        None
    }
}

/// The command line for one model.
pub fn launch_args(model_path: &str, id: &str, port: u16, context: u32, backend: &str, ram_gb: f32, cpu_only: bool) -> Vec<String> {
    let mut args: Vec<String> = ["--no-webui", "--jinja", "-m", model_path, "-a", id, "--host", "127.0.0.1", "--port", &port.to_string(), "-c", &context.to_string(), "--fit", "on"].iter().map(|s| s.to_string()).collect();
    // Flash attention lets the value cache be quantized too; Vulkan builds are steadier without it.
    let flash = if backend == "vulkan" { "off" } else { "auto" };
    args.extend(["--flash-attn".into(), flash.into(), "--cache-type-k".into(), "q8_0".into()]);
    if flash != "off" {
        args.extend(["--cache-type-v".into(), "q8_0".into()]);
    }
    // Two slots sharing one cache: a chore can run beside the conversation without halving its window.
    args.extend(["--parallel".into(), "2".into(), "-kvu".into()]);
    // Reuse the cached prefix when only the middle of a long conversation changed (after a summary).
    args.extend(["--cache-reuse".into(), "256".into()]);
    if let Some(mib) = cache_ram_mib(ram_gb) {
        args.extend(["--cache-ram".into(), mib.to_string()]);
    }
    if cpu_only {
        args.extend(["-ngl".into(), "0".into()]);
    }
    args
}

fn engine_for_settings(hardware: &crate::hardware::Hardware) -> String {
    let chosen = settings().backend;
    if matches!(chosen.as_str(), "cpu" | "vulkan" | "cuda" | "metal") { chosen } else { crate::hardware::engine_backend(hardware).to_string() }
}

/// Starts `id` unless it is loaded, making room first. Returns its port.
pub async fn ensure_loaded(id: &str) -> Result<u16, String> {
    {
        let mut loaded = LOADED.lock().await;
        if let Some(model) = loaded.get_mut(id) {
            if model.child.try_wait().ok().flatten().is_none() {
                model.last_used = Instant::now();
                return Ok(model.port);
            }
            loaded.remove(id);
        }
    }
    let _starting = STARTING.lock().await;
    if let Some(model) = LOADED.lock().await.get_mut(id) {
        model.last_used = Instant::now();
        return Ok(model.port);
    }
    start(id, None).await
}

async fn start(id: &str, context_override: Option<u32>) -> Result<u16, String> {
    let model = crate::llama_models::find(id).ok_or_else(|| format!("{id} is not downloaded. Download it in Settings → Model provider → Built-in."))?;
    let hardware = tauri::async_runtime::spawn_blocking(crate::hardware::detect).await.map_err(|e| e.to_string())?;
    let backend = engine_for_settings(&hardware);
    let engine = match installed_engine(Some(&backend)) {
        Some(engine) => engine,
        None => install(&backend, None).await?,
    };
    let path = model.path.clone();
    let info = tauri::async_runtime::spawn_blocking(move || crate::gguf::read(Path::new(&path))).await.map_err(|e| e.to_string())??;
    let weights_gb = model.bytes as f32 / 1_073_741_824.0;
    let wanted = context_override.or_else(|| Some(settings().context).filter(|c| *c >= 2048));
    let context = match wanted {
        Some(context) => context.min(if info.context_length > 0 { info.context_length } else { context }),
        None => crate::hardware::plan_context(&hardware, weights_gb, info.kv_bytes_per_token(8.5 / 8.0), info.context_length),
    };
    // Room for the new model: drop the least recently used ones past the limit.
    make_room(settings().max_loaded.max(1) as usize - 1).await;
    let mut attempt = 0;
    loop {
        let cpu_only = attempt > 0;
        let port = free_port()?;
        let args = launch_args(&model.path, &model.id, port, context, &engine.backend, hardware.ram_gb, cpu_only);
        match spawn_and_wait(&engine, &args, port).await {
            Ok((child, log)) => {
                let gpu_layers = log.lock().ok().and_then(|lines| lines.iter().rev().find(|line| line.contains("offloaded") && line.contains("layers")).map(|line| line.split("offloaded").nth(1).map(|rest| format!("offloaded{rest}")).unwrap_or_default()));
                // llama.cpp may have chosen a smaller window to fit; /props has the real one.
                let actual = props_context(port).await.unwrap_or(context);
                LOADED.lock().await.insert(model.id.clone(), Loaded { id: model.id.clone(), port, child, context: actual, backend: if cpu_only { "cpu".into() } else { engine.backend.clone() }, gpu_layers, last_used: Instant::now(), in_flight: 0, log });
                remember_pids().await;
                crate::fallback::models_changed("builtin");
                return Ok(port);
            }
            // A graphics backend that fails to start (an old driver, say) gets one try on the processor.
            Err(error) if attempt == 0 && !matches!(engine.backend.as_str(), "cpu" | "custom") => {
                log::warn!("llama-server on {} failed, retrying on the processor: {error}", engine.backend);
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn spawn_and_wait(engine: &EngineInfo, args: &[String], port: u16) -> Result<(Child, Arc<Mutex<VecDeque<String>>>), String> {
    use crate::Hidden;
    let mut command = std::process::Command::new(&engine.path);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = Path::new(&engine.path).parent() {
        command.current_dir(dir);
    }
    let mut command = command.hidden();
    let mut child = command.spawn().map_err(|e| format!("llama-server could not start: {e}"))?;
    tie_to_neru(&child);
    let log = Arc::new(Mutex::new(VecDeque::with_capacity(200)));
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
        let log = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Ok(mut log) = log.lock() {
                    if log.len() >= 200 {
                        log.pop_front();
                    }
                    log.push_back(line);
                }
            }
        });
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    let health = format!("http://127.0.0.1:{port}/health");
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let tail = log.lock().map(|lines| lines.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n")).unwrap_or_default();
            return Err(format!("llama-server stopped ({status}) while loading the model:\n{tail}"));
        }
        if let Ok(response) = crate::providers::http().get(&health).timeout(Duration::from_secs(2)).send().await {
            if response.status().is_success() {
                return Ok((child, log));
            }
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("The model took more than five minutes to load.".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// On Windows, puts the process in a job that closes with Neru, so a model never keeps its memory after
/// Neru quits or crashes (the command line included). Elsewhere the leftovers check on the next start
/// covers a crash.
#[cfg(windows)]
fn tie_to_neru(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject};
    // The job handle stays open for the life of Neru; closing it (at exit) ends every process in it.
    static JOB: LazyLock<usize> = LazyLock::new(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits as *const _ as *const std::ffi::c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
        job as usize
    });
    if *JOB != 0 {
        unsafe {
            AssignProcessToJobObject(*JOB as _, child.as_raw_handle() as _);
        }
    }
}

#[cfg(not(windows))]
fn tie_to_neru(_child: &Child) {}

async fn props_context(port: u16) -> Option<u32> {
    let body: Value = crate::providers::http().get(format!("http://127.0.0.1:{port}/props")).timeout(Duration::from_secs(3)).send().await.ok()?.json().await.ok()?;
    body["default_generation_settings"]["n_ctx"].as_u64().or_else(|| body["n_ctx"].as_u64()).map(|n| n as u32)
}

async fn make_room(keep: usize) {
    let mut loaded = LOADED.lock().await;
    while loaded.len() > keep {
        let Some(oldest) = loaded.values().filter(|model| model.in_flight == 0).min_by_key(|model| model.last_used).map(|model| model.id.clone()) else { break };
        if let Some(mut model) = loaded.remove(&oldest) {
            let _ = model.child.kill();
            let _ = model.child.wait();
        }
    }
}

pub async fn unload(id: &str) {
    if let Some(mut model) = LOADED.lock().await.remove(id) {
        let _ = model.child.kill();
        let _ = model.child.wait();
    }
    remember_pids().await;
}

/// Stops every model, when Neru quits.
pub fn shutdown() {
    if let Ok(mut loaded) = LOADED.try_lock() {
        for (_, mut model) in loaded.drain() {
            let _ = model.child.kill();
        }
    }
    if let Some(path) = pids_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn pids_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::workspace::data_dir().ok().map(|dir| dir.join("local").join("engine-pids.json"))
}

/// Writes the running processes down, so a crashed Neru's leftovers can be stopped next time.
async fn remember_pids() {
    let Some(path) = pids_path() else { return };
    let pids: Vec<(u32, u16)> = LOADED.lock().await.values().map(|model| (model.child.id(), model.port)).collect();
    let _ = std::fs::write(path, serde_json::to_vec(&pids).unwrap_or_default());
}

/// Stops llama-server processes a crashed Neru left behind: ones still answering on the port written down.
async fn stop_leftovers() {
    use crate::Hidden;
    let Some(path) = pids_path() else { return };
    let Ok(bytes) = std::fs::read(&path) else { return };
    let _ = std::fs::remove_file(&path);
    for (pid, port) in serde_json::from_slice::<Vec<(u32, u16)>>(&bytes).unwrap_or_default() {
        let alive = crate::providers::http().get(format!("http://127.0.0.1:{port}/health")).timeout(Duration::from_secs(1)).send().await.is_ok();
        if !alive {
            continue;
        }
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("taskkill");
            command.args(["/PID", &pid.to_string(), "/F"]);
            command
        } else {
            let mut command = std::process::Command::new("kill");
            command.arg(pid.to_string());
            command
        };
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let _ = command.hidden().status();
    }
}

/// Unloads models idle for longer than the keep-warm setting, every half minute.
async fn idle_loop() {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let minutes = settings().keep_warm_minutes;
        let mut stopped = false;
        {
            let mut loaded = LOADED.lock().await;
            let idle: Vec<String> = loaded.values().filter(|model| minutes > 0 && model.in_flight == 0 && model.last_used.elapsed() > Duration::from_secs(minutes as u64 * 60)).map(|model| model.id.clone()).collect();
            for id in idle {
                if let Some(mut model) = loaded.remove(&id) {
                    let _ = model.child.kill();
                    let _ = model.child.wait();
                    stopped = true;
                }
            }
            loaded.retain(|_, model| model.child.try_wait().ok().flatten().is_none());
        }
        if stopped {
            remember_pids().await;
        }
    }
}

async fn mark(id: &str, delta: i32) {
    if let Some(model) = LOADED.lock().await.get_mut(id) {
        model.in_flight = (model.in_flight as i32 + delta).max(0) as u32;
        model.last_used = Instant::now();
    }
}

// ---------- the proxy ----------

static PROXY: LazyLock<Mutex<bool>> = LazyLock::new(Default::default);

/// Starts the proxy once per process. Safe to call before every request. When the port is taken by
/// another Neru (the app and `neru` in a terminal), that one answers instead.
pub fn ensure_proxy() {
    let Ok(mut started) = PROXY.lock() else { return };
    if *started {
        return;
    }
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port())) else {
        // Probably another Neru's proxy; it serves the same models folder.
        *started = true;
        return;
    };
    let _ = listener.set_nonblocking(true);
    *started = true;
    tauri::async_runtime::spawn(async move {
        stop_leftovers().await;
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
        tauri::async_runtime::spawn(idle_loop());
        loop {
            let Ok((socket, _)) = listener.accept().await else { continue };
            tauri::async_runtime::spawn(async move {
                let _ = serve(socket).await;
            });
        }
    });
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Result<Request, String> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = socket.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("closed".into());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if buffer.len() > 64 * 1024 {
            return Err("headers too large".into());
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (method, path) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
    let length = lines.filter_map(|line| line.split_once(':')).find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length")).and_then(|(_, value)| value.trim().parse::<usize>().ok()).unwrap_or(0);
    if length > 64 * 1024 * 1024 {
        return Err("body too large".into());
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let read = socket.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(Request { method, path, body })
}

async fn respond(socket: &mut tokio::net::TcpStream, status: u16, body: &Value) -> Result<(), String> {
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", text.len());
    socket.write_all(head.as_bytes()).await.map_err(|e| e.to_string())?;
    socket.write_all(text.as_bytes()).await.map_err(|e| e.to_string())
}

fn error_body(message: &str) -> Value {
    json!({"error": {"message": message, "type": "neru_builtin"}})
}

/// The downloaded models as an OpenAI model list.
pub fn models_list() -> Value {
    let catalog = crate::llama_models::catalog();
    let data: Vec<Value> = crate::llama_models::installed()
        .iter()
        .map(|model| {
            let context = model.catalog_id.as_ref().and_then(|id| catalog.iter().find(|entry| &entry.id == id)).map(|entry| entry.context_max.min(65_536));
            json!({"id": model.id, "object": "model", "owned_by": "neru-builtin", "name": format!("{} ({})", model.name, model.quant), "context_length": context})
        })
        .collect();
    json!({"object": "list", "data": data})
}

/// The next context to try after a request overflowed: 8K, 32K, then half again, up to what the model knows.
pub fn next_context(current: u32, trained: u32) -> u32 {
    let next = if current < 8192 { 8192 } else if current < 32_768 { 32_768 } else { current + current / 2 };
    next.min(if trained == 0 { 131_072 } else { trained })
}

fn overflowed(status: u16, body: &str) -> bool {
    status == 400 && (body.contains("exceeds the available context size") || body.contains("exceed_context_size") || body.contains("context length"))
}

async fn serve(mut socket: tokio::net::TcpStream) -> Result<(), String> {
    let request = read_request(&mut socket).await?;
    let path = request.path.split('?').next().unwrap_or_default().to_string();
    match (request.method.as_str(), path.as_str()) {
        ("GET", "/health" | "/v1/health") => return respond(&mut socket, 200, &json!({"status": "ok", "engine": "neru-builtin"})).await,
        ("GET", "/v1/models" | "/models") => {
            let list = tokio::task::spawn_blocking(models_list).await.unwrap_or_else(|_| json!({"data": []}));
            return respond(&mut socket, 200, &list).await;
        }
        ("POST", _) if path.starts_with("/v1/") => {}
        _ => return respond(&mut socket, 404, &error_body("Not found")).await,
    }
    let mut body: Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(_) => return respond(&mut socket, 400, &error_body("The request is not JSON.")).await,
    };
    let id = body["model"].as_str().unwrap_or_default().to_string();
    let id = if id.is_empty() || crate::llama_models::find(&id).is_none() {
        // A request without a known model goes to the one loaded, or the only one downloaded.
        let loaded: Vec<String> = LOADED.lock().await.keys().cloned().collect();
        let installed = crate::llama_models::installed();
        match (loaded.first(), installed.as_slice()) {
            (Some(id), _) if id.is_empty() || loaded.len() == 1 => id.clone(),
            (_, [only]) => only.id.clone(),
            _ => return respond(&mut socket, 404, &error_body(&format!("The model '{id}' is not downloaded in Neru."))).await,
        }
    } else {
        id
    };
    body["model"] = json!(id);
    // Keep the conversation's prefix cached in the slot between turns (llama.cpp's default, made explicit).
    body["cache_prompt"] = json!(true);
    if body["stream"] == true && body.get("stream_options").is_none() {
        body["stream_options"] = json!({"include_usage": true});
    }
    let mut retried = false;
    loop {
        let port = match ensure_loaded(&id).await {
            Ok(port) => port,
            Err(error) => return respond(&mut socket, 503, &error_body(&error)).await,
        };
        mark(&id, 1).await;
        let result = forward(&mut socket, port, &path, &body, !retried).await;
        mark(&id, -1).await;
        match result {
            Forwarded::Done => return Ok(()),
            Forwarded::Overflow(text) if !retried => {
                // Reload with a larger window and try once more.
                let (current, trained) = {
                    let loaded = LOADED.lock().await;
                    let current = loaded.get(&id).map_or(8192, |model| model.context);
                    let trained = crate::llama_models::find(&id).and_then(|model| crate::gguf::read(Path::new(&model.path)).ok()).map_or(0, |info| info.context_length);
                    (current, trained)
                };
                let next = next_context(current, trained);
                if next <= current {
                    return respond(&mut socket, 400, &serde_json::from_str(&text).unwrap_or_else(|_| error_body(&text))).await;
                }
                unload(&id).await;
                let _starting = STARTING.lock().await;
                if let Err(error) = start(&id, Some(next)).await {
                    return respond(&mut socket, 503, &error_body(&error)).await;
                }
                retried = true;
            }
            Forwarded::Overflow(text) => return respond(&mut socket, 400, &serde_json::from_str(&text).unwrap_or_else(|_| error_body(&text))).await,
            Forwarded::Failed(error) => return respond(&mut socket, 502, &error_body(&error)).await,
        }
    }
}

enum Forwarded {
    Done,
    Overflow(String),
    Failed(String),
}

/// Sends the request to the model's server and streams its answer back as it comes.
async fn forward(socket: &mut tokio::net::TcpStream, port: u16, path: &str, body: &Value, may_retry: bool) -> Forwarded {
    let response = match crate::providers::http().post(format!("http://127.0.0.1:{port}{path}")).json(body).send().await {
        Ok(response) => response,
        Err(error) => return Forwarded::Failed(format!("The built-in engine did not answer: {error}")),
    };
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        if may_retry && overflowed(status, &text) {
            return Forwarded::Overflow(text);
        }
        let head = format!("HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", text.len());
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(text.as_bytes()).await;
        return Forwarded::Done;
    }
    let content_type = response.headers().get("content-type").and_then(|value| value.to_str().ok()).unwrap_or("application/json").to_string();
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n");
    if socket.write_all(head.as_bytes()).await.is_err() {
        return Forwarded::Done;
    }
    let mut response = response;
    // Each chunk goes out as it arrives, so tokens reach the window without buffering.
    while let Ok(Some(chunk)) = response.chunk().await {
        if socket.write_all(&chunk).await.is_err() {
            // The caller stopped reading (Stop in the window): dropping the response cancels the generation.
            break;
        }
    }
    let _ = socket.flush().await;
    Forwarded::Done
}

// ---------- commands ----------

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinStatus {
    pub engine: Option<EngineInfo>,
    /// The backend this computer would use.
    pub backend: String,
    pub base_url: String,
    pub models: Vec<crate::llama_models::Installed>,
    pub downloading: Vec<String>,
    pub loaded: Vec<LoadedInfo>,
    pub settings: EngineSettings,
    pub speeds: HashMap<String, crate::ollama::Speed>,
    pub models_dir: String,
}

#[tauri::command]
pub async fn builtin_status() -> Result<BuiltinStatus, String> {
    ensure_proxy();
    let hardware = tauri::async_runtime::spawn_blocking(crate::hardware::detect).await.map_err(|e| e.to_string())?;
    let backend = engine_for_settings(&hardware);
    let loaded = LOADED
        .lock()
        .await
        .values()
        .map(|model| LoadedInfo { id: model.id.clone(), port: model.port, context: model.context, backend: model.backend.clone(), gpu_layers: model.gpu_layers.clone(), idle_seconds: model.last_used.elapsed().as_secs(), busy: model.in_flight > 0 })
        .collect();
    let speeds = crate::ollama::speeds().into_iter().filter(|(key, _)| key.starts_with("builtin:")).map(|(key, speed)| (key.trim_start_matches("builtin:").to_string(), speed)).collect();
    Ok(BuiltinStatus {
        engine: installed_engine(Some(&backend)),
        backend,
        base_url: base_url(),
        models: tauri::async_runtime::spawn_blocking(crate::llama_models::installed).await.unwrap_or_default(),
        downloading: crate::llama_models::downloading(),
        loaded,
        settings: settings(),
        speeds,
        models_dir: crate::llama_models::models_dir().map(|dir| dir.to_string_lossy().to_string()).unwrap_or_default(),
    })
}

#[tauri::command]
pub fn set_builtin_settings(settings: EngineSettings) -> Result<EngineSettings, String> {
    let path = settings_path().ok_or("No data folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(settings)
}

/// Loads a model ahead of the first message (warm-up), so the first reply does not wait for it.
#[tauri::command]
pub async fn builtin_load(id: String) -> Result<LoadedInfo, String> {
    ensure_proxy();
    ensure_loaded(&id).await?;
    let loaded = LOADED.lock().await;
    let model = loaded.get(&id).ok_or("The model stopped right after loading.")?;
    Ok(LoadedInfo { id: model.id.clone(), port: model.port, context: model.context, backend: model.backend.clone(), gpu_layers: model.gpu_layers.clone(), idle_seconds: 0, busy: false })
}

#[tauri::command]
pub async fn builtin_unload(id: String) -> Result<(), String> {
    unload(&id).await;
    Ok(())
}

/// The last lines llama-server wrote for a loaded model, for the details view and bug reports.
#[tauri::command]
pub async fn builtin_log(id: String) -> Vec<String> {
    LOADED.lock().await.get(&id).and_then(|model| model.log.lock().ok().map(|log| log.iter().cloned().collect())).unwrap_or_default()
}

/// Measures a model with llama.cpp's own timings: about a thousand tokens to read and 64 to write.
#[tauri::command]
pub async fn builtin_speed(id: String) -> Result<crate::ollama::Speed, String> {
    let port = ensure_loaded(&id).await?;
    mark(&id, 1).await;
    let payload = json!({"model": id, "messages": [{"role": "user", "content": crate::ollama::speed_prompt()}], "max_tokens": 64, "temperature": 0, "stream": false, "cache_prompt": false});
    let result = crate::providers::http().post(format!("http://127.0.0.1:{port}/v1/chat/completions")).json(&payload).timeout(Duration::from_secs(600)).send().await;
    mark(&id, -1).await;
    let body = crate::providers::read_response(result.map_err(|e| e.to_string())?).await?;
    let speed = speed_from_timings(&body["timings"]).ok_or("llama.cpp did not report its timings.")?;
    crate::ollama::save_speed(&format!("builtin:{id}"), &speed);
    Ok(speed)
}

pub fn speed_from_timings(timings: &Value) -> Option<crate::ollama::Speed> {
    let prompt = timings["prompt_per_second"].as_f64()? as f32;
    let generate = timings["predicted_per_second"].as_f64()? as f32;
    if prompt <= 0.0 || generate <= 0.0 {
        return None;
    }
    let (verdict, sentence) = crate::ollama::judge(prompt, generate);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Some(crate::ollama::Speed { prompt_per_second: prompt, generate_per_second: generate, load_seconds: 0.0, verdict: verdict.into(), sentence, measured_at: now })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_gets_a_pinned_archive_with_a_checksum() {
        for (os, backend, arch) in [("windows", "cpu", "x86_64"), ("windows", "vulkan", "x86_64"), ("windows", "cuda", "x86_64"), ("macos", "metal", "aarch64"), ("macos", "cpu", "x86_64"), ("linux", "vulkan", "x86_64"), ("linux", "cpu", "x86_64"), ("linux", "cuda", "x86_64")] {
            let assets = assets_for(backend, os, arch);
            assert!(!assets.is_empty());
            for asset in assets {
                assert!(checksum(&asset).is_some(), "{os} {backend}: {asset}");
            }
        }
        assert_eq!(assets_for("cuda", "windows", "x86_64").len(), 2, "CUDA needs its runtime archive too");
    }

    #[test]
    fn launch_flags_follow_the_backend_and_memory() {
        let args = launch_args("m.gguf", "gemma-3-1b:Q4_K_M", 4000, 16_384, "cuda", 32.0, false);
        let joined = args.join(" ");
        assert!(joined.contains("--fit on") && joined.contains("-c 16384") && joined.contains("--flash-attn auto") && joined.contains("--cache-type-v q8_0"), "{joined}");
        assert!(!joined.contains("--cache-ram") && !joined.contains("-ngl"), "{joined}");
        assert!(!joined.contains("mmap") && !joined.contains("mlock"), "removed from llama-server: {joined}");
        let vulkan = launch_args("m.gguf", "x", 4000, 8192, "vulkan", 8.0, true).join(" ");
        assert!(vulkan.contains("--flash-attn off") && !vulkan.contains("--cache-type-v") && vulkan.contains("--cache-ram 1024") && vulkan.contains("-ngl 0"), "{vulkan}");
    }

    #[test]
    fn context_grows_in_steps_up_to_what_the_model_knows() {
        assert_eq!(next_context(4096, 131_072), 8192);
        assert_eq!(next_context(8192, 131_072), 32_768);
        assert_eq!(next_context(32_768, 131_072), 49_152);
        assert_eq!(next_context(32_768, 40_960), 40_960);
        assert!(overflowed(400, r#"{"error":{"type":"exceed_context_size_error"}}"#));
        assert!(!overflowed(500, "exceeds the available context size"));
    }

    #[test]
    fn timings_become_a_speed_verdict() {
        let speed = speed_from_timings(&json!({"prompt_per_second": 420.0, "predicted_per_second": 31.0})).unwrap();
        assert_eq!(speed.verdict, "agent");
        assert!(speed_from_timings(&json!({})).is_none());
    }

    /// Downloads the real engine and a 0.4 GB model, then chats through the proxy as the provider layer
    /// does. Run with `cargo test --lib live_built_in -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_built_in_engine_chat() {
        tauri::async_runtime::block_on(async {
            let started = Instant::now();
            let engine = install("cpu", None).await.expect("engine");
            println!("engine {:?} after {:?}", engine, started.elapsed());
            let model = crate::llama_models::download_model("unsloth/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q4_K_M.gguf", None).await.expect("model");
            println!("model {} ({} bytes) after {:?}", model.id, model.bytes, started.elapsed());
            ensure_proxy();
            let client = crate::providers::http();
            let list: Value = client.get(format!("{}/models", base_url())).send().await.unwrap().json().await.unwrap();
            println!("models {list}");
            assert!(list["data"].as_array().unwrap().iter().any(|item| item["id"] == model.id));
            let payload = json!({"model": model.id, "stream": true, "max_tokens": 200, "messages": [{"role": "system", "content": "Answer in one short sentence."}, {"role": "user", "content": "What is the capital of France? /no_think"}]});
            let mut response = client.post(format!("{}/chat/completions", base_url())).json(&payload).send().await.unwrap();
            assert!(response.status().is_success(), "{}", response.status());
            let (mut text, mut timings, mut raw) = (String::new(), Value::Null, String::new());
            while let Some(chunk) = response.chunk().await.unwrap() {
                raw.push_str(&String::from_utf8_lossy(&chunk));
            }
            for line in raw.lines().filter_map(|line| line.strip_prefix("data: ")) {
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
                        text.push_str(delta);
                    }
                    if value["timings"].is_object() {
                        timings = value["timings"].clone();
                    }
                }
            }
            println!("reply: {text:?} timings: {timings}");
            assert!(text.to_lowercase().contains("paris"), "{raw}");
            let loaded = LOADED.lock().await.get(&model.id).map(|m| (m.context, m.backend.clone(), m.gpu_layers.clone()));
            println!("loaded {loaded:?} after {:?}", started.elapsed());
            // Second turn on the same prefix: the slot's cache should be reused.
            let mut second = payload.clone();
            second["stream"] = json!(false);
            let body: Value = client.post(format!("{}/chat/completions", base_url())).json(&second).send().await.unwrap().json().await.unwrap();
            println!("second turn timings: {}", body["timings"]);
            assert!(body["timings"]["cache_n"].as_u64().unwrap_or(0) > 0, "prompt cache reused: {body}");
            shutdown();
        });
    }
}
