//! Tasks queued for later, from the window or the phone: each runs when its time comes, one at a time, in a
//! new session of its project, while Neru is open. "Tonight" is just after most free plans reset their daily
//! quota, so big jobs start with full allowances and switch models when one runs out (fallback.rs).

use std::{
    fs,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, activity::now, documents::ImageInput, workspace::data_dir};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTask {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub project_path: String,
    pub at: u64,
    pub created_at: u64,
    pub mode: String,
    /// queued, running, waiting (stopped for an approval), done, failed or canceled.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// How many photos go with it; the images themselves wait in queue-images/<id>.json.
    #[serde(default)]
    pub images: usize,
}

const KEEP_MS: u64 = 7 * 24 * 3600 * 1000;

static QUEUE: LazyLock<Mutex<Option<Vec<QueuedTask>>>> = LazyLock::new(|| Mutex::new(None));

fn path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("queue.json"))
}

fn images_path(id: &str) -> Result<PathBuf, String> {
    let dir = data_dir()?.join("queue-images");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{id}.json")))
}

fn load() -> Vec<QueuedTask> {
    path().ok().and_then(|p| fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Runs `f` on the queue, saves it, and tells the window and the phones when it changed.
fn update<T>(f: impl FnOnce(&mut Vec<QueuedTask>) -> T) -> Result<T, String> {
    let (result, list, changed) = {
        let mut guard = QUEUE.lock().map_err(|e| e.to_string())?;
        let queue = guard.get_or_insert_with(load);
        let before = queue.clone();
        let result = f(queue);
        let cutoff = now().saturating_sub(KEEP_MS);
        queue.retain(|task| matches!(task.status.as_str(), "queued" | "running") || task.at.max(task.created_at) >= cutoff);
        (result, queue.clone(), *queue != before)
    };
    if changed {
        fs::write(path()?, serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if let Some(app) = APP.get() {
            let _ = app.emit("queue://changed", &list);
        }
        crate::remote::push_event("queue", json!(list));
    }
    Ok(result)
}

static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// Every queued task, soonest first, then the ones that ran.
pub fn list() -> Vec<QueuedTask> {
    let mut list = update(|queue| queue.clone()).unwrap_or_default();
    list.sort_by_key(|task| (!matches!(task.status.as_str(), "queued" | "running"), task.at));
    list
}

pub fn add(project_path: String, prompt: String, images: Vec<ImageInput>, at: u64, mode: String) -> Result<QueuedTask, String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() && images.is_empty() {
        return Err("Say what the task is".into());
    }
    if !std::path::Path::new(&project_path).is_dir() {
        return Err(format!("{project_path} is not a folder on this computer"));
    }
    let project_path = std::path::Path::new(&project_path).canonicalize().map(|path| path.to_string_lossy().into_owned()).unwrap_or(project_path);
    if !matches!(mode.as_str(), "manual" | "accept_edits" | "auto" | "plan") {
        return Err("Unknown mode".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    if !images.is_empty() {
        let saved: Vec<serde_json::Value> = images.iter().map(|image| json!({"name": image.name, "dataUrl": image.data_url})).collect();
        fs::write(images_path(&id)?, serde_json::to_string(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    let task = QueuedTask {
        title: crate::sessions::provisional_title(&prompt),
        id,
        prompt,
        project_path,
        // "Now" (0 or a past time) runs at once; keep a real time so the week-long history keeps it.
        at: at.max(now()),
        created_at: now(),
        mode,
        status: "queued".into(),
        session_id: None,
        error: None,
        images: images.len(),
    };
    update(|queue| queue.push(task.clone()))?;
    wake();
    Ok(task)
}

pub fn cancel(id: &str) -> Result<(), String> {
    update(|queue| match queue.iter_mut().find(|task| task.id == id) {
        Some(task) if task.status == "queued" => {
            task.status = "canceled".into();
            Ok(())
        }
        Some(_) => Err("That task already started".to_string()),
        None => Err("No such task".to_string()),
    })??;
    let _ = fs::remove_file(images_path(id)?);
    Ok(())
}

static WAKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);
fn wake() {
    WAKE.notify_one();
}

/// Starts the runner. A task left "running" by a Neru that closed mid-run is marked failed.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let _ = update(|queue| {
        for task in queue.iter_mut().filter(|task| task.status == "running") {
            task.status = "failed".into();
            task.error = Some("Neru closed while this task ran. Open its session to continue.".into());
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let due = update(|queue| queue.iter().filter(|task| task.status == "queued" && task.at <= now()).min_by_key(|task| task.at).cloned()).ok().flatten();
            match due {
                Some(task) => run(&app, task).await,
                None => {
                    let _ = tokio::time::timeout(Duration::from_secs(30), WAKE.notified()).await;
                }
            }
        }
    });
}

async fn run(app: &AppHandle, task: QueuedTask) {
    let set = |status: &str, session: Option<String>, error: Option<String>| {
        let _ = update(|queue| {
            if let Some(entry) = queue.iter_mut().find(|entry| entry.id == task.id) {
                entry.status = status.into();
                if session.is_some() {
                    entry.session_id = session.clone();
                }
                entry.error = error.clone();
            }
        });
    };
    let state = app.state::<AppState>();
    let session = match crate::sessions::create_detached(&state, std::path::Path::new(&task.project_path), &task.title) {
        Ok(id) => id,
        Err(error) => return set("failed", None, Some(error)),
    };
    set("running", Some(session.clone()), None);
    let images: Vec<ImageInput> = images_path(&task.id)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<Vec<serde_json::Value>>(&text).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|image| Some(ImageInput { name: image["name"].as_str()?.into(), data_url: image["dataUrl"].as_str()?.into() }))
        .collect();
    crate::activity::note(&session, crate::activity::Kind::Queued, &task.title);
    let result = crate::remote::run_prompt(app, session.clone(), task.prompt.clone(), images, task.mode.clone()).await;
    let _ = images_path(&task.id).map(fs::remove_file);
    match result {
        Ok(true) => set("waiting", None, None),
        Ok(false) => set("done", None, None),
        Err(error) => set("failed", None, Some(error)),
    }
}

#[tauri::command]
pub fn queue_list() -> Vec<QueuedTask> {
    list()
}

#[tauri::command]
pub fn queue_add(project_path: String, prompt: String, images: Option<Vec<ImageInput>>, at: u64, mode: String) -> Result<QueuedTask, String> {
    add(project_path, prompt, images.unwrap_or_default(), at, mode)
}

#[tauri::command]
pub fn queue_cancel(id: String) -> Result<(), String> {
    cancel(&id)
}

/// The next 00:15 UTC, just after most free plans reset their daily quota (src/remoteProtocol.ts has the same).
pub fn next_quota_reset(now_ms: u64) -> u64 {
    let day = 24 * 3600 * 1000;
    let offset = 15 * 60 * 1000;
    let today = now_ms - now_ms % day + offset;
    if today > now_ms { today } else { today + day }
}

#[tauri::command]
pub fn queue_tonight() -> u64 {
    next_quota_reset(now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonight_is_the_next_quarter_past_midnight_utc() {
        let midnight = 1_790_000_000_000 - 1_790_000_000_000 % 86_400_000;
        assert_eq!(next_quota_reset(midnight), midnight + 900_000);
        assert_eq!(next_quota_reset(midnight + 900_000), midnight + 86_400_000 + 900_000);
        assert_eq!(next_quota_reset(midnight + 3_600_000), midnight + 86_400_000 + 900_000);
    }
}
