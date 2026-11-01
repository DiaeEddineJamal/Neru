//! What each session did and when: changes applied, commands run (and which failed), runs that stopped on an
//! error. Kept for a week in `activity.json`, so "While you were away" can say what happened overnight.

use std::{
    collections::HashMap,
    fs,
    sync::{LazyLock, Mutex},
};

use serde::{Deserialize, Serialize};

use crate::workspace::data_dir;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Edit,
    Command,
    FailedCommand,
    /// A run that ended with an error.
    Failed,
    /// A run that finished.
    Finished,
    /// A run started from the task queue.
    Queued,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Event {
    pub at: u64,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

const KEEP_MS: u64 = 7 * 24 * 3600 * 1000;

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

type Log = HashMap<String, Vec<Event>>;
static LOG: LazyLock<Mutex<Option<Log>>> = LazyLock::new(|| Mutex::new(None));

fn path() -> Option<std::path::PathBuf> {
    data_dir().ok().map(|dir| dir.join("activity.json"))
}

fn with<T>(f: impl FnOnce(&mut Log) -> T) -> T {
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let log = guard.get_or_insert_with(|| path().and_then(|p| fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
    f(log)
}

/// Notes one thing a session did. `session` "manual" (the desktop's own task buttons) is not a session.
pub fn note(session: &str, kind: Kind, detail: &str) {
    if session.is_empty() || session == "manual" {
        return;
    }
    let cutoff = now().saturating_sub(KEEP_MS);
    let saved = with(|log| {
        log.entry(session.to_string()).or_default().push(Event { at: now(), kind, detail: detail.chars().take(300).collect() });
        log.retain(|_, events| {
            events.retain(|event| event.at >= cutoff);
            !events.is_empty()
        });
        serde_json::to_string(log).ok()
    });
    if let (Some(path), Some(text)) = (path(), saved) {
        let _ = fs::write(path, text);
    }
}

/// Each session's events at or after `since`.
pub fn since(since: u64) -> HashMap<String, Vec<Event>> {
    with(|log| {
        log.iter()
            .filter_map(|(id, events)| {
                let recent: Vec<Event> = events.iter().filter(|event| event.at >= since).cloned().collect();
                (!recent.is_empty()).then(|| (id.clone(), recent))
            })
            .collect()
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestItem {
    pub session_id: String,
    pub title: String,
    pub project: String,
    pub project_path: String,
    /// done, waiting, failed or running.
    pub status: String,
    pub summary: String,
    pub edits: usize,
    pub commands: usize,
    pub failed_commands: usize,
    pub queued: bool,
    pub updated_at: u64,
}

#[derive(Serialize)]
pub struct Digest {
    pub since: u64,
    pub items: Vec<DigestItem>,
    pub usage: crate::usage::Totals,
}

/// What each session did since `since`, sessions that need you first, then the latest.
pub fn digest(state: &crate::AppState, since_ms: u64) -> Result<Digest, String> {
    let events = since(since_ms);
    let mut items = Vec::new();
    for (summary, waiting) in crate::sessions::all_with_status(state)? {
        let Some(list) = events.get(&summary.id) else { continue };
        let count = |kind: Kind| list.iter().filter(|event| event.kind == kind).count();
        let ended = list.iter().rev().find(|event| matches!(event.kind, Kind::Finished | Kind::Failed));
        let status = if waiting {
            "waiting"
        } else if summary.running {
            "running"
        } else if ended.is_some_and(|event| event.kind == Kind::Failed) {
            "failed"
        } else {
            "done"
        };
        let reply = crate::sessions::runtime(state, &summary.id)
            .ok()
            .and_then(|shared| shared.lock().ok().and_then(|runtime| runtime.transcript.iter().rev().find(|entry| entry.role == "assistant" && !entry.content.trim().is_empty()).map(|entry| entry.content.clone())));
        let failure = ended.filter(|event| event.kind == Kind::Failed).map(|event| event.detail.clone());
        let text = failure.or(reply).unwrap_or_default();
        let mut summary_text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(280).collect();
        if text.chars().count() > 280 {
            summary_text.push('…');
        }
        items.push(DigestItem {
            project: std::path::Path::new(&summary.project_path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "Chat".into()),
            session_id: summary.id.clone(),
            project_path: summary.project_path.clone(),
            title: summary.title.clone(),
            status: status.into(),
            summary: summary_text,
            edits: count(Kind::Edit),
            commands: count(Kind::Command) + count(Kind::FailedCommand),
            failed_commands: count(Kind::FailedCommand),
            queued: count(Kind::Queued) > 0,
            updated_at: list.last().map(|event| event.at).unwrap_or(summary.updated_at),
        });
    }
    items.sort_by_key(|item| (item.status != "waiting", std::cmp::Reverse(item.updated_at)));
    Ok(Digest { since: since_ms, items, usage: crate::usage::totals() })
}

#[tauri::command]
pub fn activity_digest(since: u64, state: tauri::State<'_, crate::AppState>) -> Result<Digest, String> {
    digest(&state, since)
}
