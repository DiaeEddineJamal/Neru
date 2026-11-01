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
    /// Input tokens the provider served from its prompt cache (part of `input`).
    #[serde(default)]
    pub cached: u64,
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
    // Tests must not count into the user's totals.
    if cfg!(test) {
        return None;
    }
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
    text_tokens(text).ceil() as u64
}

/// Tokens in a text by what it is made of: CJK takes about a token per character, code (dense with
/// symbols and short identifiers) about one per 3.2 characters, prose about one per 4.
pub fn text_tokens(text: &str) -> f64 {
    let (mut cjk, mut symbols, mut other) = (0usize, 0usize, 0usize);
    for c in text.chars() {
        if matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0xff00..=0xffef) {
            cjk += 1;
        } else if c.is_ascii_punctuation() {
            symbols += 1;
        } else {
            other += 1;
        }
    }
    let plain = symbols + other;
    // Prose has a symbol every 20 to 30 characters; code one every 4 to 8.
    let per_token = if plain > 0 && symbols * 10 > plain { 3.2 } else { 4.0 };
    cjk as f64 * 1.05 + plain as f64 / per_token
}

/// How far each model's tokenizer is off `text_tokens`, learned from the prompt tokens providers report
/// (an exponential moving average per model), so budgets track the real tokenizer without shipping one.
static CALIBRATION: LazyLock<Mutex<std::collections::HashMap<String, f64>>> = LazyLock::new(Default::default);

/// Learns from one request: Neru estimated `estimated` tokens, the provider counted `reported`.
pub fn calibrate(model: &str, estimated: f64, reported: u64) {
    // Tiny requests are mostly template overhead and say little about the tokenizer.
    if estimated < 500.0 || reported < 500 {
        return;
    }
    let ratio = (reported as f64 / estimated).clamp(0.5, 2.0);
    if let Ok(mut factors) = CALIBRATION.lock() {
        let factor = factors.entry(model.to_string()).or_insert(ratio);
        *factor = *factor * 0.7 + ratio * 0.3;
    }
}

/// The learned factor for a model, 1.0 until it reported usage.
pub fn token_factor(model: &str) -> f64 {
    CALIBRATION.lock().ok().and_then(|factors| factors.get(model).copied()).unwrap_or(1.0)
}

/// Models whose prompt cache was hit recently, keyed provider and model: staying on one of them is cheaper
/// and faster than switching to a cold one.
static WARM: LazyLock<Mutex<std::collections::HashMap<String, std::time::Instant>>> = LazyLock::new(Default::default);

/// Whether this model served cached prompt tokens in the last five minutes (providers keep caches about that long).
pub fn cache_warm(provider_id: &str, model: &str) -> bool {
    WARM.lock().ok().and_then(|warm| warm.get(&format!("{provider_id}\n{model}")).copied()).is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(300))
}

/// Tokens in a request, roughly, from its text parts.
pub fn estimate_messages(messages: &[Value]) -> u64 {
    fn walk(value: &Value) -> u64 {
        match value {
            Value::String(text) => estimate(text),
            Value::Array(items) => items.iter().map(walk).sum(),
            Value::Object(map) => map.iter().filter(|(key, _)| !matches!(key.as_str(), "image_url" | "data" | "url") && !key.starts_with('_')).map(|(_, v)| walk(v)).sum(),
            _ => 0,
        }
    }
    messages.iter().map(walk).sum()
}

/// Counts one model reply, `cached` of whose input tokens came from the provider's prompt cache.
pub fn record_cached(config: &ProviderConfig, input: u64, output: u64, cached: u64) {
    let free = is_free(config);
    if cached > 0 {
        if let Ok(mut warm) = WARM.lock() {
            warm.insert(format!("{}\n{}", config.provider_id, config.model), std::time::Instant::now());
        }
    }
    crate::limits::note_request(config, input + output);
    let saved = with(|totals| {
        let bucket = if free { &mut totals.free } else { &mut totals.paid };
        bucket.input += input;
        bucket.output += output;
        bucket.replies += 1;
        bucket.cached += cached.min(input);
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
        // "user" (1) + "text" (1) + "abcdefgh" (2) + "image_url" type tag (3, mostly symbols)
        assert_eq!(estimate_messages(&messages), 7);
        assert_eq!(estimate("12345"), 2);
    }

    #[test]
    fn estimates_follow_what_the_text_is_made_of() {
        let prose = "The quick brown fox jumps over the lazy dog and keeps running through the field.";
        let code = "fn main(){let x=vec![1,2,3];println!(\"{:?}\",x.iter().map(|v|v*2).collect::<Vec<_>>());}";
        assert!(text_tokens(code) / code.len() as f64 > text_tokens(prose) / prose.len() as f64, "code is denser");
        assert!(text_tokens("你好世界你好世界") >= 8.0, "about a token per CJK character");
    }

    #[test]
    fn calibration_moves_toward_reported_usage() {
        assert_eq!(token_factor("test-calibrate"), 1.0);
        calibrate("test-calibrate", 1_000.0, 1_300);
        assert!((token_factor("test-calibrate") - 1.3).abs() < 1e-9, "the first report sets the factor");
        calibrate("test-calibrate", 1_000.0, 1_000);
        let factor = token_factor("test-calibrate");
        assert!(factor < 1.3 && factor > 1.0, "later reports move it gradually");
        calibrate("test-calibrate", 10.0, 900);
        assert_eq!(token_factor("test-calibrate"), factor, "tiny requests are ignored");
    }

    #[test]
    fn cached_tokens_warm_the_model_and_are_counted() {
        let config = ProviderConfig { provider_id: "test-cache".into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: String::new(), model: "warm-model".into() };
        assert!(!cache_warm("test-cache", "warm-model"));
        let before = totals().paid.cached + totals().free.cached;
        record_cached(&config, 1_000, 10, 800);
        assert!(cache_warm("test-cache", "warm-model"));
        assert_eq!(totals().paid.cached + totals().free.cached, before + 800);
    }
}
