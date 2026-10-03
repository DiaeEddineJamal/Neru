//! Switches to the next best coding model when the current one is rate limited or out of daily quota.
//! On OpenRouter a per-model limit moves to another free model; the account-wide free daily cap
//! moves to another free provider whose key is saved in Neru.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{AppState, ProviderConfig, providers};

/// Providers with a free plan that Neru may fall back to, best for coding first. Groq comes last:
/// its free per-minute token cap is too small for most coding requests.
pub const FREE_PROVIDERS: &[&str] = &["nvidia", "modelscope", "gemini", "cerebras", "mistral", "openrouter", "huggingface", "groq"];

/// A request on a free plan: a free provider, or a provider's free model. These get the economy
/// treatment (smaller context, paced requests) so their quotas last.
pub fn is_free_plan(config: &ProviderConfig) -> bool {
    FREE_PROVIDERS.contains(&config.provider_id.as_str()) || is_free_model(&config.model)
}

/// What Neru remembers about each model between requests, kept on disk so a model that ran out of
/// its daily quota is not tried again right after a restart.
#[derive(Default, Serialize, Deserialize)]
struct Health {
    /// Models resting after a failure, with when they may be tried again (Unix seconds).
    resting: HashMap<String, u64>,
    /// Failures in a row per model, with when the last one happened (Unix seconds); each one
    /// doubles the next rest. A success clears it, and a day without failing forgets it.
    failures: HashMap<String, (u32, u64)>,
    /// Whole halves of a provider's catalog that cannot be used for now: its free models when the
    /// free tier only works in the provider's own app, its paid ones when the account has no
    /// credits. Keyed "provider\nfree" or "provider\npaid".
    blocked: HashMap<String, u64>,
}

static HEALTH: LazyLock<Mutex<Health>> = LazyLock::new(|| Mutex::new(load_health()));

fn health_file() -> Option<std::path::PathBuf> {
    crate::workspace::data_dir().ok().map(|dir| dir.join("model-health.json"))
}

fn load_health() -> Health {
    let mut health: Health = health_file().and_then(|file| std::fs::read(file).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    let now = unix_now();
    health.resting.retain(|_, until| *until > now);
    health.blocked.retain(|_, until| *until > now);
    health.failures.retain(|_, (_, at)| now.saturating_sub(*at) < FAILURE_MEMORY);
    health
}

/// Writes the health file, merged with what is on disk: the app and the neru CLI share it, so the
/// other process's newer rests are kept. `cleared` models just answered; their rest goes everywhere.
fn save_health(health: &Health, cleared: &[String]) {
    // Tests must not touch the user's data folder.
    if cfg!(test) {
        return;
    }
    let mut merged = load_health();
    for (id, until) in &health.resting {
        merged.resting.entry(id.clone()).and_modify(|other| *other = (*other).max(*until)).or_insert(*until);
    }
    for (id, until) in &health.blocked {
        merged.blocked.entry(id.clone()).and_modify(|other| *other = (*other).max(*until)).or_insert(*until);
    }
    for (id, streak) in &health.failures {
        merged.failures.insert(id.clone(), *streak);
    }
    for id in cleared {
        merged.resting.remove(id);
        merged.failures.remove(id);
    }
    if let (Some(file), Ok(bytes)) = (health_file(), serde_json::to_vec(&merged)) {
        let _ = std::fs::write(file, bytes);
    }
}

/// How long a model's failures count against it.
const FAILURE_MEMORY: u64 = 24 * 3600;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}\n{model}")
}

fn resting(provider_id: &str, model: &str) -> bool {
    HEALTH.lock().ok().and_then(|health| health.resting.get(&key(provider_id, model)).copied()).is_some_and(|until| until > unix_now())
}

fn rest(provider_id: &str, model: &str, for_: Duration) {
    if let Ok(mut health) = HEALTH.lock() {
        health.resting.insert(key(provider_id, model), unix_now() + for_.as_secs().max(1));
        save_health(&health, &[]);
    }
}

/// Failures in a row for this model.
fn failures(provider_id: &str, model: &str) -> u32 {
    let now = unix_now();
    HEALTH.lock().ok().and_then(|health| health.failures.get(&key(provider_id, model)).copied()).filter(|(_, at)| now.saturating_sub(*at) < FAILURE_MEMORY).map_or(0, |(count, _)| count)
}

fn note_failure(provider_id: &str, model: &str) -> u32 {
    let Ok(mut health) = HEALTH.lock() else { return 1 };
    let now = unix_now();
    let entry = health.failures.entry(key(provider_id, model)).or_insert((0, now));
    // A failure long after the last one starts a new streak.
    let count = if now.saturating_sub(entry.1) < FAILURE_MEMORY { (entry.0 + 1).min(10) } else { 1 };
    *entry = (count, now);
    count
}

/// A request to this model succeeded: it is healthy again.
pub fn record_success(config: &ProviderConfig) {
    let id = key(&config.provider_id, &config.model);
    if let Ok(mut health) = HEALTH.lock() {
        let failed = health.failures.remove(&id).is_some();
        let rested = health.resting.remove(&id).is_some();
        if failed || rested {
            save_health(&health, &[id]);
        }
    }
}

/// How long a model rests after failing with `error`: what the provider asked for, else a base by
/// kind of failure, doubled for each failure in a row (a flapping model stays out longer).
pub fn rest_for(error: &str, failures: u32) -> Duration {
    let base = if is_daily(error) || is_client_locked(error) || needs_credits(error) {
        return Duration::from_secs(6 * 3600);
    } else if is_rate_limited(error) {
        crate::subagent::retry_hint(error).map_or(60, |hint| hint.as_secs().max(20))
    } else {
        // Down, unlisted or not answering.
        10 * 60
    };
    Duration::from_secs(base.saturating_mul(1 << failures.saturating_sub(1).min(6)).min(6 * 3600))
}

/// A model of a provider's free tier: `qwen3-coder:free` on OpenRouter, `grok-code-free` on OpenCode Zen.
pub fn is_free_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.ends_with(":free") || lower.ends_with("-free") || lower == "openrouter/free"
}

fn half(provider_id: &str, free: bool) -> String {
    format!("{provider_id}\n{}", if free { "free" } else { "paid" })
}

fn block(provider_id: &str, free: bool, for_: Duration) {
    if let Ok(mut health) = HEALTH.lock() {
        health.blocked.insert(half(provider_id, free), unix_now() + for_.as_secs());
        save_health(&health, &[]);
    }
}

fn blocked(provider_id: &str, model: &str) -> bool {
    HEALTH.lock().ok().and_then(|health| health.blocked.get(&half(provider_id, is_free_model(model))).copied()).is_some_and(|until| until > unix_now())
}

/// The free tier works only inside the provider's own app: OpenCode Zen answers 403 "free tier can
/// only be used from within OpenCode" for its `-free` models, and every one of them fails alike.
pub fn is_client_locked(error: &str) -> bool {
    let lower = error.to_lowercase();
    (lower.contains("http 403") || lower.contains("forbidden"))
        && [
            "only be used from within", "only be used within", "only be used in ", "only be used with", "only available in ",
            "only available within", "only available through", "only works in ", "only works with",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// The account has no credits for this model (HTTP 402, or a balance error). Waiting will not clear it.
pub fn needs_credits(error: &str) -> bool {
    let lower = error.to_lowercase();
    [
        "http 402", "payment required", "insufficient credit", "insufficient balance", "insufficient funds", "insufficient_quota",
        "credit balance is too low", "out of credits", "lack of funds",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// The key works but may not use this model: a 403 that is not about the key itself. Another
/// model, often on another provider, may be allowed.
pub fn is_forbidden(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("http 403") && !crate::models::is_bad_key_text(&lower)
}

/// A 429, an exhausted quota or balance, or a model with no free capacity left.
pub fn is_rate_limited(error: &str) -> bool {
    if needs_credits(error) {
        return true;
    }
    let lower = error.to_lowercase();
    ["http 429", "rate limit", "rate-limit", "ratelimit", "quota", "too many requests", "resource_exhausted", "http 402", "no endpoints found"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// The model is down, overloaded, gone, never started answering, or off limits to this key:
/// another model may work. A rejected key (401, or a 403 about the key) and oversized requests are
/// not included; switching would not help.
pub fn is_unavailable(error: &str) -> bool {
    let lower = error.to_lowercase();
    if lower.contains("http 401") {
        return false;
    }
    if lower.contains("http 403") {
        return is_forbidden(error);
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
    if is_client_locked(error) {
        "is a free model this provider only serves in its own app"
    } else if needs_credits(error) {
        "needs credits this account does not have"
    } else if is_forbidden(error) {
        "is not open to this key"
    } else if is_rate_limited(error) {
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

/// Model lists by provider and URL, kept for a few minutes: a switch looks at every saved key, and
/// fetching each list again would stall the switch for seconds per provider.
static LISTS: LazyLock<Mutex<HashMap<String, (Instant, Vec<String>)>>> = LazyLock::new(Default::default);
const LIST_TTL: Duration = Duration::from_secs(10 * 60);

/// Chat models this key can call, minus the ones a check or a failed request found unavailable.
async fn listed_models(config: &ProviderConfig) -> Vec<String> {
    let id = format!("{}\n{}", config.provider_id, config.base_url);
    if let Some((at, list)) = LISTS.lock().ok().and_then(|lists| lists.get(&id).cloned()) {
        if at.elapsed() < LIST_TTL {
            return list;
        }
    }
    let list = crate::models::usable_ids(config).await;
    // An empty list is likely a network hiccup: do not keep it.
    if !list.is_empty() {
        if let Ok(mut lists) = LISTS.lock() {
            lists.insert(id, (Instant::now(), list.clone()));
        }
    }
    list
}

/// How good a switch to `model` on `candidate` would be: coding strength, less for recent
/// failures, a model whose quota is known to be used up, or a busy sibling of the model that just
/// failed; a little more for staying with the same key.
fn switch_score(current: &ProviderConfig, candidate: &ProviderConfig, model: &str) -> i64 {
    let mut score = coding_score(model) - 8 * i64::from(failures(&candidate.provider_id, model));
    if candidate.provider_id == current.provider_id {
        score += 6;
    }
    for quota in crate::limits::quotas(model) {
        if let (Some(limit), Some(remaining)) = (quota.limit, quota.remaining) {
            if remaining == 0 {
                score -= if quota.label.contains("per day") { 200 } else { 30 };
            } else if limit > 0 && remaining * 10 < limit {
                score -= 10;
            }
        }
    }
    score
}

/// The models a candidate key may switch to: OpenRouter's free ones only, unless the model that
/// failed was itself a paid OpenRouter model, so nothing is billed by surprise.
fn allowed(current: &ProviderConfig, candidate: &ProviderConfig, model: &str) -> bool {
    let paid_ok = candidate.provider_id != "openrouter" || (current.provider_id == "openrouter" && !is_free_model(&current.model) && current.model != "openrouter/free");
    (paid_ok || is_free_model(model) || model == "openrouter/free")
        && !(candidate.provider_id == current.provider_id && model == current.model)
        && !resting(&candidate.provider_id, model)
        && !blocked(&candidate.provider_id, model)
}

/// The next model to use after `current` failed with `error`, or None when nothing else is available.
/// Every key the user saved is considered at once (lists fetched in parallel and cached), and the
/// candidates are ranked by [`switch_score`].
pub async fn next_model(state: &AppState, current: &ProviderConfig, error: &str) -> Option<ProviderConfig> {
    // A model the provider says it does not serve must not come back through the rotation.
    crate::models::note_failure(current, error);
    let account = is_client_locked(error) || needs_credits(error);
    let count = note_failure(&current.provider_id, &current.model);
    let pause = rest_for(error, count);
    rest(&current.provider_id, &current.model, pause);
    if account {
        // A locked tier fails every free model alike; an empty account every paid one.
        block(&current.provider_id, is_free_model(&current.model), pause);
    }
    if current.provider_id == "openrouter" && is_openrouter_free_cap(error) {
        // The cap covers every free model on the account; rest them all until it resets.
        block(&current.provider_id, true, Duration::from_secs(6 * 3600));
    }
    // Models the user named with --fallback-model come first, in their order.
    for wanted in crate::run_options::get().fallback_models {
        if let Some(model) = fallback_choice(current, &wanted).await {
            return Some(ProviderConfig { model, ..current.clone() });
        }
    }
    let mut candidates = vec![current.clone()];
    if let Ok(keys) = state.provider_keys.lock() {
        for (id, key) in keys.iter() {
            let Some((provider_id, base_url)) = id.split_once('\n') else { continue };
            if !FREE_PROVIDERS.contains(&provider_id) || (provider_id == current.provider_id && base_url == current.base_url) {
                continue;
            }
            candidates.push(ProviderConfig { provider_id: provider_id.into(), api_format: "openai-chat".into(), base_url: base_url.into(), api_key: key.clone(), model: String::new() });
        }
    }
    let mut lists = tokio::task::JoinSet::new();
    for (index, candidate) in candidates.iter().cloned().enumerate() {
        lists.spawn(async move { (index, listed_models(&candidate).await) });
    }
    let mut best: Option<(i64, ProviderConfig)> = None;
    while let Some(Ok((index, models))) = lists.join_next().await {
        let candidate = &candidates[index];
        for model in models.into_iter().filter(|model| allowed(current, candidate, model)) {
            let score = switch_score(current, candidate, &model);
            // Ties go to the earlier key: the current one, then the order of the saved keys.
            if best.as_ref().is_none_or(|(top, chosen)| score > *top || (score == *top && candidate_rank(&candidates, chosen) > index)) {
                best = Some((score, ProviderConfig { model, ..candidate.clone() }));
            }
        }
    }
    best.map(|(_, config)| config)
}

fn candidate_rank(candidates: &[ProviderConfig], chosen: &ProviderConfig) -> usize {
    candidates.iter().position(|candidate| candidate.provider_id == chosen.provider_id && candidate.base_url == chosen.base_url).unwrap_or(usize::MAX)
}

/// A second model on the same key for read-only sub-agents on a free plan, so their requests use
/// that model's own per-minute quota instead of the parent's. Close to the parent in strength;
/// None when the key offers nothing suitable.
pub async fn helper_model(config: &ProviderConfig) -> Option<String> {
    if !is_free_plan(config) {
        return None;
    }
    let floor = coding_score(&config.model) - 12;
    let mut models: Vec<String> = listed_models(config)
        .await
        .into_iter()
        .filter(|model| allowed(config, config, model) && coding_score(model) >= floor)
        .collect();
    // Exploring is many short rounds: like Claude Code's Haiku explorers, a quick model beats a
    // slow reasoning one here.
    models.sort_by_key(|model| std::cmp::Reverse(switch_score(config, config, model) + helper_speed(model)));
    models.into_iter().next()
}

/// A bonus for models built to answer fast, a penalty for long-thinking ones.
fn helper_speed(model: &str) -> i64 {
    let name = model.to_lowercase();
    let fast = ["flash", "haiku", "fast", "turbo", "instant", "mini", "lite", "highspeed"].iter().any(|part| name.contains(part));
    let slow = ["reasoning", "thinking", "-r1", "deepseek-r", "qwq", "o1", "o3"].iter().any(|part| name.contains(part));
    if fast { 15 } else if slow { -15 } else { 0 }
}

/// The model a `--fallback-model` value names on the current provider, unless it is the one that
/// just failed or cannot be used now. Part of a name is enough, as with `--model`.
async fn fallback_choice(current: &ProviderConfig, wanted: &str) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    let listed = listed_models(current).await;
    let terms: Vec<String> = wanted.to_lowercase().split_whitespace().map(str::to_string).collect();
    let model = listed
        .iter()
        .find(|model| model.eq_ignore_ascii_case(wanted))
        .or_else(|| listed.iter().find(|model| terms.iter().all(|term| model.to_lowercase().contains(term))))
        .cloned()
        // The list may be unreachable or partial; an id the user typed is taken as is.
        .unwrap_or_else(|| wanted.to_string());
    (model != current.model && !resting(&current.provider_id, &model) && !blocked(&current.provider_id, &model)).then_some(model)
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
    fn locked_free_tiers_and_empty_accounts_switch_models() {
        let locked = "Provider HTTP 403 Forbidden: Free tier can only be used from within OpenCode";
        assert!(is_client_locked(locked) && is_unavailable(locked) && !is_transient(locked));
        assert_eq!(reason(locked), "is a free model this provider only serves in its own app");
        let broke = "Provider HTTP 402 Payment Required: Insufficient balance. Add funds to continue.";
        assert!(needs_credits(broke) && is_rate_limited(broke) && !is_client_locked(broke));
        assert_eq!(reason(broke), "needs credits this account does not have");
        assert!(needs_credits("Provider HTTP 429 Too Many Requests: You exceeded your current quota (insufficient_quota)"));
        // A 403 about the model switches; a 403 about the key does not.
        assert!(is_unavailable("Provider HTTP 403 Forbidden: You do not have access to this model"));
        assert!(!is_unavailable("Provider HTTP 403 Forbidden: Invalid API key provided"));
        assert!(!is_unavailable("Provider HTTP 401 Unauthorized: bad key"));
    }

    #[test]
    fn a_locked_free_tier_blocks_only_that_providers_free_models() {
        assert!(is_free_model("grok-code-free") && is_free_model("qwen/qwen3-coder:free") && !is_free_model("qwen3-coder"));
        block("test-zen", true, Duration::from_secs(60));
        assert!(blocked("test-zen", "big-pickle-free"));
        assert!(!blocked("test-zen", "claude-sonnet-4"));
        assert!(!blocked("test-other", "big-pickle-free"));
    }

    #[test]
    fn rests_follow_the_provider_and_grow_with_repeat_failures() {
        let busy = "Provider HTTP 429 Too Many Requests: rate limited, retry in 37s";
        assert_eq!(rest_for(busy, 1), Duration::from_secs(37));
        assert_eq!(rest_for(busy, 3), Duration::from_secs(148), "doubles for each failure in a row");
        assert_eq!(rest_for("Provider HTTP 429: rate limit", 1), Duration::from_secs(60));
        assert_eq!(rest_for("Provider HTTP 503 Service Unavailable", 1), Duration::from_secs(600));
        assert_eq!(rest_for("Rate limit exceeded: free-models-per-day", 1), Duration::from_secs(6 * 3600));
        assert_eq!(rest_for("Provider HTTP 503", 10), Duration::from_secs(6 * 3600), "capped at six hours");
    }

    #[test]
    fn switches_rank_by_strength_health_and_quota() {
        let current = ProviderConfig { provider_id: "test-switch".into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: String::new(), model: "qwen3-coder-a".into() };
        let other = ProviderConfig { provider_id: "test-other".into(), ..current.clone() };
        // Staying on the same key wins a tie.
        assert!(switch_score(&current, &current, "glm-4.6") > switch_score(&current, &other, "glm-4.6"));
        // A much stronger model elsewhere still wins.
        assert!(switch_score(&current, &other, "qwen3-coder-b") > switch_score(&current, &current, "llama-3.1-8b"));
        // A model that keeps failing drops down the list.
        let before = switch_score(&current, &other, "kimi-k2-test");
        note_failure("test-other", "kimi-k2-test");
        assert!(switch_score(&current, &other, "kimi-k2-test") < before);
        // A spent daily quota rules a model out in practice.
        crate::limits::set_quotas("deepseek-v3-test", vec![crate::limits::Quota { label: "Requests per day".into(), limit: Some(50), remaining: Some(0), resets_in: None }]);
        assert!(switch_score(&current, &current, "deepseek-v3-test") < switch_score(&current, &current, "llama-3.1-8b"));
        // OpenRouter's paid models stay out unless the user was already on one.
        let router = ProviderConfig { provider_id: "openrouter".into(), model: "qwen/qwen3-coder:free".into(), ..current.clone() };
        assert!(!allowed(&router, &router, "anthropic/claude-sonnet-4"));
        assert!(allowed(&router, &router, "moonshotai/kimi-k2:free"));
    }

    #[test]
    fn ranks_coding_models_above_small_general_ones() {
        assert!(coding_score("qwen/qwen3-coder:free") > coding_score("meta-llama/llama-3.3-70b-instruct:free"));
        assert!(coding_score("moonshotai/kimi-k2:free") > coding_score("google/gemma-3-27b-it:free"));
        assert!(coding_score("gemini-3.8-flash") > coding_score("gemini-3.8-flash-lite"));
        assert!(coding_score("meta-llama/llama-3.2-3b-instruct:free") < coding_score("openrouter/free"));
    }
}
