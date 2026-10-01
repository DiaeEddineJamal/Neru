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

/// The model is down, overloaded, gone, or never started answering: another model may work.
/// Key problems (401/403) and oversized requests are not included; switching would not help.
pub fn is_unavailable(error: &str) -> bool {
    let lower = error.to_lowercase();
    if lower.contains("http 401") || lower.contains("http 403") {
        return false;
    }
    [
        "http 500", "http 502", "http 503", "http 504", "http 520", "http 522", "http 524", "http 529",
        "overloaded", "unavailable", "temporarily", "no instances", "not deployed", "degraded",
        "model not found", "model_not_found", "does not exist", "is not a valid model", "unknown model", "no such model", "http 404",
        "did not respond", "did not start answering", "stalled", "timed out", "connection reset", "stream failed",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// A dropped connection or a brief server hiccup: the same request to the same model will
/// likely work a moment later, so Neru retries it before giving up or switching models.
/// A model that never started answering is left to the model switch; retrying it would only
/// repeat the wait.
pub fn is_transient(error: &str) -> bool {
    let lower = error.to_lowercase();
    if is_rate_limited(error)
        || lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("did not start answering")
        || lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("maximum context")
        || lower.contains("prompt is too long")
        || lower.contains("max_tokens")
        || lower.contains("moderation")
        || lower.contains("flagged")
    {
        return false;
    }
    [
        "network connection lost", "connection lost", "connection reset", "connection closed", "connection aborted",
        "connection refused", "broken pipe", "unexpected eof", "incomplete", "error decoding response body",
        "error sending request", "stream failed", "timed out", "socket hang up", "econnreset",
        "http 500", "http 502", "http 503", "http 504", "http 520", "http 521", "http 522", "http 523", "http 524", "http 529",
        "internal server error", "bad gateway", "gateway timeout", "service unavailable", "overloaded",
        "upstream", "provider returned error", "provider stream error",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Why the switch happened, in words for the notice.
pub fn reason(error: &str) -> &'static str {
    let lower = error.to_lowercase();
    if is_rate_limited(error) {
        "hit its usage limit"
    } else if lower.contains("did not start answering") || lower.contains("did not respond") || lower.contains("stalled") || lower.contains("timed out") {
        "is not responding"
    } else if lower.contains("not found") || lower.contains("does not exist") || lower.contains("http 404") || lower.contains("unknown model") {
        "is not available on this provider"
    } else {
        "is having problems"
    }
}

/// The limit resets daily rather than within the minute.
pub fn is_daily(error: &str) -> bool {
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

/// Chat models this key can call, minus the ones a check or a failed request found unavailable.
async fn listed_models(config: &ProviderConfig) -> Vec<String> {
    crate::models::usable_ids(config).await
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
    // A model the provider says it does not serve must not come back through the rotation.
    crate::models::note_failure(current, error);
    let daily = is_daily(error);
    let down = !is_rate_limited(error);
    // A broken or unlisted model stays out longer than a busy one.
    let pause = if daily { 6 * 3600 } else if down { 15 * 60 } else { 90 };
    rest(&current.provider_id, &current.model, Duration::from_secs(pause));
    if current.provider_id != "openrouter" {
        // The same key usually offers other good models; that is the smoothest switch.
        if let Some(model) = best_model(current, false).await.filter(|model| *model != current.model) {
            return Some(ProviderConfig { model, ..current.clone() });
        }
    }
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
    fn spots_models_that_are_down_but_not_key_problems() {
        assert!(is_unavailable("Provider HTTP 503 Service Unavailable: model is overloaded"));
        assert!(is_unavailable("The model did not start answering within 75 seconds"));
        assert!(is_unavailable("Provider HTTP 404: model_not_found"));
        assert!(!is_unavailable("Provider HTTP 401 Unauthorized: bad key"));
        assert_eq!(reason("The model did not start answering within 75 seconds"), "is not responding");
    }

    #[test]
    fn retries_dropped_connections_but_not_limits_or_bad_requests() {
        assert!(is_transient("Provider stream error: Network connection lost."));
        assert!(is_transient("Provider stream failed: error decoding response body"));
        assert!(is_transient("Provider request failed: error sending request for url (https://openrouter.ai/api/v1/chat/completions)"));
        assert!(is_transient("Provider HTTP 502 Bad Gateway: upstream connect error"));
        assert!(is_transient("Provider stream error: context deadline exceeded"));
        assert!(!is_transient("Provider HTTP 429 Too Many Requests: rate limit"));
        assert!(!is_transient("Provider HTTP 401 Unauthorized: bad key"));
        assert!(!is_transient("The model did not start answering within 75 seconds"));
        assert!(!is_transient("Provider stream error: This model's maximum context length is 131072 tokens"));
    }

    #[test]
    fn ranks_coding_models_above_small_general_ones() {
        assert!(coding_score("qwen/qwen3-coder:free") > coding_score("meta-llama/llama-3.3-70b-instruct:free"));
        assert!(coding_score("moonshotai/kimi-k2:free") > coding_score("google/gemma-3-27b-it:free"));
        assert!(coding_score("gemini-3.8-flash") > coding_score("gemini-3.8-flash-lite"));
        assert!(coding_score("meta-llama/llama-3.2-3b-instruct:free") < coding_score("openrouter/free"));
    }
}
