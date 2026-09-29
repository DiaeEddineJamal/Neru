//! Switches to the next best coding model when the current one is rate limited or out of daily quota.
//! On OpenRouter a per-model limit moves to another free model; the account-wide free daily cap
//! moves to another free provider whose key is saved in Neru.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use crate::{AppState, ProviderConfig, providers};

/// Providers with a free plan that Neru may fall back to, best for coding first. Groq comes last:
/// its free per-minute token cap is too small for most coding requests.
const FREE_PROVIDERS: &[&str] = &["nvidia", "modelscope", "gemini", "cerebras", "mistral", "openrouter", "huggingface", "groq"];

/// Models that hit a limit, and when they may be tried again.
static RESTING: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

fn key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}\n{model}")
}

fn resting(provider_id: &str, model: &str) -> bool {
    RESTING
        .lock()
        .ok()
        .and_then(|map| map.get(&key(provider_id, model)).copied())
        .is_some_and(|until| until > Instant::now())
}

fn rest(provider_id: &str, model: &str, for_: Duration) {
    if let Ok(mut map) = RESTING.lock() {
        map.insert(key(provider_id, model), Instant::now() + for_);
    }
}

/// A 429, an exhausted quota, or a model with no free capacity left.
pub fn is_rate_limited(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["http 429", "rate limit", "rate-limit", "ratelimit", "quota", "too many requests", "resource_exhausted", "http 402", "no endpoints found"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// The limit resets daily rather than within the minute.
fn is_daily(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["per day", "per-day", "daily", "free-models-per-day", "rpd", "tpd", "requests per day", "tokens per day"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// OpenRouter's cap on all `:free` models together, as opposed to one model being busy.
fn is_openrouter_free_cap(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("free-models-per-day") || (lower.contains("free") && is_daily(error))
}

/// Rough coding strength by model family, independent of version numbers. Higher is better.
pub fn coding_score(id: &str) -> i64 {
    let name = id.to_lowercase();
    const FAMILIES: &[(&str, i64)] = &[
        ("qwen3-coder", 100),
        ("qwen3.8-coder", 100),
        ("kimi-k", 95),
        ("glm-5", 94),
        ("glm-4", 90),
        ("deepseek-v", 90),
        ("gemini-3", 90),
        ("devstral", 88),
        ("gpt-oss-120b", 86),
        ("minimax-m", 85),
        ("qwen3", 82),
        ("deepseek-r", 80),
        ("gemini-2.5-pro", 80),
        ("codestral", 78),
        ("gemini-2.5-flash", 76),
        ("mistral-large", 75),
        ("llama-4", 70),
        ("gpt-oss", 68),
        ("llama-3.3-70b", 62),
        ("openrouter/free", 60),
        ("gemma", 50),
    ];
    let family = FAMILIES
        .iter()
        .find(|(part, _)| name.contains(part))
        .map_or(30, |(_, score)| *score);
    // Small or cut-down variants code worse; "coder" and "pro" variants better.
    let mut score = family;
    for (part, delta) in [("coder", 6), ("pro", 4), ("-lite", -12), ("mini", -10), ("nano", -15), ("small", -8), ("8b", -20), ("7b", -20), ("3b", -30), ("1b", -40), ("vision", -10), ("guard", -100)] {
        if name.contains(part) {
            score += delta;
        }
    }
    // Prefer bigger windows for medium and large projects.
    let window = providers::context_window(id);
    score + (window / 64_000).min(8) as i64
}

async fn listed_models(config: &ProviderConfig) -> Vec<String> {
    let client = reqwest::Client::new();
    let request = providers::models_request(&client, config).timeout(Duration::from_secs(10));
    let Ok(response) = request.send().await else { return Vec::new() };
    let Ok(body) = providers::read_response(response).await else { return Vec::new() };
    crate::limits::record_windows(&body, &config.provider_id);
    providers::parse_models(&body, &config.provider_id).unwrap_or_default()
}

/// Best free coding model this key can call, other than the resting ones.
async fn best_model(config: &ProviderConfig, free_only: bool) -> Option<String> {
    let mut models: Vec<String> = listed_models(config)
        .await
        .into_iter()
        .filter(|model| !free_only || model.ends_with(":free") || model == "openrouter/free")
        .filter(|model| !resting(&config.provider_id, model))
        .collect();
    models.sort_by_key(|model| std::cmp::Reverse(coding_score(model)));
    models.into_iter().next()
}

/// The next model to use after `current` failed with `error`, or None when nothing else is available.
pub async fn next_model(state: &AppState, current: &ProviderConfig, error: &str) -> Option<ProviderConfig> {
    let daily = is_daily(error);
    rest(&current.provider_id, &current.model, if daily { Duration::from_secs(6 * 3600) } else { Duration::from_secs(90) });
    if current.provider_id == "openrouter" && !is_openrouter_free_cap(error) {
        // One free model is busy; another may not be. Only free models, so nothing is billed.
        if let Some(model) = best_model(current, true).await {
            return Some(ProviderConfig { model, ..current.clone() });
        }
    }
    if current.provider_id == "openrouter" && is_openrouter_free_cap(error) {
        // The cap covers every free model on the account; rest them all until it resets.
        rest(&current.provider_id, "openrouter/free", Duration::from_secs(6 * 3600));
    }
    // Another free provider the user already has a key for.
    let keys: Vec<(String, String, String)> = state
        .provider_keys
        .lock()
        .ok()?
        .iter()
        .filter_map(|(id, key)| {
            let (provider_id, base_url) = id.split_once('\n')?;
            Some((provider_id.to_string(), base_url.to_string(), key.clone()))
        })
        .collect();
    for provider in FREE_PROVIDERS {
        for (provider_id, base_url, api_key) in keys.iter().filter(|(id, _, _)| id == provider) {
            let candidate = ProviderConfig {
                provider_id: provider_id.clone(),
                api_format: "openai-chat".into(),
                base_url: base_url.clone(),
                api_key: api_key.clone(),
                model: String::new(),
            };
            let free_only = provider_id == "openrouter";
            if let Some(model) = best_model(&candidate, free_only).await {
                if provider_id == &current.provider_id && model == current.model {
                    continue;
                }
                return Some(ProviderConfig { model, ..candidate });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_daily_caps_from_busy_models() {
        let cap = "Provider HTTP 429 Too Many Requests: Rate limit exceeded: free-models-per-day. Add 10 credits to unlock 1000 free model requests per day";
        assert!(is_rate_limited(cap) && is_openrouter_free_cap(cap));
        let busy = "Provider HTTP 429 Too Many Requests: qwen/qwen3-coder:free is temporarily rate-limited upstream. Please retry shortly";
        assert!(is_rate_limited(busy) && !is_openrouter_free_cap(busy));
        assert!(!is_rate_limited("Provider HTTP 401 Unauthorized: No auth credentials found"));
    }

    #[test]
    fn ranks_coding_models_above_small_general_ones() {
        assert!(coding_score("qwen/qwen3-coder:free") > coding_score("meta-llama/llama-3.3-70b-instruct:free"));
        assert!(coding_score("moonshotai/kimi-k2:free") > coding_score("google/gemma-3-27b-it:free"));
        assert!(coding_score("gemini-3.8-flash") > coding_score("gemini-3.8-flash-lite"));
        assert!(coding_score("meta-llama/llama-3.2-3b-instruct:free") < coding_score("openrouter/free"));
    }
}
