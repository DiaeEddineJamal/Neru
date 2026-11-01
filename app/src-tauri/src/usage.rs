//! Tokens Neru used, on free plans and local models (`free`, which cost nothing) and on paid keys. The window
//! and the phone turn the free total into "what this would have cost" on a paid model.

use std::{
    fs,
    sync::{LazyLock, Mutex},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ProviderConfig, workspace::data_dir};

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Debug)]
pub struct Bucket {
    pub input: u64,
    pub output: u64,
    pub replies: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Totals {
    pub free: Bucket,
    pub paid: Bucket,
    pub since: u64,
}

impl Default for Totals {
    fn default() -> Self {
        Self { free: Bucket::default(), paid: Bucket::default(), since: now() }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

static TOTALS: LazyLock<Mutex<Option<Totals>>> = LazyLock::new(|| Mutex::new(None));

fn path() -> Option<std::path::PathBuf> {
    data_dir().ok().map(|dir| dir.join("usage.json"))
}

fn with<T>(f: impl FnOnce(&mut Totals) -> T) -> T {
    let mut guard = TOTALS.lock().unwrap_or_else(|e| e.into_inner());
    let totals = guard.get_or_insert_with(|| path().and_then(|p| fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
    f(totals)
}

/// Free plans, free models and models on this machine.
pub fn is_free(config: &ProviderConfig) -> bool {
    crate::fallback::is_free_plan(config) || crate::privacy::is_local(config)
}

/// Rough tokens for text a provider did not count.
pub fn estimate(text: &str) -> u64 {
    text.chars().count().div_ceil(4) as u64
}

/// Tokens in a request, roughly, from its text parts.
pub fn estimate_messages(messages: &[Value]) -> u64 {
    fn walk(value: &Value) -> u64 {
        match value {
            Value::String(text) => estimate(text),
            Value::Array(items) => items.iter().map(walk).sum(),
            Value::Object(map) => map.iter().filter(|(key, _)| !matches!(key.as_str(), "image_url" | "data" | "url")).map(|(_, v)| walk(v)).sum(),
            _ => 0,
        }
    }
    messages.iter().map(walk).sum()
}

/// Counts one model reply. Unknown counts (0) are estimated from the text.
pub fn record(config: &ProviderConfig, input: u64, output: u64) {
    let free = is_free(config);
    let saved = with(|totals| {
        let bucket = if free { &mut totals.free } else { &mut totals.paid };
        bucket.input += input;
        bucket.output += output;
        bucket.replies += 1;
        totals.clone()
    });
    if let (Some(path), Ok(text)) = (path(), serde_json::to_string(&saved)) {
        let _ = fs::write(path, text);
    }
}

pub fn totals() -> Totals {
    with(|totals| totals.clone())
}

#[tauri::command]
pub fn usage_totals() -> Totals {
    totals()
}

#[tauri::command]
pub fn reset_usage() -> Totals {
    let fresh = Totals::default();
    with(|totals| *totals = fresh.clone());
    if let (Some(path), Ok(text)) = (path(), serde_json::to_string(&fresh)) {
        let _ = fs::write(path, text);
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimates_skip_images() {
        let messages = vec![json!({"role":"user","content":[{"type":"text","text":"abcdefgh"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAAAAAAAAAAAAAAAAAAAAAA"}}]})];
        // "user" (1) + "text" (1) + "abcdefgh" (2) + "image_url" type tag (3)
        assert_eq!(estimate_messages(&messages), 7);
        assert_eq!(estimate("12345"), 2);
    }
}
