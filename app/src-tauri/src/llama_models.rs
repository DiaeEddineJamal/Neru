//! GGUF models for the built-in engine: the catalog (in the model registry, with each quantization's
//! file, size and SHA-256 from Hugging Face), downloads with progress, resume, cancel and a checksum,
//! and removal. Files live under the data folder in `local/models/<repo>/<file>.gguf`.

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use tauri::{AppHandle, Emitter};

/// One quantization of a catalog model.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GgufFile {
    /// Q8_0, Q6_K, Q5_K_M, Q4_K_M, IQ4_XS, Q3_K_M.
    pub quant: String,
    pub file: String,
    pub bytes: u64,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A model in the registry's `ggufModels`.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GgufModel {
    pub id: String,
    pub name: String,
    /// The Hugging Face repository, such as `unsloth/Qwen3-4B-Instruct-2507-GGUF`.
    pub repo: String,
    /// agent (drives Code), chat, helper (chores).
    pub roles: Vec<String>,
    pub params_b: f32,
    /// Parameters read for each token: all of them when dense, the active experts in a mixture.
    pub active_b: f32,
    pub layers: u32,
    /// The key/value cache for one token at f16, in KB.
    pub kv_kb_per_token: f32,
    pub context_max: u32,
    #[serde(default)]
    pub note: String,
    pub files: Vec<GgufFile>,
}

/// The catalog. A registry fetched from an older release may not have it, so the built-in one fills in.
pub fn catalog() -> Vec<GgufModel> {
    let current = crate::registry::get().gguf_models;
    if !current.is_empty() {
        return current;
    }
    crate::registry::built_in_gguf()
}

/// `gemma-3-1b:Q4_K_M` for a catalog file, the file's stem for any other.
pub fn model_id(catalog_id: Option<&str>, file: &str, quant: Option<&str>) -> String {
    match (catalog_id, quant) {
        (Some(id), Some(quant)) => format!("{id}:{quant}"),
        _ => file.trim_end_matches(".gguf").to_lowercase(),
    }
}

pub fn models_dir() -> Result<PathBuf, String> {
    let dir = std::env::var_os("NERU_MODELS_DIR").map(PathBuf::from).map_or_else(|| crate::workspace::data_dir().map(|dir| dir.join("local").join("models")), Ok)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// A repository name as a folder: `unsloth/Qwen3-4B-GGUF` becomes `unsloth--Qwen3-4B-GGUF`.
fn repo_dir(repo: &str) -> String {
    repo.replace('/', "--").chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect()
}

fn valid_name(text: &str) -> bool {
    !text.is_empty() && !text.contains("..") && !text.contains('\\') && !text.starts_with('/') && text.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}

/// A downloaded model.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub file: String,
    pub quant: String,
    pub bytes: u64,
    pub path: String,
    #[serde(default)]
    pub catalog_id: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// The quantization a file name ends with (`…-Q4_K_M.gguf`), uppercase.
pub fn quant_of(file: &str) -> String {
    let stem = file.trim_end_matches(".gguf");
    let upper = stem.to_uppercase();
    for quant in ["Q8_0", "Q6_K", "Q5_K_M", "Q5_K_S", "Q4_K_M", "Q4_K_S", "Q4_0", "IQ4_XS", "IQ4_NL", "Q3_K_M", "Q3_K_S", "Q2_K", "BF16", "F16", "MXFP4"] {
        if upper.ends_with(quant) {
            return quant.into();
        }
    }
    upper.rsplit(['-', '.']).next().unwrap_or_default().to_string()
}

/// Every `.gguf` under the models folder, matched to the catalog where it is one of its files.
pub fn installed() -> Vec<Installed> {
    let Ok(root) = models_dir() else { return Vec::new() };
    let catalog = catalog();
    let mut found = Vec::new();
    for repo in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        if !repo.path().is_dir() {
            continue;
        }
        let repo_name = repo.file_name().to_string_lossy().replacen("--", "/", 1);
        for entry in std::fs::read_dir(repo.path()).into_iter().flatten().flatten() {
            let path = entry.path();
            let file = entry.file_name().to_string_lossy().to_string();
            // Vision projectors ride along with a model; they are not models of their own.
            if !file.ends_with(".gguf") || file.starts_with("mmproj") {
                continue;
            }
            let bytes = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            let known = catalog.iter().find_map(|model| model.files.iter().find(|f| f.file == file && model.repo == repo_name).map(|f| (model, f)));
            found.push(match known {
                Some((model, f)) => Installed { id: model_id(Some(&model.id), &file, Some(&f.quant)), name: model.name.clone(), repo: model.repo.clone(), file: file.clone(), quant: f.quant.clone(), bytes, path: path.to_string_lossy().to_string(), catalog_id: Some(model.id.clone()), roles: model.roles.clone() },
                None => Installed { id: model_id(None, &file, None), name: file.trim_end_matches(".gguf").to_string(), repo: repo_name.clone(), quant: quant_of(&file), file: file.clone(), bytes, path: path.to_string_lossy().to_string(), catalog_id: None, roles: Vec::new() },
            });
        }
    }
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

pub fn find(id: &str) -> Option<Installed> {
    installed().into_iter().find(|model| model.id == id || model.file.trim_end_matches(".gguf").eq_ignore_ascii_case(id))
}

// ---------- downloads ----------

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// `repo/file`, which the window uses to match a progress line to its row.
    pub key: String,
    pub id: String,
    /// starting, downloading, verifying, done, cancelled, failed.
    pub status: String,
    pub completed: u64,
    pub total: u64,
    /// Bytes a second over the last few seconds.
    pub speed: u64,
    pub done: bool,
    pub error: Option<String>,
}

static DOWNLOADS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> = LazyLock::new(Default::default);

pub fn downloading() -> Vec<String> {
    DOWNLOADS.lock().map(|downloads| downloads.keys().cloned().collect()).unwrap_or_default()
}

fn report(app: Option<&AppHandle>, progress: &DownloadProgress) {
    let Some(app) = app else { return };
    let _ = app.emit("builtin://download", progress);
}

/// The Hugging Face address of a file. `HF_ENDPOINT` points at a mirror, as the Hugging Face tools read it.
pub fn file_url(repo: &str, file: &str) -> String {
    let host = std::env::var("HF_ENDPOINT").ok().filter(|host| host.starts_with("http")).unwrap_or_else(|| "https://huggingface.co".into());
    format!("{}/{repo}/resolve/main/{file}?download=true", host.trim_end_matches('/'))
}

/// Downloads `file` from `repo`. A catalog file is checked against its SHA-256; any other GGUF file in
/// a repository works too. A partial download is kept as `.part` and resumed by the next request.
#[tauri::command]
pub async fn builtin_download(repo: String, file: String, app: AppHandle) -> Result<Installed, String> {
    download_model(&repo, &file, Some(&app)).await
}

pub async fn download_model(repo: &str, file: &str, app: Option<&AppHandle>) -> Result<Installed, String> {
    let (repo, file) = (repo.trim().to_string(), file.trim().to_string());
    if !valid_name(&repo) || !repo.contains('/') || !valid_name(&file) || file.contains('/') || !file.ends_with(".gguf") {
        return Err("Name a Hugging Face repository (owner/name) and a .gguf file in it.".into());
    }
    let known = catalog().into_iter().find_map(|model| model.files.iter().find(|f| f.file == file && model.repo == repo).cloned().map(|f| (model, f)));
    let key = format!("{repo}/{file}");
    let cancel = {
        let mut downloads = DOWNLOADS.lock().map_err(|e| e.to_string())?;
        if downloads.contains_key(&key) {
            return Err("This model is already downloading.".into());
        }
        let cancel = Arc::new(tokio::sync::Notify::new());
        downloads.insert(key.clone(), cancel.clone());
        cancel
    };
    let id = model_id(known.as_ref().map(|(model, _)| model.id.as_str()), &file, known.as_ref().map(|(_, f)| f.quant.as_str()));
    let mut progress = DownloadProgress { key: key.clone(), id, status: "starting".into(), ..DownloadProgress::default() };
    report(app, &progress);
    let result = download(&repo, &file, known.as_ref().map(|(_, f)| f), app, &cancel, &mut progress).await;
    if let Ok(mut downloads) = DOWNLOADS.lock() {
        downloads.remove(&key);
    }
    match &result {
        Ok(_) => {
            progress.status = "done".into();
            progress.done = true;
        }
        Err(error) if error == "cancelled" => {
            progress.status = "cancelled".into();
            progress.done = true;
        }
        Err(error) => {
            progress.status = "failed".into();
            progress.error = Some(error.clone());
            progress.done = true;
        }
    }
    report(app, &progress);
    crate::fallback::models_changed("builtin");
    result
}

async fn download(repo: &str, file: &str, known: Option<&GgufFile>, app: Option<&AppHandle>, cancel: &tokio::sync::Notify, progress: &mut DownloadProgress) -> Result<Installed, String> {
    let dir = models_dir()?.join(repo_dir(repo));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(file);
    let part = dir.join(format!("{file}.part"));
    if let Some(known) = known {
        if target.metadata().is_ok_and(|meta| meta.len() == known.bytes) {
            return find_path(&target);
        }
        let have = part.metadata().map(|meta| meta.len()).unwrap_or(0);
        let free = tauri::async_runtime::spawn_blocking({
            let dir = dir.clone();
            move || crate::hardware::free_space(&dir)
        })
        .await
        .ok()
        .flatten();
        let needed = known.bytes.saturating_sub(have) as f64 / 1_073_741_824.0;
        if let Some(free) = free.filter(|free| (*free as f64) < needed + 1.0) {
            return Err(format!("{file} needs {needed:.1} GB more and the drive has {free:.1} GB free. Free some space, or set NERU_MODELS_DIR to a folder on another drive."));
        }
    }
    let mut offset = part.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut request = crate::providers::http().get(file_url(repo, file)).header("User-Agent", "Neru");
    if offset > 0 {
        request = request.header("Range", format!("bytes={offset}-"));
    }
    let mut response = request.send().await.map_err(|e| format!("Hugging Face could not be reached: {e}"))?;
    let status = response.status();
    if status.as_u16() == 416 {
        // The part file is already complete (or longer than the file): start over.
        let _ = std::fs::remove_file(&part);
        offset = 0;
        response = crate::providers::http().get(file_url(repo, file)).header("User-Agent", "Neru").send().await.map_err(|e| format!("Hugging Face could not be reached: {e}"))?;
    } else if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("Hugging Face asks to sign in for this model (a gated repository). Pick another one, or download it in a browser into the models folder.".into());
    } else if status.as_u16() == 404 {
        return Err(format!("{file} is not in {repo} on Hugging Face."));
    } else if !status.is_success() {
        return Err(format!("Hugging Face answered HTTP {status}"));
    }
    // A server that ignores Range sends the whole file again.
    if response.status().as_u16() != 206 {
        offset = 0;
    }
    let total = offset + response.content_length().unwrap_or(0);
    progress.total = known.map_or(total, |known| known.bytes);
    progress.completed = offset;
    progress.status = "downloading".into();
    report(app, progress);
    let mut out = std::fs::OpenOptions::new().create(true).append(offset > 0).write(true).truncate(offset == 0).open(&part).map_err(|e| e.to_string())?;
    let (mut last_report, mut window_start, mut window_bytes) = (Instant::now(), Instant::now(), 0u64);
    let mut buffer = Vec::with_capacity(1 << 20);
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|e| format!("The download stopped: {e}. Download again to continue where it stopped."))?,
            _ = cancel.notified() => {
                out.write_all(&buffer).map_err(|e| e.to_string())?;
                return Err("cancelled".into());
            }
        };
        let Some(chunk) = chunk else { break };
        buffer.extend_from_slice(&chunk);
        progress.completed += chunk.len() as u64;
        window_bytes += chunk.len() as u64;
        // Writes in megabyte batches: fewer system calls on a fast connection.
        if buffer.len() >= 1 << 20 {
            out.write_all(&buffer).map_err(|e| format!("Could not write the model: {e}"))?;
            buffer.clear();
        }
        if last_report.elapsed() >= Duration::from_millis(250) {
            let seconds = window_start.elapsed().as_secs_f64();
            if seconds >= 1.0 {
                progress.speed = (window_bytes as f64 / seconds) as u64;
                if seconds >= 3.0 {
                    window_start = Instant::now();
                    window_bytes = 0;
                }
            }
            report(app, progress);
            last_report = Instant::now();
        }
    }
    out.write_all(&buffer).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    if let Some(known) = known {
        let size = part.metadata().map(|meta| meta.len()).unwrap_or(0);
        if size != known.bytes {
            return Err(format!("The download ended early ({:.2} of {:.2} GB). Download again to continue.", size as f64 / 1e9, known.bytes as f64 / 1e9));
        }
        if let Some(expected) = known.sha256.clone() {
            progress.status = "verifying".into();
            report(app, progress);
            let path = part.clone();
            let actual = tauri::async_runtime::spawn_blocking(move || sha256_file(&path)).await.map_err(|e| e.to_string())??;
            if !actual.eq_ignore_ascii_case(&expected) {
                let _ = std::fs::remove_file(&part);
                return Err("The downloaded file does not match its checksum, so it was deleted. Download it again.".into());
            }
        }
    }
    crate::gguf::read(&part).map_err(|error| {
        let _ = std::fs::remove_file(&part);
        format!("The downloaded file is not a model llama.cpp can read ({error}).")
    })?;
    std::fs::rename(&part, &target).map_err(|e| e.to_string())?;
    find_path(&target)
}

fn find_path(target: &Path) -> Result<Installed, String> {
    let path = target.to_string_lossy().to_string();
    installed().into_iter().find(|model| model.path == path).ok_or_else(|| "The model was saved but could not be listed.".into())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

#[tauri::command]
pub fn builtin_cancel_download(key: String) {
    if let Some(cancel) = DOWNLOADS.lock().ok().and_then(|downloads| downloads.get(key.trim()).cloned()) {
        cancel.notify_one();
    }
}

/// Removes a downloaded model (and a partial download of it), stopping it first if it is loaded.
#[tauri::command]
pub async fn builtin_delete(id: String) -> Result<(), String> {
    let model = find(&id).ok_or("That model is not downloaded.")?;
    crate::llama_server::unload(&model.id).await;
    let path = PathBuf::from(&model.path);
    let root = models_dir()?;
    if !path.starts_with(&root) {
        return Err("Only models in Neru's models folder can be removed here.".into());
    }
    std::fs::remove_file(&path).map_err(|e| format!("Could not remove the model: {e}"))?;
    let _ = std::fs::remove_file(path.with_extension("gguf.part"));
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
    crate::fallback::models_changed("builtin");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_lists_real_files_with_checksums() {
        let catalog = crate::registry::built_in_gguf();
        assert!(catalog.len() >= 5);
        for model in &catalog {
            assert!(model.repo.contains('/') && !model.files.is_empty(), "{model:?}");
            for file in &model.files {
                assert!(file.file.ends_with(".gguf") && file.bytes > 100_000_000, "{file:?}");
                assert_eq!(file.sha256.as_deref().map(str::len), Some(64), "{file:?}");
                assert_eq!(quant_of(&file.file), file.quant);
            }
        }
        assert!(catalog.iter().any(|model| model.roles.iter().any(|role| role == "helper")));
        assert!(catalog.iter().any(|model| model.roles.iter().any(|role| role == "agent")));
    }

    #[test]
    fn ids_and_names() {
        assert_eq!(model_id(Some("gemma-3-1b"), "gemma-3-1b-it-Q4_K_M.gguf", Some("Q4_K_M")), "gemma-3-1b:Q4_K_M");
        assert_eq!(model_id(None, "My-Model-Q5_K_M.gguf", None), "my-model-q5_k_m");
        assert_eq!(repo_dir("unsloth/Qwen3-4B-GGUF"), "unsloth--Qwen3-4B-GGUF");
        assert!(valid_name("unsloth/Qwen3-4B-GGUF") && !valid_name("../etc") && !valid_name("a\\b"));
        assert_eq!(quant_of("Qwen3-4B-Instruct-2507-IQ4_XS.gguf"), "IQ4_XS");
        assert!(file_url("a/b", "c.gguf").ends_with("/a/b/resolve/main/c.gguf?download=true"));
    }

    #[test]
    fn sha256_of_a_file() {
        let dir = std::env::temp_dir().join(format!("neru-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_dir_all(dir);
    }
}
