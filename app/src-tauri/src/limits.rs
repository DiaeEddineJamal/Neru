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
/// The quotas each model's last response reported, and when.
static QUOTAS: LazyLock<Mutex<HashMap<String, (std::time::Instant, Vec<Quota>)>>> = LazyLock::new(Default::default);

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

pub fn quotas(model: &str) -> Vec<Quota> {
    QUOTAS.lock().ok().and_then(|quotas| quotas.get(model).map(|(_, list)| list.clone())).unwrap_or_default()
}

pub(crate) fn set_quotas(model: &str, list: Vec<Quota>) {
    if let Ok(mut quotas) = QUOTAS.lock() {
        quotas.insert(model.to_string(), (std::time::Instant::now(), list));
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
pub fn wait_before(model: &str, tokens: usize) -> Option<std::time::Duration> {
    let (at, list) = QUOTAS.lock().ok()?.get(model).cloned()?;
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
    set_quotas(&config.model, quotas);
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
            set_quotas(&config.model, list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

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

    #[test]
    fn requests_wait_for_a_spent_minute_but_not_a_spent_day() {
        let quota = |label: &str, remaining, reset: &str| Quota { label: label.into(), limit: Some(30), remaining: Some(remaining), resets_in: Some(reset.into()) };
        set_quotas("pace/minute", vec![quota("Requests per minute", 0, "12s")]);
        let wait = wait_before("pace/minute", 1_000).unwrap();
        assert!(wait > std::time::Duration::from_secs(11) && wait <= std::time::Duration::from_secs(13));
        set_quotas("pace/tokens", vec![Quota { label: "Tokens per minute".into(), limit: Some(6_000), remaining: Some(500), resets_in: Some("5s".into()) }]);
        assert!(wait_before("pace/tokens", 2_000).is_some(), "not enough tokens left this minute");
        assert!(wait_before("pace/tokens", 400).is_none(), "enough left");
        set_quotas("pace/day", vec![quota("Requests per day", 0, "5h")]);
        assert!(wait_before("pace/day", 10).is_none(), "a daily cap is the model switch's job");
        assert!(wait_before("pace/unknown", 10).is_none());
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
