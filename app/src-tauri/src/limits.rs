//! What providers report about a model's size and a key's remaining quota. Feeds the context meter
//! and sizes requests, since a free plan's per-minute token cap can be far below the model's window.

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ProviderConfig, providers, workspace::data_dir};

/// One rate limit on the key, as the provider last reported it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    pub label: String,
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub resets_in: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    /// Context window per model, from the provider's model list.
    windows: HashMap<String, usize>,
    /// Largest request per model: a tokens-per-minute cap or a limit named in a "too large" error.
    request_caps: HashMap<String, usize>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(load()));
/// The quotas each (provider, model, key) last reported, and when (see `ledger_key`).
static QUOTAS: LazyLock<Mutex<HashMap<String, (std::time::Instant, u64, Vec<Quota>)>>> = LazyLock::new(Default::default);
/// Requests and tokens Neru sent per (provider, model, key) in the current minute, hour and day, for
/// providers that send no quota headers (Gemini) and for the meter. The day's counts are kept on disk,
/// so a restart does not make a spent free plan look fresh.
static COUNTS: LazyLock<Mutex<HashMap<String, Counters>>> = LazyLock::new(|| Mutex::new(load_days()));
/// Keys that must not be sent to before a time (Unix seconds): a Retry-After, or a spent daily quota.
static BLOCKED: LazyLock<Mutex<HashMap<String, (u64, String)>>> = LazyLock::new(Default::default);

/// Unit tests keep the store in memory: they must neither read what this machine's providers
/// reported nor overwrite it.
fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    data_dir().ok().map(|dir| dir.join("model-limits.json"))
}

fn load() -> Store {
    let mut store: Store = path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    // Caps saved before they were kept per provider would limit the model everywhere.
    store.request_caps.retain(|key, cap| key.contains('|') && *cap >= MIN_CAP);
    store
}

fn save(store: &Store) {
    if let (Some(path), Ok(text)) = (path(), serde_json::to_string_pretty(store)) {
        let _ = fs::write(path, text);
    }
}

fn counts_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    data_dir().ok().map(|dir| dir.join("request-counts.json"))
}

/// The day windows saved by `save_days`: (day start, requests, tokens) per ledger key.
fn load_days() -> HashMap<String, Counters> {
    let saved: HashMap<String, (u64, u64, u64)> = counts_path().and_then(|path| fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
    saved.into_iter().map(|(key, (start, requests, tokens))| (key, Counters { day: Window { start, requests, tokens }, ..Counters::default() })).collect()
}

fn save_days(counts: &HashMap<String, Counters>) {
    let Some(path) = counts_path() else { return };
    let now = unix_now();
    // Days that are over are dropped; two days covers every provider's clock.
    let days: HashMap<&String, (u64, u64, u64)> = counts.iter().filter(|(_, counters)| now.saturating_sub(counters.day.start) < 2 * 86_400).map(|(key, counters)| (key, (counters.day.start, counters.day.requests, counters.day.tokens))).collect();
    if let Ok(text) = serde_json::to_string(&days) {
        let _ = fs::write(path, text);
    }
}

/// The context window the provider reported for this model, if any.
pub fn window(model: &str) -> Option<usize> {
    STORE.lock().ok()?.windows.get(model).copied()
}

/// Caps belong to a key's plan on one provider: Groq's 8K tokens a minute for gpt-oss says nothing
/// about the same model on another provider.
fn cap_key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}|{model}")
}

/// Smallest cap worth keeping. Below this a provider sent a placeholder (some report a token limit
/// of 0), and sizing requests by it would leave no room for even one message.
const MIN_CAP: usize = 2_000;

pub fn request_cap(provider_id: &str, model: &str) -> Option<usize> {
    STORE.lock().ok()?.request_caps.get(&cap_key(provider_id, model)).copied().filter(|cap| *cap >= MIN_CAP)
}

pub fn set_request_cap(provider_id: &str, model: &str, cap: usize) {
    if cap < MIN_CAP {
        return;
    }
    if let Ok(mut store) = STORE.lock() {
        let key = cap_key(provider_id, model);
        if store.request_caps.get(&key) != Some(&cap) {
            store.request_caps.insert(key, cap);
            save(&store);
        }
    }
}

/// Quotas belong to one key on one provider: the same model on another key, or on another provider,
/// has limits of its own. The key itself is never stored, only a fingerprint.
pub fn ledger_key(config: &ProviderConfig) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.api_key.hash(&mut hasher);
    format!("{}|{}|{:016x}", config.provider_id, config.model, hasher.finish())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// What the key reported (headers, OpenRouter's /key), or else what Neru counted itself.
pub fn quotas(config: &ProviderConfig) -> Vec<Quota> {
    let key = ledger_key(config);
    let list = QUOTAS.lock().ok().and_then(|quotas| quotas.get(&key).map(|(_, _, list)| list.clone())).unwrap_or_default();
    if !list.is_empty() {
        return list;
    }
    COUNTS.lock().ok().and_then(|counts| counts.get(&key).cloned()).map(|counts| counts.as_quotas(&config.provider_id, unix_now())).unwrap_or_default()
}

pub(crate) fn set_quotas(config: &ProviderConfig, list: Vec<Quota>) {
    if let Ok(mut quotas) = QUOTAS.lock() {
        quotas.insert(ledger_key(config), (std::time::Instant::now(), unix_now(), list));
    }
}

// ---------- reset clocks ----------

/// Days since 1970-01-01 for a date in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year a day (since 1970-01-01) falls in.
fn civil_year(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month = (5 * doy + 2) / 153;
    yoe + era * 400 + i64::from(month >= 10)
}

/// The `nth` Sunday of a month, as days since the epoch.
fn nth_sunday(year: i64, month: i64, nth: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    // 1970-01-01 was a Thursday, so day 3 was a Sunday.
    first + (3 - first).rem_euclid(7) + 7 * (nth - 1)
}

/// Seconds Pacific time is off UTC at `unix`: -7h in daylight saving time (second Sunday of March,
/// 2:00, to the first Sunday of November, 2:00), else -8h.
fn pacific_offset(unix: u64) -> i64 {
    let unix = unix as i64;
    let year = civil_year(unix.div_euclid(86_400));
    let starts = nth_sunday(year, 3, 2) * 86_400 + 10 * 3_600;
    let ends = nth_sunday(year, 11, 1) * 86_400 + 9 * 3_600;
    if (starts..ends).contains(&unix) { -7 * 3_600 } else { -8 * 3_600 }
}

/// When the current day started on the provider's quota clock: Google resets free quotas at midnight
/// Pacific time, the others (OpenRouter, Cerebras, Groq…) at midnight UTC.
pub fn day_start(provider_id: &str, unix: u64) -> u64 {
    let offset = if provider_id == "gemini" { pacific_offset(unix) } else { 0 };
    let local = unix as i64 + offset;
    (local - local.rem_euclid(86_400) - offset).max(0) as u64
}

/// How long until the provider's daily quotas reset.
pub fn until_daily_reset(provider_id: &str) -> std::time::Duration {
    let now = unix_now();
    std::time::Duration::from_secs(next_day(provider_id, now).saturating_sub(now).max(60))
}

/// The start of the provider's next quota day, right across daylight-saving changes.
pub(crate) fn next_day(provider_id: &str, unix: u64) -> u64 {
    day_start(provider_id, day_start(provider_id, unix) + 26 * 3_600)
}

/// The first of the next month, midnight UTC: when monthly allowances (Z.AI's plan limits, Mistral's
/// and others' monthly quotas) start over.
pub(crate) fn next_month(unix: u64) -> u64 {
    let days = (unix / 86_400) as i64;
    let year = civil_year(days);
    // The month is the one whose first day is the latest not after today.
    let month = (1..=12).rev().find(|month| days_from_civil(year, *month, 1) <= days).unwrap_or(1);
    let (year, month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    (days_from_civil(year, month, 1) * 86_400) as u64
}

/// How long until a monthly allowance starts over.
pub fn until_monthly_reset() -> std::time::Duration {
    let now = unix_now();
    std::time::Duration::from_secs(next_month(now).saturating_sub(now).max(60))
}

/// Requests Neru sent to a provider today, over all its models and keys: how busy that free plan is.
pub fn requests_today(provider_id: &str) -> u64 {
    let today = day_start(provider_id, unix_now());
    let prefix = format!("{provider_id}|");
    COUNTS
        .lock()
        .map(|counts| counts.iter().filter(|(key, counters)| key.starts_with(&prefix) && counters.day.start == today).map(|(_, counters)| counters.day.requests).sum())
        .unwrap_or(0)
}

// ---------- Neru's own counts ----------

#[derive(Clone, Default, Debug)]
struct Window {
    start: u64,
    requests: u64,
    tokens: u64,
}

impl Window {
    fn add(&mut self, start: u64, tokens: u64) {
        if self.start != start {
            *self = Window { start, requests: 0, tokens: 0 };
        }
        self.requests += 1;
        self.tokens += tokens;
    }
}

#[derive(Clone, Default, Debug)]
struct Counters {
    minute: Window,
    hour: Window,
    day: Window,
}

impl Counters {
    fn as_quotas(&self, provider_id: &str, now: u64) -> Vec<Quota> {
        if self.day.start != day_start(provider_id, now) || self.day.requests == 0 {
            return Vec::new();
        }
        let resets = Some(human(next_day(provider_id, now).saturating_sub(now)));
        vec![
            Quota { label: format!("Requests today: {} (counted by Neru)", self.day.requests), limit: None, remaining: None, resets_in: resets.clone() },
            Quota { label: format!("Tokens today: {} (counted by Neru)", self.day.tokens), limit: None, remaining: None, resets_in: resets },
        ]
    }
}

/// Counts one request Neru sent and the tokens it used, on the provider's own clocks.
pub fn note_request(config: &ProviderConfig, tokens: u64) {
    let now = unix_now();
    if let Ok(mut counts) = COUNTS.lock() {
        let entry = counts.entry(ledger_key(config)).or_default();
        entry.minute.add(now - now % 60, tokens);
        entry.hour.add(now - now % 3_600, tokens);
        entry.day.add(day_start(&config.provider_id, now), tokens);
        save_days(&counts);
    }
}

// ---------- blocks: Retry-After and spent quotas ----------

/// The provider asked to wait (Retry-After): nothing goes to this key before then.
pub fn note_retry_after(config: &ProviderConfig, wait: std::time::Duration) {
    let seconds = wait.as_secs().max(1);
    block_until(config, unix_now() + seconds, format!("the provider asked to wait {}", human(seconds)));
}

/// A daily quota is spent: nothing goes to this key until the provider's day turns over.
pub fn note_daily_exhausted(config: &ProviderConfig) {
    block_until(config, next_day(&config.provider_id, unix_now()), "its quota per day is used up".into());
}

/// A monthly allowance is spent: nothing goes to this key until the month turns over.
pub fn note_monthly_exhausted(config: &ProviderConfig) {
    block_until(config, next_month(unix_now()), "its quota per month is used up".into());
}

fn block_until(config: &ProviderConfig, until: u64, reason: String) {
    if let Ok(mut blocked) = BLOCKED.lock() {
        let entry = blocked.entry(ledger_key(config)).or_insert((0, String::new()));
        // A longer block already in place wins: Retry-After is a minimum, never a shortcut.
        if until > entry.0 {
            *entry = (until, reason);
        }
    }
}

/// What to do before sending a request to a key.
#[derive(Debug, PartialEq)]
pub enum Preflight {
    Send,
    /// A per-minute limit or a short Retry-After clears soon: wait, then send.
    Wait(std::time::Duration),
    /// The key cannot take this request for a while: switch models now instead of spending a request
    /// on a 429. The text reads like a provider's rate-limit error, so the usual model switch handles it.
    Switch(String),
}

pub(crate) fn human(seconds: u64) -> String {
    if seconds >= 3_600 {
        format!("{}h{}m", seconds / 3_600, seconds % 3_600 / 60)
    } else if seconds >= 60 {
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

// ---------- free capacity across keys ----------

/// What one free plan has left today, for the meter beside the message box.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapacity {
    pub provider_id: String,
    /// The provider the conversation uses now.
    pub current: bool,
    /// Requests Neru sent to this provider today, over all its models.
    pub used_today: u64,
    /// The daily request allowance, as the provider reported it or as its free plan documents it.
    pub limit: Option<u64>,
    pub left: Option<u64>,
    /// Models on this key resting after a limit, and how soon the first is back.
    pub resting_models: usize,
    pub back_in: Option<String>,
    /// When the provider's day starts over.
    pub resets_in: String,
}

/// Daily request allowances free plans document but do not report in headers (registry.rs).
fn documented_daily(provider_id: &str) -> Option<u64> {
    crate::registry::with(|registry| registry.daily_limits.get(provider_id).copied())
}

pub fn capacity(provider_id: &str, current: bool) -> ProviderCapacity {
    let now = unix_now();
    let prefix = format!("{provider_id}|");
    let today = day_start(provider_id, now);
    let used_today = requests_today(provider_id);
    // A daily allowance the provider reported today, on any of its models: the least left wins.
    let reported = QUOTAS.lock().ok().and_then(|quotas| {
        quotas
            .iter()
            .filter(|(key, (_, at, _))| key.starts_with(&prefix) && *at >= today)
            .flat_map(|(_, (_, _, list))| list.iter().filter(|quota| matches!(quota.label.as_str(), "Requests per day" | "Free-model requests per day")).cloned())
            .min_by_key(|quota| quota.remaining.unwrap_or(u64::MAX))
    });
    let limit = reported.as_ref().and_then(|quota| quota.limit).or_else(|| documented_daily(provider_id));
    let left = reported.as_ref().and_then(|quota| quota.remaining).or_else(|| documented_daily(provider_id).map(|limit| limit.saturating_sub(used_today)));
    let resting: Vec<u64> = BLOCKED.lock().map(|blocked| blocked.iter().filter(|(key, (until, _))| key.starts_with(&prefix) && *until > now).map(|(_, (until, _))| until - now).collect()).unwrap_or_default();
    ProviderCapacity {
        provider_id: provider_id.to_string(),
        current,
        used_today,
        limit,
        left,
        resting_models: resting.len(),
        back_in: resting.iter().min().map(|seconds| human(*seconds)),
        resets_in: human(next_day(provider_id, now).saturating_sub(now)),
    }
}

/// Every free plan with a saved key (and the one in use), for the meter beside the message box.
#[tauri::command]
pub fn free_capacity(state: tauri::State<'_, crate::AppState>) -> Result<Vec<ProviderCapacity>, String> {
    let current = state.provider.lock().map_err(|e| e.to_string())?.provider_id.clone();
    let mut ids: Vec<String> = state.provider_keys.lock().map_err(|e| e.to_string())?.keys().filter_map(|key| key.split('\n').next().map(str::to_string)).collect();
    ids.push(current.clone());
    ids.sort();
    ids.dedup();
    Ok(ids.into_iter().filter(|id| crate::fallback::FREE_PROVIDERS.contains(&id.as_str())).map(|id| capacity(&id, id == current)).collect())
}

/// Checks the ledger before sending `tokens` to this key: a block (Retry-After, a spent day), an hourly or
/// daily quota the last response said is spent, or a per-minute budget this request would overrun.
pub fn preflight(config: &ProviderConfig, tokens: usize) -> Preflight {
    let now = unix_now();
    let key = ledger_key(config);
    if let Some((until, reason)) = BLOCKED.lock().ok().and_then(|blocked| blocked.get(&key).cloned()).filter(|(until, _)| *until > now) {
        let left = until - now;
        if left <= 60 {
            return Preflight::Wait(std::time::Duration::from_secs(left) + std::time::Duration::from_millis(300));
        }
        let daily = if reason.contains("per day") || reason.contains("per month") { "" } else { " (rate limit)" };
        return Preflight::Switch(format!("Provider HTTP 429 Too Many Requests: {} on this key: {reason}{daily}; resets in {}. Neru switched before sending.", config.model, human(left)));
    }
    if let Some((at, at_unix, list)) = QUOTAS.lock().ok().and_then(|quotas| quotas.get(&key).cloned()) {
        for quota in list.iter().filter(|quota| quota.label.ends_with("per day") || quota.label.ends_with("per hour")) {
            let spent = match quota.remaining {
                Some(0) => true,
                Some(left) => quota.label.starts_with("Tokens") && (left as usize) < tokens,
                None => false,
            };
            if !spent {
                continue;
            }
            // How long the report holds: its reset time, else until the provider's day (or hour) turns.
            let left = match quota.resets_in.as_deref().and_then(parse_reset) {
                Some(reset) => reset.saturating_sub(at.elapsed()).as_secs(),
                None if quota.label.ends_with("per day") => next_day(&config.provider_id, at_unix).saturating_sub(now),
                None => (at_unix - at_unix % 3_600 + 3_600).saturating_sub(now),
            };
            if left > 60 {
                return Preflight::Switch(format!("Provider HTTP 429 Too Many Requests: {} used up on this key for {}; resets in {}. Neru switched before sending.", quota.label.to_lowercase(), config.model, human(left)));
            }
        }
    }
    match wait_before(config, tokens) {
        Some(wait) => Preflight::Wait(wait),
        None => Preflight::Send,
    }
}

/// A reset time as providers write it: "7.66s", "2m59.56s", "1h2m", "20ms", or plain seconds.
pub fn parse_reset(text: &str) -> Option<std::time::Duration> {
    let text = text.trim();
    if let Ok(seconds) = text.parse::<f64>() {
        return Some(std::time::Duration::from_secs_f64(seconds.max(0.0)));
    }
    let (mut total, mut number, mut rest) = (0.0, String::new(), text);
    while let Some(c) = rest.chars().next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            rest = &rest[1..];
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        let (unit, len) = if rest.starts_with("ms") { (0.001, 2) } else if rest.starts_with('h') { (3600.0, 1) } else if rest.starts_with('m') { (60.0, 1) } else if rest.starts_with('s') { (1.0, 1) } else { return None };
        total += value * unit;
        rest = &rest[len..];
    }
    number.is_empty().then(|| std::time::Duration::from_secs_f64(total))
}

/// How long to wait before sending `tokens` more to `model` so a per-minute limit has reset: the
/// last response said no requests are left, or fewer tokens than this request needs. None to send
/// now; waits longer than a minute are left to the model switch.
pub fn wait_before(config: &ProviderConfig, tokens: usize) -> Option<std::time::Duration> {
    let (at, _, list) = QUOTAS.lock().ok()?.get(&ledger_key(config)).cloned()?;
    let elapsed = at.elapsed();
    let wait = list
        .iter()
        .filter(|quota| quota.label.ends_with("per minute"))
        .filter(|quota| match quota.remaining {
            Some(0) => true,
            Some(left) => quota.label.starts_with("Tokens") && (left as usize) < tokens && quota.limit.is_none_or(|limit| tokens as u64 <= limit),
            None => false,
        })
        .filter_map(|quota| quota.resets_in.as_deref().and_then(parse_reset))
        .map(|reset| reset.saturating_sub(elapsed))
        .max()?;
    (wait > std::time::Duration::from_millis(200) && wait <= std::time::Duration::from_secs(60)).then(|| wait + std::time::Duration::from_millis(300))
}

/// Context window of one entry in a `/models` list, in the field each provider uses.
fn listed_window(item: &Value) -> Option<usize> {
    let number = |value: &Value| value.as_u64().or_else(|| value.as_str().and_then(|text| text.parse().ok()));
    let direct = [
        &item["context_window"],
        &item["context_length"],
        &item["max_context_length"],
        &item["max_model_len"],
        &item["inputTokenLimit"],
        &item["input_token_limit"],
        &item["max_input_tokens"],
        &item["top_provider"]["context_length"],
        &item["limits"]["max_context_window_tokens"],
    ]
    .into_iter()
    .find_map(number);
    // Hugging Face's router lists one entry per serving provider.
    let hosted = item["providers"]
        .as_array()
        .and_then(|list| list.iter().filter_map(|entry| number(&entry["context_length"])).max());
    direct.or(hosted).map(|tokens| tokens as usize).filter(|tokens| *tokens >= 1_000)
}

/// Remembers the context windows in a provider's model list. Returns how many it found.
pub fn record_windows(body: &Value, provider_id: &str) -> usize {
    let Some(items) = body["data"].as_array().or_else(|| body["models"].as_array()) else {
        return 0;
    };
    let found: Vec<(String, usize)> = items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str().or_else(|| item["name"].as_str())?;
            let id = if provider_id == "gemini" { id.trim_start_matches("models/") } else { id };
            Some((id.to_string(), listed_window(item)?))
        })
        .collect();
    if let (false, Ok(mut store)) = (found.is_empty(), STORE.lock()) {
        store.windows.extend(found.iter().cloned());
        save(&store);
    }
    found.len()
}

/// Human period of a limit, from a header suffix or what the provider documents.
fn period(provider_id: &str, kind: &str, suffix: Option<&str>) -> &'static str {
    match suffix {
        Some("minute") => "minute",
        Some("hour") => "hour",
        Some("day") => "day",
        // Groq documents its plain requests header as a daily cap and tokens as per minute.
        _ if provider_id == "groq" && kind == "requests" => "day",
        _ => "minute",
    }
}

/// Reads `x-ratelimit-*` (OpenAI, Groq, Cerebras, Mistral…) and `anthropic-ratelimit-*` headers.
pub fn parse_headers(provider_id: &str, headers: &HeaderMap) -> Vec<Quota> {
    // (kind, period) -> (limit, remaining, reset)
    let mut groups: Vec<((String, &'static str), (Option<u64>, Option<u64>, Option<String>))> = Vec::new();
    for (name, value) in headers {
        let name = name.as_str().to_ascii_lowercase();
        let Ok(value) = value.to_str() else { continue };
        let parsed = if let Some(rest) = name.strip_prefix("x-ratelimit-") {
            // limit-requests, remaining-tokens-day, reset-requests-minute
            let (field, rest) = rest.split_once('-').unwrap_or((rest, ""));
            let (kind, suffix) = match rest.split_once('-') {
                Some((kind, suffix)) => (kind, Some(suffix)),
                None => (rest, None),
            };
            Some((field.to_string(), kind.to_string(), suffix.map(str::to_string)))
        } else if let Some(rest) = name.strip_prefix("anthropic-ratelimit-") {
            // requests-limit, input-tokens-remaining, tokens-reset
            rest.rsplit_once('-').map(|(kind, field)| (field.to_string(), kind.replace('-', " "), None))
        } else {
            None
        };
        let Some((field, kind, suffix)) = parsed else { continue };
        if !matches!(field.as_str(), "limit" | "remaining" | "reset") || !(kind.contains("requests") || kind.contains("tokens")) {
            continue;
        }
        let key = (kind.clone(), period(provider_id, &kind, suffix.as_deref()));
        let position = groups.iter().position(|(existing, _)| *existing == key).unwrap_or_else(|| {
            groups.push((key, (None, None, None)));
            groups.len() - 1
        });
        let entry = &mut groups[position].1;
        match field.as_str() {
            "limit" => entry.0 = value.trim().parse().ok(),
            "remaining" => entry.1 = value.trim().parse().ok(),
            _ => entry.2 = Some(value.trim().to_string()),
        }
    }
    let mut quotas: Vec<Quota> = groups
        .into_iter()
        .filter(|(_, (limit, remaining, _))| limit.is_some() || remaining.is_some())
        .map(|((kind, period), (limit, remaining, resets_in))| Quota {
            label: format!("{}{} per {period}", kind[..1].to_uppercase(), &kind[1..]),
            limit,
            remaining,
            resets_in,
        })
        .collect();
    quotas.sort_by(|a, b| a.label.cmp(&b.label));
    quotas
}

/// Records the quota headers of a chat response; a per-minute token cap also bounds request size.
pub fn record_headers(config: &ProviderConfig, headers: &HeaderMap) {
    // The headers are the first byte of the answer: the model's latency (fallback.rs health).
    crate::fallback::note_first_byte(config);
    let quotas = parse_headers(&config.provider_id, headers);
    if quotas.is_empty() {
        return;
    }
    let per_minute_tokens = quotas
        .iter()
        .filter(|quota| quota.label.contains("okens per minute") && !quota.label.contains("Output"))
        .filter_map(|quota| quota.limit)
        .min();
    if let Some(cap) = per_minute_tokens {
        // Only a cap below the window matters; large paid limits leave the window in charge.
        if (cap as usize) < providers::context_window(&config.model) {
            set_request_cap(&config.provider_id, &config.model, cap as usize);
        }
    }
    set_quotas(config, quotas);
}

/// OpenRouter reports free-model requests and credit left on `GET /key` rather than in headers.
async fn openrouter_quotas(client: &reqwest::Client, config: &ProviderConfig) -> Result<Vec<Quota>, String> {
    let request = providers::authorize(client.get(format!("{}/key", config.base_url)), config)
        .timeout(std::time::Duration::from_secs(10));
    let body = providers::read_response(request.send().await.map_err(|e| e.to_string())?).await?;
    let data = &body["data"];
    let mut quotas = Vec::new();
    let free = &data["free_model_daily_requests"];
    if free.is_object() {
        quotas.push(Quota {
            label: "Free-model requests per day".into(),
            limit: free["limit"].as_u64(),
            remaining: free["remaining"].as_u64(),
            resets_in: Some("midnight UTC".into()),
        });
    }
    if let Some(remaining) = data["limit_remaining"].as_f64() {
        quotas.push(Quota {
            label: "Credit left (USD)".into(),
            limit: data["limit"].as_f64().map(|limit| limit.round() as u64),
            remaining: Some(remaining.round() as u64),
            resets_in: data["limit_reset"].as_str().map(str::to_string),
        });
    }
    Ok(quotas)
}

/// Fills in what the meter needs for the configured model: its window (from the model list, once)
/// and quotas that are not sent as headers.
pub async fn refresh(config: &ProviderConfig) {
    let client = reqwest::Client::new();
    if window(&config.model).is_none() {
        let request = providers::models_request(&client, config).timeout(std::time::Duration::from_secs(10));
        if let Ok(response) = request.send().await {
            if let Ok(body) = providers::read_response(response).await {
                record_windows(&body, &config.provider_id);
            }
        }
    }
    if config.provider_id == "openrouter" {
        if let Ok(list) = openrouter_quotas(&client, config).await {
            set_quotas(config, list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_counts_today_and_what_is_left() {
        let key = |model: &str| ProviderConfig { provider_id: "modelscope".into(), api_format: "openai-chat".into(), base_url: "https://cap.test/v1".into(), model: model.into(), api_key: "cap-test".into() };
        note_request(&key("cap/a"), 10);
        note_request(&key("cap/b"), 10);
        let before = capacity("modelscope", true);
        assert!(before.used_today >= 2 && before.current);
        assert_eq!(before.limit, Some(2_000), "the documented allowance when nothing was reported");
        assert_eq!(before.left, Some(2_000 - before.used_today));
        note_daily_exhausted(&key("cap/a"));
        let after = capacity("modelscope", true);
        assert_eq!(after.resting_models, 1);
        assert!(after.back_in.is_some());
        let unknown = capacity("nvidia-cap-test", false);
        assert_eq!((unknown.limit, unknown.left, unknown.resting_models), (None, None, 0));
    }
    use reqwest::header::{HeaderName, HeaderValue};

    #[test]
    fn monthly_allowances_reset_on_the_first_of_the_next_month() {
        let day = |y, m, d| (days_from_civil(y, m, d) * 86_400) as u64;
        assert_eq!(next_month(day(2026, 11, 1) + 3_600), day(2026, 12, 1));
        assert_eq!(next_month(day(2026, 12, 31) + 80_000), day(2027, 1, 1));
        assert_eq!(next_month(day(2028, 2, 29)), day(2028, 3, 1));
        assert!(until_monthly_reset().as_secs() >= 60);
        let busy = config("test-busy-today", "busy/model", "k");
        note_request(&busy, 10);
        note_request(&ProviderConfig { model: "busy/other".into(), ..busy.clone() }, 10);
        assert_eq!(requests_today("test-busy-today"), 2, "all of a provider's models count");
    }

    #[test]
    fn reset_times_read_in_every_provider_format() {
        use std::time::Duration;
        assert_eq!(parse_reset("7.5s"), Some(Duration::from_millis(7_500)));
        assert_eq!(parse_reset("2m30s"), Some(Duration::from_secs(150)));
        assert_eq!(parse_reset("20ms"), Some(Duration::from_millis(20)));
        assert_eq!(parse_reset("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_reset("12"), Some(Duration::from_secs(12)));
        assert_eq!(parse_reset("soon"), None);
    }

    fn config(provider_id: &str, model: &str, key: &str) -> ProviderConfig {
        ProviderConfig { provider_id: provider_id.into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: key.into(), model: model.into() }
    }

    #[test]
    fn requests_wait_for_a_spent_minute_but_not_a_spent_day() {
        let quota = |label: &str, remaining, reset: &str| Quota { label: label.into(), limit: Some(30), remaining: Some(remaining), resets_in: Some(reset.into()) };
        let minute = config("test-pace", "pace/minute", "k");
        set_quotas(&minute, vec![quota("Requests per minute", 0, "12s")]);
        let wait = wait_before(&minute, 1_000).unwrap();
        assert!(wait > std::time::Duration::from_secs(11) && wait <= std::time::Duration::from_secs(13));
        let tokens = config("test-pace", "pace/tokens", "k");
        set_quotas(&tokens, vec![Quota { label: "Tokens per minute".into(), limit: Some(6_000), remaining: Some(500), resets_in: Some("5s".into()) }]);
        assert!(wait_before(&tokens, 2_000).is_some(), "not enough tokens left this minute");
        assert!(wait_before(&tokens, 400).is_none(), "enough left");
        let day = config("test-pace", "pace/day", "k");
        set_quotas(&day, vec![quota("Requests per day", 0, "5h")]);
        assert!(wait_before(&day, 10).is_none(), "a daily cap is the model switch's job");
        assert!(matches!(preflight(&day, 10), Preflight::Switch(text) if text.contains("per day")), "…which preflight hands it");
        assert_eq!(preflight(&config("test-pace", "pace/unknown", "k"), 10), Preflight::Send);
    }

    #[test]
    fn quotas_are_kept_per_key_and_provider() {
        let first = config("test-ledger", "shared-model", "key-one");
        set_quotas(&first, vec![Quota { label: "Requests per day".into(), limit: Some(50), remaining: Some(0), resets_in: Some("3h".into()) }]);
        assert_eq!(quotas(&first).len(), 1);
        assert!(quotas(&config("test-ledger", "shared-model", "key-two")).is_empty(), "another key has its own quota");
        assert!(quotas(&config("test-other", "shared-model", "key-one")).is_empty(), "so does another provider");
        assert!(!ledger_key(&first).contains("key-one"), "the key itself is never part of the ledger");
    }

    #[test]
    fn retry_after_is_a_minimum_and_long_blocks_switch() {
        let short = config("test-block", "short", "k");
        note_retry_after(&short, std::time::Duration::from_secs(20));
        assert!(matches!(preflight(&short, 10), Preflight::Wait(wait) if wait >= std::time::Duration::from_secs(19)));
        // A later, shorter Retry-After does not shorten the wait.
        note_retry_after(&short, std::time::Duration::from_secs(2));
        assert!(matches!(preflight(&short, 10), Preflight::Wait(wait) if wait >= std::time::Duration::from_secs(19)));
        let long = config("test-block", "long", "k");
        note_retry_after(&long, std::time::Duration::from_secs(600));
        assert!(matches!(preflight(&long, 10), Preflight::Switch(text) if crate::fallback::is_rate_limited(&text) && !crate::fallback::is_daily(&text)));
        let day = config("gemini", "test-day", "k");
        note_daily_exhausted(&day);
        assert!(matches!(preflight(&day, 10), Preflight::Switch(text) if crate::fallback::is_daily(&text)));
    }

    #[test]
    fn days_reset_on_each_providers_clock() {
        // 2026-11-01 08:30 UTC is 01:30 PDT on the day daylight saving ends.
        let unix = days_from_civil(2026, 11, 1) as u64 * 86_400 + 8 * 3_600 + 30 * 60;
        assert_eq!(day_start("openrouter", unix), days_from_civil(2026, 11, 1) as u64 * 86_400);
        assert_eq!(day_start("gemini", unix), days_from_civil(2026, 11, 1) as u64 * 86_400 + 7 * 3_600, "midnight PDT");
        // In January Pacific time is UTC-8.
        let winter = days_from_civil(2027, 1, 15) as u64 * 86_400 + 3 * 3_600;
        assert_eq!(day_start("gemini", winter), days_from_civil(2027, 1, 14) as u64 * 86_400 + 8 * 3_600);
        assert_eq!(next_day("gemini", unix), days_from_civil(2026, 11, 2) as u64 * 86_400 + 8 * 3_600, "the next day starts in PST");
        assert_eq!(civil_year(days_from_civil(2024, 2, 29)), 2024);
        assert_eq!(nth_sunday(2026, 3, 2), days_from_civil(2026, 3, 8));
    }

    #[test]
    fn neru_counts_what_providers_do_not_report() {
        let gemini = config("gemini", "test-count", "k");
        note_request(&gemini, 1_200);
        note_request(&gemini, 800);
        let list = quotas(&gemini);
        assert!(list[0].label.starts_with("Requests today: 2"));
        assert!(list[1].label.starts_with("Tokens today: 2000"));
    }
    use serde_json::json;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn reads_groq_and_cerebras_quota_headers() {
        let groq = parse_headers("groq", &headers(&[
            ("x-ratelimit-limit-requests", "1000"),
            ("x-ratelimit-remaining-requests", "987"),
            ("x-ratelimit-reset-requests", "18m43s"),
            ("x-ratelimit-limit-tokens", "8000"),
            ("x-ratelimit-remaining-tokens", "5120"),
        ]));
        assert_eq!(groq.len(), 2);
        assert_eq!(groq[0], Quota { label: "Requests per day".into(), limit: Some(1000), remaining: Some(987), resets_in: Some("18m43s".into()) });
        assert_eq!(groq[1].label, "Tokens per minute");
        assert_eq!(groq[1].limit, Some(8000));
        let cerebras = parse_headers("cerebras", &headers(&[
            ("x-ratelimit-limit-tokens-day", "1000000"),
            ("x-ratelimit-remaining-tokens-day", "940000"),
            ("x-ratelimit-limit-requests-minute", "5"),
        ]));
        assert!(cerebras.iter().any(|quota| quota.label == "Tokens per day" && quota.remaining == Some(940_000)));
        assert!(cerebras.iter().any(|quota| quota.label == "Requests per minute" && quota.limit == Some(5)));
    }

    #[test]
    fn reads_windows_from_model_lists() {
        assert_eq!(listed_window(&json!({"id":"a","context_window":131072})), Some(131_072));
        assert_eq!(listed_window(&json!({"id":"b","top_provider":{"context_length":262144}})), Some(262_144));
        assert_eq!(listed_window(&json!({"id":"c","providers":[{"context_length":32768},{"context_length":131072}]})), Some(131_072));
        assert_eq!(listed_window(&json!({"id":"d"})), None);
    }
}
