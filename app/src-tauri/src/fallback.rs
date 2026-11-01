//! Switches to the next best model when the current one fails: rate limited, out of its daily or
//! monthly quota, out of credit, gone, or down.
//!
//! How a switch is chosen ([`next_model_for`]): the failure is classified ([`Failure`]) and sets a
//! cooldown of its own kind (Retry-After or a growing backoff for rate limits, the provider's reset
//! for spent quotas, a day or more for missing credit, a short circuit breaker on a provider whose
//! servers keep failing). Every saved free key is then listed at once, and its models are filtered by
//! what the request needs (tool calls, images, room for the prompt) and by those cooldowns. The rest
//! are ranked by [`switch_score`]: coding strength, measured health (an exponentially weighted
//! success rate and latency from real requests), checks that showed the model answers, continuity
//! with the model that failed, no ping-pong back to a model that just failed, and how busy each free
//! plan is today. Last-resort providers come only after everything else, and a free plan never
//! switches to a model that bills. On OpenRouter a per-model limit moves to another free model; the
//! account-wide free daily cap moves to another free provider whose key is saved in Neru.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppState, ProviderConfig, providers};

/// Providers whose free plan a new account can use without a card, best for coding first. Each was
/// checked on the provider's official pages on 2026-11-01 (sources next to the presets in
/// app/src/providerCatalog.ts). Groq comes late: its free per-minute token cap is too small for most
/// coding requests. SambaNova's and LLM7's free plans allow too few requests to work on, so they are
/// only last fallbacks ([`LAST_RESORT`]). Not here, because they bill: Z.AI (its GLM Flash models are
/// priced at zero, see [`free_on`], but keys without a balance get "Insufficient balance", code 1113),
/// Hugging Face (its free credit is a few cents a month), Cerebras (a $5 trial with a card) and Ollama
/// Cloud (pay as you go).
pub const FREE_PROVIDERS: &[&str] = &["nvidia", "modelscope", "gemini", "mistral", "openrouter", "cloudflare", "ovhcloud", "kilo", "groq", "sambanova", "llm7"];

/// Free providers used only when nothing else is left, in this order: never picked first, never for chores.
const LAST_RESORT: &[&str] = &["sambanova", "llm7"];

/// Whether a provider serves this model at no cost. Z.AI prices GLM-4.7-Flash, GLM-4.5-Flash and
/// GLM-4.6V-Flash at zero and bills the rest, GLM-5.3-Flash included (docs.z.ai/guides/overview/pricing,
/// checked 2026-11-01). OpenRouter's and Kilo's free models end in ":free" (kilo.ai/docs/gateway).
fn free_on(provider_id: &str, model: &str) -> bool {
    let lower = model.to_lowercase();
    match provider_id {
        "zai" => model.is_empty() || (lower.contains("glm-4") && lower.contains("flash")),
        "openrouter" | "kilo" => model.is_empty() || is_free_model(model),
        _ => true,
    }
}

/// A request on a free plan: a free provider's free model, or a model a provider serves at no cost.
/// These get the economy treatment (smaller context, paced requests) so their quotas last, and never
/// switch to a model that bills.
pub fn is_free_plan(config: &ProviderConfig) -> bool {
    let id = config.provider_id.as_str();
    ((FREE_PROVIDERS.contains(&id) || id == "zai") && free_on(id, &config.model)) || is_free_model(&config.model)
}

// ---------- health ----------

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
    /// Measured health per model, from real requests.
    #[serde(default)]
    stats: HashMap<String, Stat>,
    /// Providers whose servers failed again and again: server errors in a row, and until when the
    /// provider is skipped (Unix seconds).
    #[serde(default)]
    circuits: HashMap<String, (u32, u64)>,
}

/// A model's exponentially weighted success rate (0 to 1) and time to first byte in milliseconds.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
struct Stat {
    ok: f64,
    ms: f64,
    n: u32,
    at: u64,
}

/// Weight of the newest request in a model's health: roughly the last five requests count.
const ALPHA: f64 = 0.3;
/// Where a model's success rate starts: a model nobody measured is assumed mostly fine.
const PRIOR: f64 = 0.9;
/// How long measured health counts.
const STAT_MEMORY: u64 = 7 * 24 * 3600;

impl Stat {
    fn add(&mut self, ok: bool, ms: Option<f64>, now: u64) {
        let start = if self.n == 0 { PRIOR } else { self.ok };
        self.ok = start + ALPHA * (if ok { 1.0 } else { 0.0 } - start);
        if let Some(ms) = ms {
            self.ms = if self.ms == 0.0 { ms } else { self.ms + ALPHA * (ms - self.ms) };
        }
        self.n = self.n.saturating_add(1);
        self.at = now;
    }
}

static HEALTH: LazyLock<Mutex<Health>> = LazyLock::new(|| Mutex::new(load_health()));

fn health_file() -> Option<std::path::PathBuf> {
    crate::workspace::data_dir().ok().map(|dir| dir.join("model-health.json"))
}

fn load_health() -> Health {
    // Tests start from a clean slate, whatever this machine's models did lately.
    if cfg!(test) {
        return Health::default();
    }
    let mut health: Health = health_file().and_then(|file| std::fs::read(file).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    let now = unix_now();
    health.resting.retain(|_, until| *until > now);
    health.blocked.retain(|_, until| *until > now);
    health.failures.retain(|_, (_, at)| now.saturating_sub(*at) < FAILURE_MEMORY);
    health.stats.retain(|_, stat| now.saturating_sub(stat.at) < STAT_MEMORY);
    health.circuits.retain(|_, (_, until)| *until > now);
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
    for (id, stat) in &health.stats {
        if merged.stats.get(id).is_none_or(|other| other.at <= stat.at) {
            merged.stats.insert(id.clone(), *stat);
        }
    }
    for (id, circuit) in &health.circuits {
        merged.circuits.entry(id.clone()).and_modify(|other| *other = (*other).max(*circuit)).or_insert(*circuit);
    }
    for id in cleared {
        merged.resting.remove(id);
        merged.failures.remove(id);
        if let Some((provider_id, _)) = id.split_once('\n') {
            merged.circuits.remove(provider_id);
        }
    }
    if let (Some(file), Ok(bytes)) = (health_file(), serde_json::to_vec(&merged)) {
        let _ = std::fs::write(file, bytes);
    }
}

/// When health that only changed by a success was last written: such writes are spaced out.
static LAST_SAVE: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(Default::default);

/// How long a model's failures count against it.
const FAILURE_MEMORY: u64 = 24 * 3600;
/// A model that failed this recently is not switched back to unless nothing else is left.
const PING_PONG: u64 = 15 * 60;

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

/// Failures in a row for this model, and when the last one happened.
fn streak(provider_id: &str, model: &str) -> Option<(u32, u64)> {
    let now = unix_now();
    HEALTH.lock().ok().and_then(|health| health.failures.get(&key(provider_id, model)).copied()).filter(|(_, at)| now.saturating_sub(*at) < FAILURE_MEMORY)
}

/// Failures in a row for this model.
fn failures(provider_id: &str, model: &str) -> u32 {
    streak(provider_id, model).map_or(0, |(count, _)| count)
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

fn stat(provider_id: &str, model: &str) -> Option<Stat> {
    HEALTH.lock().ok().and_then(|health| health.stats.get(&key(provider_id, model)).copied()).filter(|stat| stat.n > 0)
}

fn add_stat(provider_id: &str, model: &str, ok: bool, ms: Option<f64>) {
    if let Ok(mut health) = HEALTH.lock() {
        health.stats.entry(key(provider_id, model)).or_default().add(ok, ms, unix_now());
    }
}

/// When the last request to each model was sent, and how long its first byte took once it arrived.
static TIMING: LazyLock<Mutex<HashMap<String, (Instant, Option<Duration>)>>> = LazyLock::new(Default::default);

/// A chat request to this model is being sent (providers::chat_request).
pub fn note_sent(config: &ProviderConfig) {
    if let Ok(mut timing) = TIMING.lock() {
        timing.insert(key(&config.provider_id, &config.model), (Instant::now(), None));
    }
}

/// The response's headers arrived (limits::record_headers): the model's time to first byte.
pub fn note_first_byte(config: &ProviderConfig) {
    if let Ok(mut timing) = TIMING.lock() {
        if let Some(entry) = timing.get_mut(&key(&config.provider_id, &config.model)) {
            entry.1.get_or_insert_with(|| entry.0.elapsed());
        }
    }
}

/// A request to this model succeeded: it is healthy again, and its provider's circuit closes.
pub fn record_success(config: &ProviderConfig) {
    let id = key(&config.provider_id, &config.model);
    let ms = TIMING.lock().ok().and_then(|mut timing| timing.remove(&id)).and_then(|(_, first_byte)| first_byte).map(|took| took.as_secs_f64() * 1_000.0);
    if let Ok(mut health) = HEALTH.lock() {
        health.stats.entry(id.clone()).or_default().add(true, ms, unix_now());
        let failed = health.failures.remove(&id).is_some();
        let rested = health.resting.remove(&id).is_some();
        let tripped = health.circuits.remove(&config.provider_id).is_some();
        let due = LAST_SAVE.lock().ok().is_some_and(|last| last.is_none_or(|at| at.elapsed() > Duration::from_secs(30)));
        if failed || rested || tripped || due {
            if let Ok(mut last) = LAST_SAVE.lock() {
                *last = Some(Instant::now());
            }
            save_health(&health, &[id]);
        }
    }
}

/// Server errors in a row on a provider before it is skipped for a while.
const CIRCUIT_TRIP: u32 = 2;

/// Notes a server error on a provider. From the second in a row (within ten minutes) the provider is
/// skipped for a minute, doubling with each further one up to ten minutes.
fn note_server_error(provider_id: &str) {
    if let Ok(mut health) = HEALTH.lock() {
        let now = unix_now();
        let entry = health.circuits.entry(provider_id.to_string()).or_insert((0, 0));
        // `until` doubles as the time of the last error while the circuit is still closed.
        let count = if now.saturating_sub(entry.1) < 600 { entry.0 + 1 } else { 1 };
        let until = if count >= CIRCUIT_TRIP { now + (60u64 << (count - CIRCUIT_TRIP).min(3)).min(600) } else { now };
        *entry = (count, until);
        save_health(&health, &[]);
    }
}

/// The provider's servers failed repeatedly just now: none of its models is picked.
fn circuit_open(provider_id: &str) -> bool {
    HEALTH.lock().ok().and_then(|health| health.circuits.get(provider_id).copied()).is_some_and(|(count, until)| count >= CIRCUIT_TRIP && until > unix_now())
}

// ---------- what went wrong ----------

/// What kind of failure ended a request, as far as picking the next model goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The free tier only works inside the provider's own app.
    Locked,
    /// No credit or balance for this model, or the key's plan does not include it.
    NoCredit,
    /// The provider does not serve this model, or not to this key.
    Gone,
    /// A daily quota is spent.
    DailyQuota,
    /// A weekly or monthly allowance is spent.
    MonthlyQuota,
    /// A per-minute or other short rate limit.
    RateLimit,
    /// A 5xx or an overloaded server.
    ServerError,
    /// The model never started answering, or stalled.
    NotResponding,
    Other,
}

/// Sorts an error into a [`Failure`].
pub fn classify(error: &str) -> Failure {
    let lower = error.to_lowercase();
    let any = |markers: &[&str]| markers.iter().any(|marker| lower.contains(marker));
    if is_client_locked(error) {
        Failure::Locked
    } else if needs_credits(error) || plan_excludes(error) {
        Failure::NoCredit
    } else if is_rate_limited(error) {
        if is_monthly(error) {
            Failure::MonthlyQuota
        } else if is_daily(error) {
            Failure::DailyQuota
        } else {
            Failure::RateLimit
        }
    } else if any(&["did not start answering", "did not respond", "stalled", "timed out"]) {
        Failure::NotResponding
    } else if lower.contains("404 page not found") || is_server_error(&lower) {
        // A gateway's bare 404 is the gateway, not the model.
        Failure::ServerError
    } else if is_forbidden(error)
        || any(&["model not found", "model_not_found", "does not exist", "is not a valid model", "unknown model", "no such model", "http 404", "http 410", "not deployed"])
    {
        Failure::Gone
    } else {
        Failure::Other
    }
}

fn is_server_error(lower: &str) -> bool {
    [
        "http 500", "http 502", "http 503", "http 504", "http 520", "http 521", "http 522", "http 523", "http 524", "http 529",
        "internal server error", "bad gateway", "gateway timeout", "service unavailable", "overloaded", "no instances", "degraded",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// How long a model rests after failing with `error`, by kind of failure: what the provider asked
/// for or a minute for a rate limit, two minutes for a server error, ten for a model that does not
/// answer, six hours for a missing credit or a locked tier. Repeats double it (a flapping model stays
/// out longer). Spent daily and monthly quotas rest until the provider's reset ([`record_failure`]).
pub fn rest_for(error: &str, failures: u32) -> Duration {
    let grow = |base: u64, cap: u64| Duration::from_secs(base.saturating_mul(1 << failures.saturating_sub(1).min(6)).min(cap));
    match classify(error) {
        Failure::Locked | Failure::NoCredit | Failure::Gone | Failure::DailyQuota => Duration::from_secs(6 * 3600),
        Failure::MonthlyQuota => Duration::from_secs(24 * 3600),
        Failure::RateLimit => grow(crate::subagent::retry_hint(error).map_or(60, |hint| hint.as_secs().max(20)), 3600),
        Failure::ServerError => grow(120, 30 * 60),
        Failure::NotResponding | Failure::Other => grow(10 * 60, 6 * 3600),
    }
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

/// The account has no credit for this model: HTTP 402, a balance or billing error, OpenAI's
/// `insufficient_quota`, Z.AI's 1113 and 1309. Waiting will not clear it, and it is not a rate limit.
/// (Gemini's per-minute 429 mentions "billing details" too, so plain "billing" is not enough.)
pub fn needs_credits(error: &str) -> bool {
    let lower = error.to_lowercase();
    [
        "http 402", "payment required", "insufficient credit", "insufficient balance", "insufficient funds", "insufficient_quota",
        "credit balance is too low", "out of credits", "lack of funds", "no credit", "not enough credit", "no remaining credit",
        "no resource package", "please recharge", "package has expired", "requires a paid plan", "paid plan required",
        "requires a paid subscription", "only available on paid plans", "billing is not enabled", "billing not enabled",
        "billing to be enabled", "enable billing", "billing_not_active", "billing account", "billing hard limit", "billing_hard_limit",
        "no payment method", "add a payment method", "(code 1113)", "(code 1309)",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// The key's plan does not include this model (Z.AI's 1311 "does not yet include access to"):
/// another model on the same key may work, waiting will not.
pub fn plan_excludes(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["does not yet include access", "does not include access", "not included in your plan", "not available on your plan", "(code 1311)"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// The key works but may not use this model: a 403 that is not about the key itself. Another
/// model, often on another provider, may be allowed.
pub fn is_forbidden(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("http 403") && !crate::models::is_bad_key_text(&lower)
}

/// A 429, a spent quota, or a model with no free capacity left. Billing and credit errors are not
/// rate limits: [`needs_credits`].
pub fn is_rate_limited(error: &str) -> bool {
    if needs_credits(error) || plan_excludes(error) {
        return false;
    }
    let lower = error.to_lowercase();
    ["http 429", "rate limit", "rate-limit", "ratelimit", "quota", "too many requests", "resource_exhausted", "no endpoints found", "usage limit", "limit exhausted"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// The model is down, overloaded, gone, never started answering, off limits to this key, or the
/// account has no credit for it: another model may work. A rejected key (401, or a 403 about the
/// key) and oversized requests are not included; switching would not help.
pub fn is_unavailable(error: &str) -> bool {
    if needs_credits(error) || plan_excludes(error) {
        return true;
    }
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
        || needs_credits(error)
        || plan_excludes(error)
        || lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("did not start answering")
        || lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("maximum context")
        || lower.contains("prompt is too long")
        || lower.contains("prompt too long")
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

/// Why the switch happened, in words for the notice: "<model> hit its daily limit, so Neru switched
/// to <next>." (agent.rs, subagent.rs).
pub fn reason(error: &str) -> &'static str {
    match classify(error) {
        Failure::Locked => "is a free model this provider only serves in its own app",
        Failure::NoCredit if plan_excludes(error) => "is not included in this key's plan",
        Failure::NoCredit => "has no credit on this key (the provider bills it)",
        Failure::Gone if is_forbidden(error) => "is not open to this key",
        Failure::Gone => "is not available on this provider",
        Failure::DailyQuota => "hit its daily limit",
        Failure::MonthlyQuota => "used up its monthly allowance",
        Failure::RateLimit => "hit its rate limit",
        Failure::ServerError => "is having server problems",
        Failure::NotResponding => "is not responding",
        Failure::Other => "is having problems",
    }
}

/// The limit resets daily or later rather than within the minute.
pub fn is_daily(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["per day", "per-day", "daily", "free-models-per-day", "rpd", "tpd", "requests per day", "tokens per day"]
        .iter()
        .any(|marker| lower.contains(marker))
        || is_monthly(error)
}

/// The limit is a weekly or monthly allowance (Z.AI's 1310 "Weekly/Monthly Limit Exhausted").
pub fn is_monthly(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["per month", "per-month", "monthly", "per week", "weekly", "(code 1310)"].iter().any(|marker| lower.contains(marker))
}

/// OpenRouter's cap on all `:free` models together, as opposed to one model being busy.
fn is_openrouter_free_cap(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("free-models-per-day") || (lower.contains("free") && is_daily(error))
}

/// Records a failed request and sets the cooldown its kind calls for: the model rests (until the
/// provider's reset for a spent quota), a missing credit or locked tier blocks that half of the
/// provider's catalog, and repeated server errors open the provider's circuit. Returns the kind.
fn record_failure(current: &ProviderConfig, error: &str) -> Failure {
    // A model the provider says it does not serve, or bills, must not come back through the rotation.
    crate::models::note_failure(current, error);
    let failure = classify(error);
    let count = note_failure(&current.provider_id, &current.model);
    let pause = match failure {
        Failure::DailyQuota => {
            // A spent daily quota comes back when the provider's day turns over (midnight Pacific for
            // Gemini, UTC for the others), not after a fixed rest.
            crate::limits::note_daily_exhausted(current);
            crate::limits::until_daily_reset(&current.provider_id)
        }
        Failure::MonthlyQuota => {
            crate::limits::note_monthly_exhausted(current);
            crate::limits::until_monthly_reset()
        }
        _ => rest_for(error, count),
    };
    rest(&current.provider_id, &current.model, pause);
    if matches!(failure, Failure::Locked | Failure::NoCredit) && !plan_excludes(error) {
        // A locked tier fails every free model alike; an empty account every paid one.
        block(&current.provider_id, is_free_model(&current.model), pause);
    }
    if current.provider_id == "openrouter" && is_openrouter_free_cap(error) {
        // The cap covers every free model on the account; rest them all until it resets.
        block(&current.provider_id, true, Duration::from_secs(6 * 3600));
    }
    if failure == Failure::ServerError {
        note_server_error(&current.provider_id);
    }
    // Only failures that say something about the model count against its success rate; limits and
    // billing have their own cooldowns.
    if matches!(failure, Failure::ServerError | Failure::NotResponding | Failure::Gone | Failure::Other) {
        add_stat(&current.provider_id, &current.model, false, None);
    }
    failure
}

// ---------- ranking ----------

/// Rough coding strength by model family, independent of version numbers. Higher is better.
pub fn coding_score(id: &str) -> i64 {
    let name = id.to_lowercase();
    // Family scores live in the model registry, which updates without a release (registry.rs).
    let family = crate::registry::with(|registry| registry.coding_families.iter().find(|(part, _)| name.contains(part.as_str())).map_or(30, |(_, score)| *score));
    // Small or cut-down variants code worse; "coder" and "pro" variants better.
    let mut score = family;
    for (part, delta) in [("coder", 6), ("pro", 4), ("-lite", -12), ("mini", -10), ("nano", -15), ("small", -8), ("vision", -10), ("guard", -100)] {
        if name.contains(part) {
            score += delta;
        }
    }
    // Sizes as whole words, so 27b is not taken for 7b, nor 671b for 1b.
    let words: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric() && c != '.').collect();
    for (size, delta) in [("8b", -20), ("7b", -20), ("3b", -30), ("1b", -40)] {
        if words.contains(&size) {
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

fn same_key(a: &ProviderConfig, b: &ProviderConfig) -> bool {
    a.provider_id == b.provider_id && a.base_url == b.base_url
}

/// A model that failed in the last [`PING_PONG`] minutes, even if its rest is over: switching straight
/// back to it is how a switch ping-pongs between two failing models.
fn failed_recently(provider_id: &str, model: &str) -> bool {
    streak(provider_id, model).is_some_and(|(_, at)| unix_now().saturating_sub(at) < PING_PONG)
}

/// How good a switch to `model` on `candidate` would be after `current` failed with `failure`:
/// coding strength; less for recent failures, a poor measured success rate or slow first bytes, a
/// spent or nearly spent quota, or a busy free plan; more for a model a check showed answering, the same family or a similar strength as the current
/// one, a warm prompt cache, and staying on the same key unless the key's whole free plan is the
/// problem.
fn switch_score(current: &ProviderConfig, candidate: &ProviderConfig, model: &str, failure: Failure) -> i64 {
    let mut score = coding_score(model) - 8 * i64::from(failures(&candidate.provider_id, model));
    // On a free plan a rate limit, spent quota or outage usually covers the key's other models too:
    // spread the load to another free provider instead.
    let spread = is_free_plan(current) && matches!(failure, Failure::RateLimit | Failure::DailyQuota | Failure::MonthlyQuota | Failure::ServerError);
    if same_key(candidate, current) {
        score += if spread { -10 } else { 6 };
    }
    // A prompt cache that is still warm makes the next request cheaper and faster.
    if crate::usage::cache_warm(&candidate.provider_id, model) {
        score += 5;
    }
    // Models without native tool calls work on the text protocol, which is less reliable.
    if crate::models::supports_tools(&candidate.provider_id, model) == Some(false) {
        score -= 15;
    }
    let config = ProviderConfig { model: model.to_string(), ..candidate.clone() };
    for quota in crate::limits::quotas(&config) {
        if let (Some(limit), Some(remaining)) = (quota.limit, quota.remaining) {
            if remaining == 0 {
                score -= if quota.label.contains("per day") { 200 } else { 30 };
            } else if limit > 0 && remaining * 10 < limit {
                score -= 10;
            }
        }
    }
    // Measured health: a success rate below about 85% costs up to 34 points, slow first bytes up to 12.
    if let Some(stat) = stat(&candidate.provider_id, model) {
        score += ((stat.ok - 0.85) * 40.0).round() as i64;
        score -= (stat.ms / 2_500.0).min(12.0) as i64;
    }
    if crate::models::verified_ok(&config, model) {
        score += 8;
    }
    // Continuity: the same family, or a model of similar strength, behaves most like the last one.
    if crate::review::family(model) == crate::review::family(&current.model) {
        score += 6;
    }
    score += (6 - (coding_score(model) - coding_score(&current.model)).abs() / 4).max(0);
    // Spread work over the free plans by how much each was used today.
    if is_free_plan(&config) {
        score -= (crate::limits::requests_today(&candidate.provider_id) / 40).min(12) as i64;
    }
    score
}

/// The models a candidate key may switch to: never the one that failed, nor one resting or
/// blocked. Another key's models must be free, and a free plan never moves to a model that bills
/// (OpenRouter's paid models, Z.AI's non-Flash ones), so nothing is billed by surprise.
fn allowed(current: &ProviderConfig, candidate: &ProviderConfig, model: &str) -> bool {
    let free_needed = !same_key(candidate, current) || is_free_plan(current);
    // A model on this computer costs nothing, whatever the plan.
    (!free_needed || crate::privacy::is_local(candidate) || is_free_plan(&ProviderConfig { model: model.to_string(), ..candidate.clone() }))
        && !(candidate.provider_id == current.provider_id && model == current.model)
        && !resting(&candidate.provider_id, model)
        && !blocked(&candidate.provider_id, model)
}

/// What the request needs from the model it switches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
    /// Native tool calls and a 32K window (Code mode): [`crate::models::fits_code_id`].
    pub tools: bool,
    /// The conversation holds images.
    pub vision: bool,
    /// The current prompt, estimated in tokens; 0 when unknown.
    pub prompt_tokens: usize,
}

impl Default for Needs {
    /// What a caller that does not say needs: Code mode's tools.
    fn default() -> Self {
        Needs { tools: true, vision: false, prompt_tokens: 0 }
    }
}

impl Needs {
    /// The needs of a request with these `messages`.
    pub fn of(messages: &[Value], tools: bool, prompt_tokens: usize) -> Self {
        let vision = messages.iter().any(|message| message["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "image_url" || part["type"] == "image")));
        Needs { tools, vision, prompt_tokens }
    }
}

/// Whether `model` can take the request. `strict` also asks for images and room for the prompt;
/// without it, Neru drops the images or trims the context to fit.
fn fits(provider_id: &str, model: &str, needs: Needs, strict: bool) -> bool {
    let room = || {
        let window = providers::context_window(model);
        crate::limits::request_cap(provider_id, model).map_or(window, |cap| cap.min(window))
    };
    (!needs.tools || crate::models::fits_code_id(provider_id, model))
        && (!strict || !needs.vision || crate::models::reads_images(provider_id, model))
        && (!strict || needs.prompt_tokens == 0 || room() >= needs.prompt_tokens + needs.prompt_tokens / 10)
}

/// The best model in `lists` (one list per key in `candidates`, same order): models [`allowed`], on
/// a provider whose circuit is closed, that [`fits`] the request; last-resort providers only when
/// nothing else is left, in [`LAST_RESORT`] order; then models that did not fail minutes ago
/// ([`failed_recently`]); then the highest [`switch_score`]; ties go to the earlier key. When nothing
/// fits the request's images or size, a model that fits the rest is taken.
fn pick(current: &ProviderConfig, candidates: &[ProviderConfig], lists: &[Vec<String>], needs: Needs, failure: Failure) -> Option<ProviderConfig> {
    use std::cmp::Reverse;
    for strict in [true, false] {
        let mut best: Option<((Reverse<usize>, Reverse<bool>, i64, Reverse<usize>), ProviderConfig)> = None;
        for (index, (candidate, models)) in candidates.iter().zip(lists).enumerate() {
            if circuit_open(&candidate.provider_id) {
                continue;
            }
            let tier = LAST_RESORT.iter().position(|id| *id == candidate.provider_id).map_or(0, |at| at + 1);
            for model in models.iter().filter(|model| allowed(current, candidate, model) && fits(&candidate.provider_id, model, needs, strict)) {
                let rank = (Reverse(tier), Reverse(failed_recently(&candidate.provider_id, model)), switch_score(current, candidate, model, failure), Reverse(index));
                if best.as_ref().is_none_or(|(top, _)| rank > *top) {
                    best = Some((rank, ProviderConfig { model: model.clone(), ..candidate.clone() }));
                }
            }
        }
        if best.is_some() {
            return best.map(|(_, config)| config);
        }
    }
    None
}

/// The next model to use after `current` failed with `error`, or None when nothing else is available,
/// for a request with Code mode's needs. See [`next_model_for`].
pub async fn next_model(state: &AppState, current: &ProviderConfig, error: &str, rule: &str) -> Option<ProviderConfig> {
    next_model_for(state, current, error, rule, Needs::default()).await
}

/// The next model to use after `current` failed with `error`, for a request that `needs` tools,
/// images or room for its prompt. Every key the user saved is considered at once (lists fetched in
/// parallel and cached), and the choice is made by [`pick`]. Only models `rule` allows (privacy.rs)
/// are considered.
pub async fn next_model_for(state: &AppState, current: &ProviderConfig, error: &str, rule: &str, needs: Needs) -> Option<ProviderConfig> {
    let failure = record_failure(current, error);
    // Models the user named with --fallback-model come first, in their order.
    for wanted in crate::run_options::get().fallback_models {
        if let Some(model) = fallback_choice(current, &wanted).await.filter(|_| crate::privacy::allows(rule, current)) {
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
    candidates.retain(|candidate| crate::privacy::allows(rule, candidate));
    let mut lists = vec![Vec::new(); candidates.len()];
    let mut fetching = tokio::task::JoinSet::new();
    for (index, candidate) in candidates.iter().cloned().enumerate() {
        fetching.spawn(async move { (index, listed_models(&candidate).await) });
    }
    while let Some(Ok((index, models))) = fetching.join_next().await {
        lists[index] = models;
    }
    pick(current, &candidates, &lists, needs, failure)
}

/// A model of another family than `family` to review an edit: the strongest free one on the saved keys
/// (and on the author's own key), quick ones preferred, never a paid OpenRouter model. None when every model
/// on the keys is of the same family.
pub async fn reviewer_model(state: &AppState, author: &ProviderConfig, family: &str, rule: &str) -> Option<ProviderConfig> {
    let mut candidates = vec![author.clone()];
    if let Ok(keys) = state.provider_keys.lock() {
        for (id, key) in keys.iter() {
            let Some((provider_id, base_url)) = id.split_once('\n') else { continue };
            if !FREE_PROVIDERS.contains(&provider_id) || (provider_id == author.provider_id && base_url == author.base_url) {
                continue;
            }
            candidates.push(ProviderConfig { provider_id: provider_id.into(), api_format: "openai-chat".into(), base_url: base_url.into(), api_key: key.clone(), model: String::new() });
        }
    }
    candidates.retain(|candidate| crate::privacy::allows(rule, candidate));
    let mut lists = tokio::task::JoinSet::new();
    for (index, candidate) in candidates.iter().cloned().enumerate() {
        lists.spawn(async move { (index, listed_models(&candidate).await) });
    }
    let mut best: Option<(i64, ProviderConfig)> = None;
    while let Some(Ok((index, models))) = lists.join_next().await {
        let candidate = &candidates[index];
        // On a paid key only its free models review, so a second opinion never adds to the bill.
        let paid_key = !is_free_plan(&ProviderConfig { model: String::new(), ..candidate.clone() }) && !crate::privacy::is_local(candidate);
        for model in models {
            if !allowed(author, candidate, &model) || crate::review::family(&model) == family || (paid_key && !is_free_model(&model)) {
                continue;
            }
            let score = coding_score(&model) + helper_speed(&model) / 3;
            if best.as_ref().is_none_or(|(top, _)| score > *top) {
                best = Some((score, ProviderConfig { model, ..candidate.clone() }));
            }
        }
    }
    best.map(|(_, config)| config)
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
        .filter(|model| allowed(config, config, model) && crate::models::fits_code_id(&config.provider_id, model) && coding_score(model) >= floor)
        .collect();
    // Exploring is many short rounds: like Claude Code's Haiku explorers, a quick model beats a
    // slow reasoning one here.
    models.sort_by_key(|model| std::cmp::Reverse(switch_score(config, config, model, Failure::Other) + helper_speed(model)));
    models.into_iter().next()
}

/// The small models chosen lately, by main key, model and privacy rule.
static SMALL: LazyLock<Mutex<HashMap<String, (Instant, Option<ProviderConfig>)>>> = LazyLock::new(Default::default);

// ---------- starter model ----------

/// OVHcloud AI Endpoints answers without a key (about 2 requests a minute per model), so a first message
/// works before any key is added.
const STARTER_BASE_URL: &str = "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1";
/// Where a fresh install points before a model is connected: a local gateway few people run.
const DEFAULT_LOCAL_URL: &str = "http://localhost:3001/v1";

/// Whether `config` cannot answer because no model was really connected: a provider that needs a key
/// has none, or the install's default local gateway, untouched, has nothing listening.
pub async fn needs_starter(config: &ProviderConfig) -> bool {
    if config.provider_id == "local" && config.base_url == DEFAULT_LOCAL_URL && config.model == "auto" {
        let connect = tokio::net::TcpStream::connect("127.0.0.1:3001");
        return !matches!(tokio::time::timeout(Duration::from_millis(400), connect).await, Ok(Ok(_)));
    }
    config.api_key.is_empty() && !crate::privacy::is_local(config) && !providers::keyless(&config.provider_id)
}

/// The best keyless model to answer with until a key is added, or None when the privacy `rule` keeps
/// work off cloud models or the list cannot be read.
pub async fn starter_model(rule: &str) -> Option<ProviderConfig> {
    let base_url = crate::registry::get().starter_base_url.unwrap_or_else(|| STARTER_BASE_URL.into());
    let mut config = ProviderConfig { provider_id: "ovhcloud".into(), api_format: "openai-chat".into(), base_url, model: String::new(), api_key: String::new() };
    if crate::privacy::blocked_reason(rule, &config).is_some() {
        return None;
    }
    config.model = best_starter(&listed_models(&config).await)?;
    Some(config)
}

fn best_starter(models: &[String]) -> Option<String> {
    models.iter().filter(|model| coding_score(model) > 0).max_by_key(|model| coding_score(model)).cloned()
}

/// The fastest capable model across the saved free keys and the main key, for chores: session titles,
/// conversation summaries, commit messages, repairing tool calls, planning research queries. Groq,
/// Cerebras, Flash-class and local models come first; providers the privacy `rule` blocks are skipped.
/// None when nothing suitable is saved: callers use the main model then. Chosen lists are cached for ten
/// minutes.
pub async fn small_model(state: &AppState, main: &ProviderConfig, rule: &str) -> Option<ProviderConfig> {
    let id = format!("{}\n{}\n{}\n{rule}", main.provider_id, main.base_url, main.model);
    if let Some((at, chosen)) = SMALL.lock().ok().and_then(|small| small.get(&id).cloned()) {
        if at.elapsed() < LIST_TTL && chosen.as_ref().is_none_or(|chosen| !resting(&chosen.provider_id, &chosen.model)) {
            return chosen;
        }
    }
    let mut candidates = vec![main.clone()];
    if let Ok(keys) = state.provider_keys.lock() {
        for (key_id, key) in keys.iter() {
            let Some((provider_id, base_url)) = key_id.split_once('\n') else { continue };
            if !FREE_PROVIDERS.contains(&provider_id) || LAST_RESORT.contains(&provider_id) || (provider_id == main.provider_id && base_url == main.base_url) {
                continue;
            }
            candidates.push(ProviderConfig { provider_id: provider_id.into(), api_format: "openai-chat".into(), base_url: base_url.into(), api_key: key.clone(), model: String::new() });
        }
    }
    // Ollama running beside a cloud model: its small models take the chores, which then cost no
    // cloud request and stay on this computer.
    if main.provider_id != "ollama" {
        if let Some(base_url) = running_ollama().await {
            candidates.push(ProviderConfig { provider_id: "ollama".into(), api_format: "openai-chat".into(), base_url, api_key: String::new(), model: String::new() });
        }
    }
    candidates.retain(|candidate| crate::privacy::allows(rule, candidate));
    let mut lists = tokio::task::JoinSet::new();
    for (index, candidate) in candidates.iter().cloned().enumerate() {
        lists.spawn(async move { (index, listed_models(&candidate).await) });
    }
    let mut best: Option<(i64, ProviderConfig)> = None;
    while let Some(Ok((index, models))) = lists.join_next().await {
        let candidate = &candidates[index];
        for model in models {
            if !allowed(main, candidate, &model) {
                continue;
            }
            let Some(score) = small_score(&candidate.provider_id, &model) else { continue };
            if best.as_ref().is_none_or(|(top, _)| score > *top) {
                best = Some((score, ProviderConfig { model, api_format: if index == 0 { candidate.api_format.clone() } else { "openai-chat".into() }, ..candidate.clone() }));
            }
        }
    }
    let chosen = best.map(|(_, config)| config);
    if let Ok(mut small) = SMALL.lock() {
        small.insert(id, (Instant::now(), chosen.clone()));
    }
    chosen
}

/// How well a model suits quick chores: fast hosts and fast variants first, enough skill to follow a
/// format. None for models too weak, too slow (long thinkers) or not meant for chat.
fn small_score(provider_id: &str, model: &str) -> Option<i64> {
    if matches!(provider_id, "ollama" | "local") {
        return local_helper_score(model);
    }
    let strength = coding_score(model);
    let speed = helper_speed(model);
    let name = model.to_lowercase();
    if strength < 45 || speed < 0 || ["guard", "embed", "vision", "audio", "tts", "whisper"].iter().any(|part| name.contains(part)) {
        return None;
    }
    let host = match provider_id {
        "groq" | "cerebras" => 30,
        "ollama" | "local" => 20,
        "gemini" | "mistral" => 10,
        _ => 0,
    };
    Some(host + speed * 2 + strength / 3)
}

/// Billions of parameters a model tag names (`gemma3:4b`, `qwen2.5-coder:1.5b`), if it names them.
fn parameters_b(model: &str) -> Option<f32> {
    let name = model.to_lowercase();
    name.split(|c: char| !c.is_ascii_alphanumeric() && c != '.').filter_map(|word| word.strip_suffix('b')?.parse::<f32>().ok()).last()
}

/// Small models on this computer come before any cloud model for chores: they cost no request and
/// keep the text here. Larger ones (slow on most machines), thinking ones (slow for a one-line answer)
/// and ones whose size the tag does not give are left to the cloud lane.
fn local_helper_score(model: &str) -> Option<i64> {
    let name = model.to_lowercase();
    if ["embed", "guard", "vision", "whisper", "-r1", "qwq", "thinking"].iter().any(|part| name.contains(part)) {
        return None;
    }
    let size = parameters_b(model).filter(|size| *size <= 9.0)?;
    // Three or four billion parameters write a good commit message; one billion often does not.
    let quality = (size.min(4.0) * 5.0) as i64;
    // Plain Qwen3 thinks before every answer unless told not to.
    let thinks = name.starts_with("qwen3:") && !name.contains("instruct");
    Some(100 + quality - if thinks { 15 } else { 0 })
}

/// Ollama's address as Ollama itself reads it (OLLAMA_HOST, else 127.0.0.1:11434), when something
/// answers there on this computer. Checked at most every half minute.
async fn running_ollama() -> Option<String> {
    static SEEN: LazyLock<Mutex<Option<(Instant, Option<String>)>>> = LazyLock::new(Default::default);
    if let Some((at, found)) = SEEN.lock().ok().and_then(|seen| seen.clone()) {
        if at.elapsed() < Duration::from_secs(30) {
            return found;
        }
    }
    let host = std::env::var("OLLAMA_HOST").ok().map(|host| host.trim().to_string()).filter(|host| !host.is_empty()).unwrap_or_else(|| "127.0.0.1:11434".into());
    let host = if host.contains("://") { host } else { format!("http://{host}") };
    // 0.0.0.0 is how Ollama is told to listen everywhere; on this computer it answers on 127.0.0.1.
    let base_url = format!("{}/v1", host.trim_end_matches('/').replace("0.0.0.0", "127.0.0.1"));
    let found = match reqwest::Url::parse(&base_url) {
        // Only this computer: an Ollama elsewhere is not private, whatever its provider id says.
        Ok(url) if providers::is_local(&url) => {
            let address = format!("{}:{}", url.host_str().unwrap_or("127.0.0.1"), url.port_or_known_default().unwrap_or(11434));
            matches!(tokio::time::timeout(Duration::from_millis(300), tokio::net::TcpStream::connect(address)).await, Ok(Ok(_))).then_some(base_url)
        }
        _ => None,
    };
    if let Ok(mut seen) = SEEN.lock() {
        *seen = Some((Instant::now(), found.clone()));
    }
    found
}

/// A bonus for models built to answer fast, a penalty for long-thinking ones.
fn helper_speed(model: &str) -> i64 {
    // "gemini" contains "mini", which says nothing about speed.
    let name = model.to_lowercase().replace("gemini", "gem");
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
    fn small_local_models_take_chores_before_any_cloud_model() {
        assert_eq!(parameters_b("gemma3:4b"), Some(4.0));
        assert_eq!(parameters_b("qwen2.5-coder:1.5b"), Some(1.5));
        assert_eq!(parameters_b("llama3.2"), None);
        let gemma = small_score("ollama", "gemma3:4b").unwrap();
        let cloud = ["llama-3.1-8b-instant", "gemini-3-flash", "openai/gpt-oss-20b"].iter().filter_map(|model| small_score("groq", model)).max().unwrap_or(0);
        assert!(gemma > cloud, "local {gemma} should beat the best cloud helper {cloud}");
        assert!(small_score("ollama", "gemma3:4b") > small_score("ollama", "gemma3:1b"), "a little bigger writes better");
        assert!(small_score("ollama", "qwen3:4b") < small_score("ollama", "gemma3:4b"), "thinking models are slower for one line");
        assert_eq!(small_score("ollama", "qwen3-coder:30b"), None, "too slow for chores on most machines");
        assert_eq!(small_score("ollama", "deepseek-r1:7b"), None);
        assert_eq!(small_score("ollama", "nomic-embed-text"), None);
        assert_eq!(small_score("ollama", "llama3.2"), None, "no size in the tag: left to the cloud lane");
        let main = ProviderConfig { provider_id: "gemini".into(), api_format: "openai-chat".into(), base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(), model: "gemini-3-flash".into(), api_key: "k".into() };
        let ollama = ProviderConfig { provider_id: "ollama".into(), api_format: "openai-chat".into(), base_url: "http://127.0.0.1:11434/v1".into(), model: String::new(), api_key: String::new() };
        assert!(allowed(&main, &ollama, "gemma3:4b"), "a local model is free whatever the main plan");
    }

    #[test]
    fn the_starter_is_the_best_coder_on_the_keyless_list() {
        let listed: Vec<String> = ["gpt-oss-20b", "Meta-Llama-3_3-70B-Instruct", "Mistral-Nemo-Instruct-2407", "Qwen3Guard-Gen-8B", "Qwen3.8-27B"].map(String::from).to_vec();
        assert_eq!(best_starter(&listed).as_deref(), Some("Qwen3.8-27B"));
        // Sizes are whole words: a 27B model is not a 7B one, nor a 671B one a 1B one.
        assert!(coding_score("Qwen3.8-27B") > coding_score("Qwen3-8B"));
        assert!(coding_score("deepseek-v3-671b") >= coding_score("deepseek-v3"));
        assert!(coding_score("qwen2.5-coder:7b") < coding_score("qwen2.5-coder:32b"));
        assert_eq!(best_starter(&["Qwen3Guard-Gen-8B".to_string()]), None, "never a guard model");
        assert_eq!(best_starter(&[]), None);
    }

    #[test]
    fn only_an_unconnected_model_needs_the_starter() {
        let config = |provider_id: &str, base_url: &str, key: &str| ProviderConfig { provider_id: provider_id.into(), api_format: "openai-chat".into(), base_url: base_url.into(), model: "m".into(), api_key: key.into() };
        let needs = |config: ProviderConfig| tauri::async_runtime::block_on(async move { needs_starter(&config).await });
        assert!(needs(config("gemini", "https://generativelanguage.googleapis.com/v1beta/openai", "")));
        assert!(!needs(config("gemini", "https://generativelanguage.googleapis.com/v1beta/openai", "AIza-test")));
        assert!(!needs(config("ovhcloud", STARTER_BASE_URL, "")), "keyless already");
        assert!(!needs(config("ollama", "http://localhost:11434/v1", "")), "a local server the user chose");
    }

    fn config(provider_id: &str, model: &str) -> ProviderConfig {
        ProviderConfig { provider_id: provider_id.into(), api_format: "openai-chat".into(), base_url: format!("https://{provider_id}.test/v1"), api_key: String::new(), model: model.into() }
    }

    fn lists(items: &[&[&str]]) -> Vec<Vec<String>> {
        items.iter().map(|models| models.iter().map(|model| model.to_string()).collect()).collect()
    }

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
        assert!(!is_transient("Provider HTTP 402 Payment Required: upstream says insufficient credits"));
    }

    #[test]
    fn locked_free_tiers_and_empty_accounts_switch_models() {
        let locked = "Provider HTTP 403 Forbidden: Free tier can only be used from within OpenCode";
        assert!(is_client_locked(locked) && is_unavailable(locked) && !is_transient(locked));
        assert_eq!(reason(locked), "is a free model this provider only serves in its own app");
        let broke = "Provider HTTP 402 Payment Required: Insufficient balance. Add funds to continue.";
        assert!(needs_credits(broke) && !is_rate_limited(broke) && is_unavailable(broke) && !is_client_locked(broke));
        assert_eq!(reason(broke), "has no credit on this key (the provider bills it)");
        assert!(needs_credits("Provider HTTP 429 Too Many Requests: You exceeded your current quota (insufficient_quota)"));
        // A 403 about the model switches; a 403 about the key does not.
        assert!(is_unavailable("Provider HTTP 403 Forbidden: You do not have access to this model"));
        assert!(!is_unavailable("Provider HTTP 403 Forbidden: Invalid API key provided"));
        assert!(!is_unavailable("Provider HTTP 401 Unauthorized: bad key"));
    }

    #[test]
    fn billing_errors_are_their_own_kind_not_rate_limits() {
        for error in [
            "Provider HTTP 402 Payment Required: request failed",
            "Provider HTTP 429 Too Many Requests: Insufficient balance or no resource package. Please recharge. (code 1113)",
            "Provider HTTP 429 Too Many Requests: You exceeded your current quota, please check your plan and billing details. (code insufficient_quota)",
            "Provider HTTP 403 Forbidden: This model requires a paid plan",
            "Provider HTTP 400 Bad Request: You have no credits left",
            "Provider HTTP 403 Forbidden: This API method requires billing to be enabled",
        ] {
            assert_eq!(classify(error), Failure::NoCredit, "{error}");
            assert!(!is_rate_limited(error) && is_unavailable(error) && !is_transient(error), "{error}");
        }
        // Gemini's per-minute limit mentions billing too, but it is a rate limit.
        let gemini = "Provider HTTP 429 Too Many Requests: You exceeded your current quota, please check your plan and billing details. limit: 10";
        assert_eq!(classify(gemini), Failure::RateLimit);
        let plan = "Provider HTTP 429 Too Many Requests: Your current subscription plan does not yet include access to GLM-5 (code 1311)";
        assert_eq!(classify(plan), Failure::NoCredit);
        assert_eq!(reason(plan), "is not included in this key's plan");
        assert_eq!(classify("Provider HTTP 429 Too Many Requests: Weekly/Monthly Limit Exhausted (code 1310)"), Failure::MonthlyQuota);
        assert_eq!(reason("Provider HTTP 429 Too Many Requests: Rate limit exceeded: free-models-per-day"), "hit its daily limit");
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
    fn rests_follow_the_kind_of_failure_and_grow_with_repeats() {
        let busy = "Provider HTTP 429 Too Many Requests: rate limited, retry in 37s";
        assert_eq!(rest_for(busy, 1), Duration::from_secs(37));
        assert_eq!(rest_for(busy, 3), Duration::from_secs(148), "doubles for each failure in a row");
        assert_eq!(rest_for("Provider HTTP 429: rate limit", 1), Duration::from_secs(60));
        assert_eq!(rest_for("Provider HTTP 429: rate limit", 10), Duration::from_secs(3600), "a rate limit rests an hour at most");
        assert_eq!(rest_for("Provider HTTP 503 Service Unavailable", 1), Duration::from_secs(120));
        assert_eq!(rest_for("Provider HTTP 503", 10), Duration::from_secs(30 * 60), "server errors rest half an hour at most");
        assert_eq!(rest_for("The model did not start answering within 75 seconds", 1), Duration::from_secs(600));
        assert_eq!(rest_for("Rate limit exceeded: free-models-per-day", 1), Duration::from_secs(6 * 3600));
        assert_eq!(rest_for("Provider HTTP 402 Payment Required", 1), Duration::from_secs(6 * 3600));
    }

    #[test]
    fn spent_quotas_rest_until_the_providers_reset() {
        let gemini = config("gemini", "gemini-test-daily");
        assert_eq!(record_failure(&gemini, "Provider HTTP 429 Too Many Requests: Quota exceeded: requests per day"), Failure::DailyQuota);
        let until = HEALTH.lock().unwrap().resting[&key("gemini", "gemini-test-daily")];
        assert!(until.abs_diff(crate::limits::next_day("gemini", unix_now())) <= 2, "rests until midnight Pacific");
        let zai = config("zai", "glm-test-monthly");
        assert_eq!(record_failure(&zai, "Provider HTTP 429 Too Many Requests: Weekly/Monthly Limit Exhausted (code 1310)"), Failure::MonthlyQuota);
        let until = HEALTH.lock().unwrap().resting[&key("zai", "glm-test-monthly")];
        assert!(until.abs_diff(crate::limits::next_month(unix_now())) <= 2, "rests until the month turns over");
    }

    #[test]
    fn switches_rank_by_strength_health_and_quota() {
        let current = ProviderConfig { provider_id: "test-switch".into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: String::new(), model: "qwen3-coder-a".into() };
        let other = ProviderConfig { provider_id: "test-other".into(), ..current.clone() };
        let other_ = Failure::Other;
        // Staying on the same key wins a tie.
        assert!(switch_score(&current, &current, "glm-4.6", other_) > switch_score(&current, &other, "glm-4.6", other_));
        // A much stronger model elsewhere still wins.
        assert!(switch_score(&current, &other, "qwen3-coder-b", other_) > switch_score(&current, &current, "llama-3.1-8b", other_));
        // A model that keeps failing drops down the list.
        let before = switch_score(&current, &other, "kimi-k2-test", other_);
        note_failure("test-other", "kimi-k2-test");
        assert!(switch_score(&current, &other, "kimi-k2-test", other_) < before);
        // A spent daily quota rules a model out in practice.
        crate::limits::set_quotas(&ProviderConfig { model: "deepseek-v3-test".into(), ..current.clone() }, vec![crate::limits::Quota { label: "Requests per day".into(), limit: Some(50), remaining: Some(0), resets_in: None }]);
        assert!(switch_score(&current, &current, "deepseek-v3-test", other_) < switch_score(&current, &current, "llama-3.1-8b", other_));
        // OpenRouter's paid models stay out unless the user was already on one.
        let router = ProviderConfig { provider_id: "openrouter".into(), model: "qwen/qwen3-coder:free".into(), ..current.clone() };
        assert!(!allowed(&router, &router, "anthropic/claude-sonnet-4"));
        assert!(allowed(&router, &router, "moonshotai/kimi-k2:free"));
        let paid_router = ProviderConfig { model: "anthropic/claude-opus-4".into(), ..router.clone() };
        assert!(allowed(&paid_router, &paid_router, "anthropic/claude-sonnet-4"), "a paid OpenRouter model may move to another paid one");
    }

    #[test]
    fn measured_health_and_checks_move_models_up_or_down() {
        let current = config("test-health", "qwen3-coder-h");
        let healthy = switch_score(&current, &current, "glm-4.6-healthy", Failure::Other);
        for _ in 0..3 {
            add_stat("test-health", "glm-4.6-flaky", false, None);
        }
        assert!(switch_score(&current, &current, "glm-4.6-flaky", Failure::Other) < healthy - 15, "a low success rate costs");
        add_stat("test-health", "glm-4.6-slow", true, Some(30_000.0));
        assert!(switch_score(&current, &current, "glm-4.6-slow", Failure::Other) < healthy, "a slow first byte costs");
        // A successful request measures time to first byte.
        let fast = config("test-health", "glm-4.6-timed");
        note_sent(&fast);
        note_first_byte(&fast);
        record_success(&fast);
        let measured = stat("test-health", "glm-4.6-timed").unwrap();
        assert!(measured.n == 1 && measured.ok > PRIOR && measured.ms < 1_000.0);
    }

    #[test]
    fn free_providers_bill_only_what_they_bill() {
        let zai = ProviderConfig { provider_id: "zai".into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: String::new(), model: "glm-4.7-flash".into() };
        assert!(!FREE_PROVIDERS.contains(&"zai"), "Z.AI keys need a balance: not a free provider");
        assert!(is_free_plan(&zai), "but its Flash models cost nothing");
        assert!(!is_free_plan(&ProviderConfig { model: "glm-4.7".into(), ..zai.clone() }), "Z.ai bills its other models");
        assert!(!is_free_plan(&ProviderConfig { model: "glm-5.3-flash".into(), ..zai.clone() }), "GLM-5.3-Flash is billed");
        assert!(!allowed(&zai, &zai, "glm-4.7"));
        assert!(allowed(&zai, &zai, "glm-4.5-flash"));
        let hf = ProviderConfig { provider_id: "huggingface".into(), model: "openai/gpt-oss-120b".into(), ..zai.clone() };
        assert!(!is_free_plan(&hf), "Hugging Face's credit is a few cents");
        assert!(!is_free_plan(&config("openrouter", "anthropic/claude-sonnet-4")), "OpenRouter's paid models bill");
    }

    #[test]
    fn picks_skip_rate_limited_and_spent_models() {
        let current = config("nvidia", "qwen3-coder-pick-a");
        let candidates = [current.clone(), config("mistral", "")];
        let models = lists(&[&["qwen3-coder-pick-a", "qwen3-coder-pick-b", "llama-3.3-70b-pick"], &["devstral-pick"]]);
        // The failed model is never picked again; a rate-limited sibling rests too.
        record_failure(&ProviderConfig { model: "qwen3-coder-pick-b".into(), ..current.clone() }, "Provider HTTP 429 Too Many Requests: rate limit, retry in 30s");
        let next = pick(&current, &candidates, &models, Needs::default(), Failure::RateLimit).unwrap();
        assert_eq!(next.model, "devstral-pick", "rate limited on a free plan: spread to another provider");
        // A spent daily quota rests until the reset.
        record_failure(&config("mistral", "devstral-pick"), "Provider HTTP 429 Too Many Requests: requests per day exceeded");
        let next = pick(&current, &candidates, &models, Needs::default(), Failure::RateLimit).unwrap();
        assert_eq!(next.model, "llama-3.3-70b-pick");
    }

    #[test]
    fn picks_never_choose_unavailable_or_billed_models() {
        let current = config("nvidia", "qwen3-coder-bill-a");
        let candidates = [current.clone(), config("openrouter", ""), config("kilo", ""), config("groq", "")];
        let models = lists(&[&["qwen3-coder-bill-a"], &["anthropic/claude-sonnet-bill"], &["qwen/qwen3-coder-bill:free", "moonshotai/kimi-k2-bill:free", "anthropic/claude-bill-kilo"], &["llama-3.3-70b-bill"]]);
        // No credit: the model rests and that half of the provider's catalog is blocked, the sibling too.
        record_failure(&config("kilo", "qwen/qwen3-coder-bill:free"), "Provider HTTP 402 Payment Required: Insufficient credits");
        let next = pick(&current, &candidates, &models, Needs::default(), Failure::Other).unwrap();
        assert_eq!(next.provider_id, "groq", "neither OpenRouter's billed model nor a Kilo model without credit");
        // A model gone from the provider stays out.
        record_failure(&config("groq", "llama-3.3-70b-bill"), "Provider HTTP 404 Not Found: model_not_found");
        assert!(pick(&current, &candidates, &models, Needs::default(), Failure::Other).is_none());
    }

    #[test]
    fn picks_match_the_requests_needs() {
        let current = config("test-needs", "qwen3-coder-needs");
        let candidates = [current.clone()];
        // Code mode: a model whose tool test failed is out.
        let list = crate::models::parse_models(&serde_json::json!({"data":[{"id":"glm-4.6-needs-notools","supported_parameters":["temperature"]},{"id":"llama-3.3-70b-needs","supported_parameters":["tools"]}]}), "test-needs").unwrap();
        crate::models::remember("test-needs", &list);
        let models = lists(&[&["glm-4.6-needs-notools", "llama-3.3-70b-needs"]]);
        if !crate::models::fits_code_id("test-needs", "glm-4.6-needs-notools") {
            assert_eq!(pick(&current, &candidates, &models, Needs::default(), Failure::Other).unwrap().model, "llama-3.3-70b-needs");
        }
        // Images: a model that reads them wins over a stronger one that does not.
        let models = lists(&[&["qwen3-coder-needs-b", "qwen2.5-vl-72b-needs"]]);
        let vision = Needs { tools: false, vision: true, prompt_tokens: 0 };
        assert!(crate::models::reads_images("test-needs", "qwen2.5-vl-72b-needs"));
        assert_eq!(pick(&current, &candidates, &models, vision, Failure::Other).unwrap().model, "qwen2.5-vl-72b-needs");
        // Room for the prompt: a 1M-token window beats a stronger 256K one for a 300K prompt.
        let models = lists(&[&["qwen3-coder-needs-c", "gemini-2.5-flash-needs"]]);
        let big = Needs { tools: false, vision: false, prompt_tokens: 300_000 };
        assert_eq!(pick(&current, &candidates, &models, big, Failure::Other).unwrap().model, "gemini-2.5-flash-needs");
        // When nothing fits, the best of the rest is taken (Neru trims the context).
        let models = lists(&[&["qwen3-coder-needs-d"]]);
        assert_eq!(pick(&current, &candidates, &models, big, Failure::Other).unwrap().model, "qwen3-coder-needs-d");
        let messages = vec![serde_json::json!({"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]})];
        assert!(Needs::of(&messages, true, 10).vision && !Needs::of(&[], true, 10).vision);
    }

    #[test]
    fn picks_do_not_ping_pong_back_to_a_model_that_just_failed() {
        let second = config("test-pingpong", "glm-4.6-pong");
        let candidates = [second.clone()];
        let models = lists(&[&["qwen3-coder-ping", "glm-4.6-pong", "llama-3.3-70b-pong"]]);
        // The first model failed and its short rest is already over, then the second fails too.
        note_failure("test-pingpong", "qwen3-coder-ping");
        let next = pick(&second, &candidates, &models, Needs::default(), Failure::Other).unwrap();
        assert_eq!(next.model, "llama-3.3-70b-pong", "not straight back to the model that failed minutes ago");
    }

    #[test]
    fn last_resort_providers_come_last_in_their_order() {
        let current = config("nvidia", "qwen3-coder-last-a");
        let candidates = [current.clone(), config("llm7", ""), config("sambanova", ""), config("groq", "")];
        let models = lists(&[&["qwen3-coder-last-a"], &["qwen3-coder-last-llm7"], &["qwen3-coder-last-samba"], &["llama-3.1-8b-last"]]);
        // A weak model on a regular free provider beats strong ones on last-resort providers.
        assert_eq!(pick(&current, &candidates, &models, Needs::default(), Failure::Other).unwrap().provider_id, "groq");
        // Then SambaNova before LLM7, whatever the order of the saved keys.
        rest("groq", "llama-3.1-8b-last", Duration::from_secs(60));
        assert_eq!(pick(&current, &candidates, &models, Needs::default(), Failure::Other).unwrap().provider_id, "sambanova");
        rest("sambanova", "qwen3-coder-last-samba", Duration::from_secs(60));
        assert_eq!(pick(&current, &candidates, &models, Needs::default(), Failure::Other).unwrap().provider_id, "llm7");
    }

    #[test]
    fn repeated_server_errors_open_a_brief_circuit() {
        let current = config("test-circuit", "qwen3-coder-circuit-a");
        let candidates = [current.clone()];
        let models = lists(&[&["qwen3-coder-circuit-a", "glm-4.6-circuit-b"]]);
        assert_eq!(record_failure(&current, "Provider HTTP 503 Service Unavailable"), Failure::ServerError);
        assert!(!circuit_open("test-circuit"), "one server error rests only that model");
        assert_eq!(pick(&current, &candidates, &models, Needs::default(), Failure::ServerError).unwrap().model, "glm-4.6-circuit-b");
        record_failure(&ProviderConfig { model: "glm-4.6-circuit-b".into(), ..current.clone() }, "Provider HTTP 502 Bad Gateway");
        assert!(circuit_open("test-circuit"), "a second one skips the provider for a while");
        // A success closes it again.
        record_success(&ProviderConfig { model: "glm-4.6-circuit-c".into(), ..current.clone() });
        assert!(!circuit_open("test-circuit"));
    }

    #[test]
    fn small_lane_prefers_fast_capable_models() {
        let groq = small_score("groq", "llama-3.3-70b-versatile").unwrap();
        let big = small_score("nvidia", "qwen/qwen3-coder-480b-a35b-instruct").unwrap();
        assert!(groq > big, "a fast host beats a stronger slow model for chores");
        assert!(small_score("gemini", "gemini-3.8-flash").unwrap() > small_score("gemini", "gemini-3.8-pro").unwrap());
        assert!(small_score("groq", "deepseek-r1-distill-llama-70b").is_none(), "long thinkers are too slow");
        assert!(small_score("groq", "llama-guard-4-12b").is_none());
        assert!(small_score("groq", "llama-3.2-1b-preview").is_none(), "too weak to follow a format");
    }

    #[test]
    fn warm_caches_and_native_tools_win_ties() {
        let current = ProviderConfig { provider_id: "test-warm".into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: String::new(), model: "qwen3-coder-a".into() };
        let before = switch_score(&current, &current, "glm-4.6-warm", Failure::Other);
        crate::usage::record_cached(&ProviderConfig { model: "glm-4.6-warm".into(), ..current.clone() }, 2_000, 10, 1_500);
        assert_eq!(switch_score(&current, &current, "glm-4.6-warm", Failure::Other), before + 5);
        let list = crate::models::parse_models(&serde_json::json!({"data":[{"id":"glm-4.6-text","supported_parameters":["temperature"]}]}), "test-warm").unwrap();
        crate::models::remember("test-warm", &list);
        assert!(switch_score(&current, &current, "glm-4.6-text", Failure::Other) < switch_score(&current, &current, "glm-4.6-other", Failure::Other));
    }

    #[test]
    fn ranks_coding_models_above_small_general_ones() {
        assert!(coding_score("qwen/qwen3-coder:free") > coding_score("meta-llama/llama-3.3-70b-instruct:free"));
        assert!(coding_score("moonshotai/kimi-k2:free") > coding_score("google/gemma-3-27b-it:free"));
        assert!(coding_score("gemini-3.8-flash") > coding_score("gemini-3.8-flash-lite"));
        assert!(coding_score("meta-llama/llama-3.2-3b-instruct:free") < coding_score("openrouter/free"));
    }
}
