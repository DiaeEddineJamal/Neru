//! What each provider can actually serve. Turns a provider's `/models` answer into classified records
//! (text only, text + vision, reasoning, code, tools), drops ids that are not chat models, and checks
//! with a one-token request whether a model is callable with the user's key.
//!
//! Classification uses the provider's own metadata when the list carries it (OpenRouter modalities,
//! Mistral capabilities, Ollama capabilities, Hugging Face serving status…) and falls back to name
//! rules for lists that are only ids (NVIDIA, OpenAI, Groq, Cerebras…).

use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, ProviderConfig, providers, workspace::data_dir};

// ---------- records ----------

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

/// One model a provider offers, classified.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// "chat", "reasoning" or "code": what the model is mostly for.
    pub kind: String,
    pub modalities: Modalities,
    pub vision: bool,
    pub tools: bool,
    pub reasoning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<usize>,
    pub free: bool,
    /// "ok" (a test request succeeded), "unavailable" (the provider refused it) or "unknown".
    pub verified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_reason: Option<String>,
    /// "metadata" when the provider said what the model can do, "name" when Neru guessed from the id.
    pub source: String,
}

// ---------- name rules ----------

fn tokens(id: &str) -> Vec<String> {
    id.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn has_token(tokens: &[String], wanted: &str) -> bool {
    tokens.iter().any(|token| token == wanted)
}

/// Ids that are never chat models on any provider: embeddings, rerankers, safety classifiers, speech,
/// image and video generators, OCR/parsers, and science models.
const NOT_CHAT: &[&str] = &[
    "embed", "rerank", "reward", "guard", "safety", "shield", "moderation", "whisper", "transcribe", "speech",
    "realtime", "audio", "dall-e", "gpt-image", "imagen", "imagine", "sora", "flux", "stable-diffusion", "stablediffusion",
    "sdxl", "genmo", "mochi", "video", "imagegen", "-image", "image-", "cogview", "cogvideo", "wanx", "wan2", "nvclip",
    "paligemma", "deplot", "kosmos", "fuyu", "davinci", "babbage", "text-ada", "text-curie", "computer-use", "deep-research",
    "search-preview", "gpt-3.5-turbo-instruct", "native-audio", "-live-", "lyria", "veo-", "nemoretriever", "cosmos-predict",
    "cosmos-transfer", "cosmos-embed", "alphafold", "openfold", "diffdock", "genmol", "molmim", "proteinmpnn", "rfdiffusion",
    "colabfold", "corrdiff", "fourcastnet", "earth2", "cuopt", "audio2face", "studiovoice", "eyecontact", "lipsync",
    "background-noise", "fastpitch", "parakeet", "canary", "cosyvoice", "paraformer", "sensevoice", "orpheus", "playai",
    "text-embedding", "retrieval", "reranker", "classifier", "prompt-guard", "jailbreak", "topic-control", "gliner",
    "vista3d", "yolox", "dinov2", "consistory", "trellis", "evo2", "bionemo", "paddleocr", "robotics", "vidu", "seedream", "seedance",
];

/// Short ids that only count as a whole word ("tts" but not "attstuff").
const NOT_CHAT_WORDS: &[&str] = &[
    "asr", "tts", "stt", "clip", "esm", "esm2", "esmfold", "ocr", "sd", "sd3", "sdxl", "vad", "riva", "sana", "boltz", "boltz2", "maisi",
    "parse", "bge", "e5", "aqa",
];

/// Legacy vision endpoints on NVIDIA that answer to a different API than chat completions.
const NVIDIA_LEGACY_VLM: &[&str] = &["neva", "vila", "llava", "kosmos", "fuyu", "paligemma", "deplot"];

/// True when the id can be used as a chat model on this provider.
pub fn is_chat_id(provider_id: &str, id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if lower.starts_with("ft:") || lower.starts_with("models/embedding") {
        return false;
    }
    if NOT_CHAT.iter().any(|needle| lower.contains(needle)) {
        return false;
    }
    let words = tokens(&lower);
    if NOT_CHAT_WORDS.iter().any(|word| has_token(&words, word)) {
        // "nemotron-parse", "paddleocr" and friends are not chat; but "bge" in a word never matches here.
        return false;
    }
    if provider_id == "nvidia" && NVIDIA_LEGACY_VLM.iter().any(|word| has_token(&words, word)) {
        return false;
    }
    if provider_id == "nvidia" && matches!(lower.split('/').next(), Some("ipd" | "arc" | "baai" | "black-forest-labs" | "stabilityai" | "openfold" | "mit")) {
        return false;
    }
    true
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Traits {
    vision: bool,
    reasoning: bool,
    code: bool,
    tools: bool,
}

fn glm_vision(words: &[String]) -> bool {
    has_token(words, "glm")
        && words.iter().any(|token| token.len() >= 2 && token.ends_with('v') && token[..token.len() - 1].chars().all(|c| c.is_ascii_digit()))
}

fn name_vision(lower: &str, words: &[String]) -> bool {
    let has = |part: &str| lower.contains(part);
    if has("vision") || has("llava") || has("pixtral") || has("multimodal") || has("omni") || has("molmo") || has("internvl") || has("minicpm-v") || has("cogvlm") {
        return true;
    }
    if has_token(words, "vl") || has_token(words, "vlm") || glm_vision(words) || has("cosmos-reason") || has("phi-4-multimodal") {
        return true;
    }
    if has("gemma-3") && !has("gemma-3-1b") && !has("gemma-3-270m") {
        return true;
    }
    if has("gemma-3n") || has("llama-4") || has("gpt-4o") || has("gpt-4.1") || has("gpt-4.5") || has("gpt-5") || has("gemini") {
        return true;
    }
    if has_token(words, "o4") || ((has_token(words, "o1") || has_token(words, "o3")) && !has_token(words, "mini")) {
        return true;
    }
    if has("claude") {
        return !(has("claude-2") || has("claude-instant") || has("claude-1"));
    }
    if has("mistral-large-3") || has("mistral-medium") || has("mistral-small-3.1") || has("mistral-small-3.2") || has("mistral-small-4") || has("ministral-3") || has("magistral") {
        return true;
    }
    if has("grok-4") || has("grok-2-vision") || has("grok-vision") {
        return true;
    }
    if has("kimi-k2.5") || has("kimi-k3") || has("kimi-vl") || has("kimi-latest") {
        return true;
    }
    false
}

fn name_reasoning(lower: &str, words: &[String]) -> bool {
    let has = |part: &str| lower.contains(part);
    if has("thinking") || has("reasoner") || has("reasoning") || has("qwq") || has("qvq") || has("gpt-oss") || has("magistral") || has("gpt-5") {
        return true;
    }
    if has_token(words, "r1") || has_token(words, "o4") || (has_token(words, "o1") || has_token(words, "o3")) {
        return true;
    }
    if has("kimi-k2.5") || has("kimi-k3") || has("minimax-m") || has("glm-4.5") || has("glm-4.6") || has("glm-4.7") || has("glm-5") {
        return true;
    }
    if has("gemini-2.5") || has("gemini-3") || has("grok-4") || has("grok-3-mini") || has("grok-code") || has("deepseek-v3.1") || has("deepseek-v3.2") || has("deepseek-v4") {
        return true;
    }
    if has("claude") && !(has("claude-2") || has("claude-instant") || has("claude-3-haiku") || has("claude-3-opus") || has("claude-3-5") || has("claude-3-sonnet")) {
        return true;
    }
    has("nemotron") && (has("super") || has("ultra") || has("nano") || has("reason") || has("nemotron-3"))
}

fn name_code(lower: &str) -> bool {
    ["coder", "codestral", "devstral", "codegemma", "codellama", "code-llama", "codex", "starcoder", "-code-", "grok-code", "codeqwen"]
        .iter()
        .any(|part| lower.contains(part))
        || lower.ends_with("-code")
}

/// Models that cannot take a `tools` list, or are known to reject it.
fn name_no_tools(lower: &str, words: &[String]) -> bool {
    let has = |part: &str| lower.contains(part);
    has("compound")
        || has("gemma-1")
        || has("gemma-2")
        || has("gemma-3")
        || has("llava")
        || has("-base")
        || has("phi-3")
        || has("llama-3.2-11b-vision")
        || has("llama-3.2-90b-vision")
        || has("falcon")
        || has("chatglm")
        || has("baichuan")
        || has("dbrx")
        || has("solar-")
        || has("yi-")
        || has_token(words, "qvq")
        || (has_token(words, "r1") && !has("0528") && !has("r1-0"))
        || has("o1-preview")
        || has("o1-mini")
}

fn traits_from_name(id: &str) -> Traits {
    let lower = id.to_ascii_lowercase();
    let words = tokens(&lower);
    Traits {
        vision: name_vision(&lower, &words),
        reasoning: name_reasoning(&lower, &words),
        code: name_code(&lower),
        tools: !name_no_tools(&lower, &words),
    }
}

// ---------- metadata ----------

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| list.iter().filter_map(|item| item.as_str().map(str::to_ascii_lowercase)).collect())
        .unwrap_or_default()
}

/// A capability that is either `true` or `{ "supported": true }`.
fn flag(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| value["supported"].as_bool())
}

fn zero_price(value: &Value) -> bool {
    match value {
        Value::String(text) => text.trim().parse::<f64>().map(|n| n == 0.0).unwrap_or(false),
        Value::Number(number) => number.as_f64() == Some(0.0),
        _ => false,
    }
}

fn model_id(item: &Value, provider_id: &str) -> Option<String> {
    let id = item["id"].as_str().or_else(|| item["name"].as_str()).or_else(|| item["model"].as_str())?;
    let id = if provider_id == "gemini" { id.trim_start_matches("models/") } else { id };
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// Classifies one entry of a provider's model list. None when it is not a chat model this key can call.
fn info_from_item(item: &Value, provider_id: &str) -> Option<ModelInfo> {
    let id = model_id(item, provider_id)?;
    if item["active"].as_bool() == Some(false) {
        return None;
    }
    if provider_id == "opencode" && (id.starts_with("gemini-") || id.starts_with("jev-")) {
        return None;
    }
    if let Some(kind) = item["type"].as_str() {
        let kind = kind.to_ascii_lowercase();
        if ["embed", "image", "audio", "moderation", "rerank", "video", "tts", "stt", "speech", "transcri"].iter().any(|part| kind.contains(part)) {
            return None;
        }
    }
    if !is_chat_id(provider_id, &id) {
        return None;
    }

    let guess = traits_from_name(&id);
    let mut vision: Option<bool> = None;
    let mut tools: Option<bool> = None;
    let mut reasoning: Option<bool> = None;
    let mut input = strings(&item["architecture"]["input_modalities"]);
    if input.is_empty() {
        input = strings(&item["input_modalities"]);
    }
    if input.is_empty() {
        input = strings(&item["modalities"]["input"]);
    }
    let mut output = strings(&item["architecture"]["output_modalities"]);
    if output.is_empty() {
        output = strings(&item["output_modalities"]);
    }
    if output.is_empty() {
        output = strings(&item["modalities"]["output"]);
    }
    if !output.is_empty() && !output.iter().any(|kind| kind == "text") {
        return None;
    }
    if !input.is_empty() {
        vision = Some(input.iter().any(|kind| kind == "image"));
    }

    // OpenRouter and Hugging Face: parameters and serving status.
    let parameters = strings(&item["supported_parameters"]);
    if !parameters.is_empty() {
        tools = Some(parameters.iter().any(|p| p == "tools"));
        reasoning = Some(parameters.iter().any(|p| p == "reasoning" || p == "include_reasoning"));
    }
    if let Some(hosts) = item["providers"].as_array().filter(|hosts| !hosts.is_empty()) {
        if !hosts.iter().any(|host| host["status"].as_str().is_none_or(|status| status == "live")) {
            return None;
        }
        if hosts.iter().any(|host| host["supports_tools"].is_boolean()) {
            tools = Some(hosts.iter().any(|host| host["supports_tools"].as_bool() == Some(true)));
        }
    }

    // Mistral and Anthropic: a capabilities object.
    let capabilities = &item["capabilities"];
    if capabilities.is_object() {
        if capabilities["completion_chat"].as_bool() == Some(false) {
            return None;
        }
        if let Some(value) = flag(&capabilities["vision"]).or_else(|| flag(&capabilities["image_input"])) {
            vision = Some(value);
        }
        if let Some(value) = flag(&capabilities["function_calling"]).or_else(|| flag(&capabilities["tools"])) {
            tools = Some(value);
        }
        if let Some(value) = flag(&capabilities["thinking"]).or_else(|| flag(&capabilities["reasoning"])) {
            reasoning = Some(value);
        }
    }
    // Ollama: a capabilities list.
    let listed = strings(capabilities);
    if !listed.is_empty() {
        if !listed.iter().any(|c| c == "completion" || c == "chat") {
            return None;
        }
        vision = Some(listed.iter().any(|c| c == "vision"));
        tools = Some(listed.iter().any(|c| c == "tools"));
        reasoning = Some(listed.iter().any(|c| c == "thinking"));
    }
    // Moonshot.
    if let Some(value) = item["supports_image_in"].as_bool() {
        vision = Some(value);
    }
    if let Some(value) = item["supports_reasoning"].as_bool() {
        reasoning = Some(value);
    }
    // Gemini's own list.
    if let Some(methods) = item["supportedGenerationMethods"].as_array() {
        if !methods.iter().any(|m| m.as_str() == Some("generateContent")) {
            return None;
        }
        if let Some(value) = item["thinking"].as_bool() {
            reasoning = Some(value);
        }
        // Every Gemini model reads images; Gemma served through the same list mostly does not take tools.
        if id.to_ascii_lowercase().starts_with("gemini") {
            vision = Some(true);
            tools = Some(true);
        }
    }

    let from_metadata = vision.is_some() || tools.is_some() || reasoning.is_some() || !input.is_empty();
    let vision = vision.unwrap_or(guess.vision);
    let reasoning = reasoning.unwrap_or(guess.reasoning);
    let tools = tools.unwrap_or(guess.tools);
    let mut modalities_in = vec!["text".to_string()];
    for kind in ["image", "audio", "video", "file"] {
        let wanted = if kind == "image" { vision } else { input.iter().any(|item| item == kind) };
        if wanted {
            modalities_in.push(kind.to_string());
        }
    }
    let lower = id.to_ascii_lowercase();
    let free = lower.ends_with(":free")
        || lower == "openrouter/free"
        || (zero_price(&item["pricing"]["prompt"]) && zero_price(&item["pricing"]["completion"]));
    let name = ["display_name", "displayName", "name"]
        .iter()
        .find_map(|key| item[*key].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != id && !name.starts_with("models/"))
        .map(str::to_string);
    let kind = if guess.code {
        "code"
    } else if reasoning {
        "reasoning"
    } else {
        "chat"
    };
    Some(ModelInfo {
        id,
        name,
        kind: kind.into(),
        modalities: Modalities { input: modalities_in, output: vec!["text".into()] },
        vision,
        tools,
        reasoning,
        context_window: item_window(item),
        free,
        verified: "unknown".into(),
        verified_reason: None,
        source: if from_metadata { "metadata" } else { "name" }.into(),
    })
}

fn item_window(item: &Value) -> Option<usize> {
    let number = |value: &Value| value.as_u64().or_else(|| value.as_str().and_then(|text| text.parse().ok()));
    [
        &item["context_window"],
        &item["context_length"],
        &item["max_context_length"],
        &item["max_model_len"],
        &item["inputTokenLimit"],
        &item["max_input_tokens"],
        &item["top_provider"]["context_length"],
    ]
    .into_iter()
    .find_map(number)
    .or_else(|| item["providers"].as_array().and_then(|list| list.iter().filter_map(|entry| number(&entry["context_length"])).max()))
    .map(|tokens| tokens as usize)
    .filter(|tokens| *tokens >= 1_000)
}

/// Classified chat models in a provider's `/models` answer. Non-chat ids are dropped, duplicates merged.
pub fn parse_models(body: &Value, provider_id: &str) -> Result<Vec<ModelInfo>, String> {
    let items = body["data"]
        .as_array()
        .or_else(|| body["models"].as_array())
        .ok_or("Provider model list has an unexpected shape")?;
    let mut seen = std::collections::HashSet::new();
    Ok(items
        .iter()
        .filter_map(|item| info_from_item(item, provider_id))
        .filter(|info| seen.insert(info.id.clone()))
        .collect())
}

// ---------- learned capabilities ----------

static KNOWN: LazyLock<Mutex<HashMap<String, (bool, bool)>>> = LazyLock::new(Default::default);

fn known_key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}\n{model}")
}

pub fn remember(provider_id: &str, models: &[ModelInfo]) {
    if let Ok(mut known) = KNOWN.lock() {
        for info in models {
            known.insert(known_key(provider_id, &info.id), (info.vision, info.source == "metadata"));
        }
    }
}

/// True when the provider itself said this model cannot read images.
pub fn certainly_text_only(provider_id: &str, model: &str) -> bool {
    KNOWN
        .lock()
        .ok()
        .and_then(|known| known.get(&known_key(provider_id, model)).copied())
        .is_some_and(|(vision, from_provider)| !vision && from_provider)
}

// ---------- probe outcome ----------

#[derive(Debug, PartialEq, Clone)]
pub enum Verdict {
    Ok(String),
    Unavailable(String),
    /// Rate limits, overload, timeouts and unclear errors: says nothing about the model.
    Unknown(String),
    RateLimited(String),
    BadKey(String),
}

impl Verdict {
    fn status(&self) -> &'static str {
        match self {
            Self::Ok(_) => "ok",
            Self::Unavailable(_) => "unavailable",
            Self::Unknown(_) | Self::RateLimited(_) => "unknown",
            Self::BadKey(_) => "badKey",
        }
    }

    fn reason(&self) -> &str {
        match self {
            Self::Ok(text) | Self::Unavailable(text) | Self::Unknown(text) | Self::RateLimited(text) | Self::BadKey(text) => text,
        }
    }
}

/// The provider says this id is not a model it serves (to this account).
const STRONG_UNAVAILABLE: &[&str] = &[
    "model_not_found", "model not found", "model_not_supported", "does not exist", "no such model", "unknown model", "invalid model",
    "not a valid model", "not a chat model", "not a chat-completions", "does not support chat", "decommission", "no longer available",
    "no longer supported", "been deprecated", "has been retired", "has been shut down", "not found for account", "function not found",
    "is not supported by the provider", "model is not supported", "unsupported model", "not available for this account",
    "not enabled for this account", "degraded function", "do not have access to", "don't have access to", "not authorized to access",
    "not deployed", "model access denied", "no endpoints found",
];

/// A test parameter Neru sent was refused; the model itself exists.
const PARAMETER_ISSUE: &[&str] = &[
    "max_tokens", "max_completion_tokens", "max_output_tokens", "unsupported parameter", "unsupported value", "temperature",
    "stream_options", "tool_choice", "unrecognized request argument", "unknown parameter", "extra inputs are not permitted",
];

const BAD_KEY: &[&str] = &["invalid api key", "invalid_api_key", "incorrect api key", "invalid token", "authentication", "unauthenticated", "api key not valid", "api_key_invalid", "expired"];

/// The error (lowercase) is about the key itself, not the model.
pub fn is_bad_key_text(lower: &str) -> bool {
    BAD_KEY.iter().any(|marker| lower.contains(marker))
}

/// What a check reports for a free model the provider only serves inside its own app. The app's
/// name comes from the provider's message ("…can only be used from within OpenCode").
pub fn client_locked_reason(message: &str) -> String {
    let lower = message.to_lowercase();
    let app = ["from within ", "used within ", "used in ", "available in ", "available within ", "works in "]
        .iter()
        .find_map(|marker| lower.find(marker).map(|at| at + marker.len()))
        .and_then(|start| message.get(start..))
        .map(|rest| rest.split(['.', ',', ';', '"', '\n']).next().unwrap_or("").trim())
        .filter(|name| !name.is_empty() && name.len() <= 40)
        .map_or_else(|| "the provider's own app".to_string(), |name| name.trim_start_matches("the ").to_string());
    format!("Its free tier only works inside {app}, not in other apps such as Neru. Pick a paid model this key has credits for, or another provider")
}

fn excerpt(body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value["error"]["message"]
                .as_str()
                .or_else(|| value["error"].as_str())
                .or_else(|| value["message"].as_str())
                .or_else(|| value["detail"].as_str())
                .or_else(|| value[0]["error"]["message"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());
    let message: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    message.chars().take(160).collect()
}

/// Turns an HTTP answer to the test request into what it says about the model.
pub fn classify_probe(status: u16, body: &str) -> Verdict {
    let lower = body.to_ascii_lowercase();
    let said = excerpt(body);
    let said_or = |fallback: &str| if said.is_empty() { fallback.to_string() } else { said.clone() };
    let strong = STRONG_UNAVAILABLE.iter().any(|marker| lower.contains(marker))
        || (lower.contains("function") && lower.contains("not found"));
    match status {
        200..=299 => Verdict::Ok("Answered a test request".into()),
        401 => Verdict::BadKey("The provider rejected the API key".into()),
        429 => Verdict::RateLimited("Rate limited right now; try again in a minute".into()),
        402 => Verdict::Unknown(format!("The provider asks for credits or a paid plan ({})", said_or("HTTP 402"))),
        403 if crate::fallback::is_client_locked(&format!("http 403 {lower}")) => Verdict::Unavailable(client_locked_reason(&said)),
        403 => {
            if is_bad_key_text(&lower) {
                Verdict::BadKey("The provider rejected the API key".into())
            } else if lower.contains("quota") || lower.contains("rate limit") {
                Verdict::Unknown(said_or("Quota reached"))
            } else {
                Verdict::Unavailable(format!("This key has no access to the model ({})", said_or("HTTP 403")))
            }
        }
        // A bare "404 page not found" is the gateway itself, not an answer about the model (NVIDIA
        // says "Function … not found for account" for a model it does not serve).
        404 if !strong && lower.contains("404 page not found") => Verdict::Unknown("The provider's gateway answered 404; try again".into()),
        404 | 410 => Verdict::Unavailable(format!("Not served by this provider ({})", said_or(&format!("HTTP {status}")))),
        400 | 405 | 415 | 422 => {
            if strong {
                Verdict::Unavailable(said_or("Not supported by the provider"))
            } else if PARAMETER_ISSUE.iter().any(|marker| lower.contains(marker)) {
                Verdict::Ok("The model exists (it refused a test parameter)".into())
            } else if (lower.contains("not supported") || lower.contains("unsupported")) && lower.contains("model") {
                Verdict::Unavailable(said_or("Not supported by the provider"))
            } else {
                Verdict::Unknown(said_or(&format!("HTTP {status}")))
            }
        }
        408 | 425 => Verdict::Unknown("The provider timed out".into()),
        500..=599 => {
            if strong {
                Verdict::Unavailable(said_or("Not available on this provider"))
            } else {
                Verdict::Unknown(format!("The provider had a problem (HTTP {status})"))
            }
        }
        _ => Verdict::Unknown(said_or(&format!("HTTP {status}"))),
    }
}

/// Same rules for the text of a failed chat request ("Provider HTTP 404 Not Found: …").
pub fn classify_error_text(error: &str) -> Option<Verdict> {
    let lower = error.to_ascii_lowercase();
    let start = lower.find("http ")? + 5;
    let status: u16 = lower[start..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some(classify_probe(status, error))
}

// ---------- cache of probe results ----------

const TTL_SECONDS: u64 = 24 * 3600;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Entry {
    status: String,
    reason: String,
    at: u64,
}

type Scopes = HashMap<String, HashMap<String, Entry>>;

static CACHE: LazyLock<Mutex<Scopes>> = LazyLock::new(|| Mutex::new(load_cache()));

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn cache_path() -> Option<std::path::PathBuf> {
    data_dir().ok().map(|dir| dir.join("model-availability.json"))
}

fn load_cache() -> Scopes {
    let mut scopes: Scopes = cache_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let cutoff = now().saturating_sub(TTL_SECONDS);
    for entries in scopes.values_mut() {
        // Gateway 404s were once stored as "unavailable"; they said nothing about the model.
        entries.retain(|_, entry| entry.at >= cutoff && !entry.reason.contains("404 page not found"));
    }
    scopes.retain(|_, entries| !entries.is_empty());
    scopes
}

fn save_cache(scopes: &Scopes) {
    if let (Some(path), Ok(text)) = (cache_path(), serde_json::to_string(scopes)) {
        let _ = std::fs::write(path, text);
    }
}

/// Provider, endpoint and a fingerprint of the key. The key itself is never stored.
pub fn scope(config: &ProviderConfig) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.provider_id.hash(&mut hasher);
    config.api_key.hash(&mut hasher);
    format!("{}\n{}\n{:016x}", config.provider_id, config.base_url.trim_end_matches('/'), hasher.finish())
}

fn cached(config: &ProviderConfig, model: &str) -> Option<Entry> {
    let cache = CACHE.lock().ok()?;
    let entry = cache.get(&scope(config))?.get(model)?;
    (entry.at + TTL_SECONDS > now()).then(|| entry.clone())
}

fn store(config: &ProviderConfig, model: &str, verdict: &Verdict) {
    let status = verdict.status();
    if !matches!(status, "ok" | "unavailable") {
        return;
    }
    if let Ok(mut cache) = CACHE.lock() {
        cache
            .entry(scope(config))
            .or_default()
            .insert(model.to_string(), Entry { status: status.into(), reason: verdict.reason().into(), at: now() });
        save_cache(&cache);
    }
}

/// True when a recent check found this model unavailable.
#[cfg(test)]
fn known_unavailable(config: &ProviderConfig, model: &str) -> bool {
    cached(config, model).is_some_and(|entry| entry.status == "unavailable")
}

/// Records a chat request that failed with "this model does not exist / is not supported".
pub fn note_failure(config: &ProviderConfig, error: &str) {
    if let Some(verdict @ Verdict::Unavailable(_)) = classify_error_text(error) {
        store(config, &config.model, &verdict);
    }
}

fn apply_cache(config: &ProviderConfig, models: &mut [ModelInfo]) {
    for info in models {
        if let Some(entry) = cached(config, &info.id) {
            info.verified = entry.status;
            info.verified_reason = Some(entry.reason);
        }
    }
}

// ---------- listing ----------

/// Ids the provider serves that are not known to be unavailable and can call tools: what the automatic
/// fallback may switch to.
pub async fn usable_ids(config: &ProviderConfig) -> Vec<String> {
    let client = Client::new();
    let Ok(mut models) = fetch_list(&client, config).await else { return Vec::new() };
    apply_cache(config, &mut models);
    models
        .into_iter()
        .filter(|info| info.verified != "unavailable" && info.tools)
        .map(|info| info.id)
        .collect()
}

/// Models MiniMax documents; its API has no reliable model list.
const MINIMAX_MODELS: &[&str] = &["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.7-highspeed", "MiniMax-M2.5", "MiniMax-M2.5-highspeed", "MiniMax-M2.1"];

fn provider_root(base_url: &str, suffix: &str) -> Option<String> {
    base_url.trim_end_matches('/').strip_suffix(suffix).map(str::to_string)
}

async fn json_ok(request: reqwest::RequestBuilder) -> Option<Value> {
    let response = request.timeout(Duration::from_secs(12)).send().await.ok()?;
    providers::read_response(response).await.ok()
}

/// Gemini's own list says what each model can do; the OpenAI-compatible one is only ids.
async fn gemini_native(client: &Client, config: &ProviderConfig) -> Option<Value> {
    let root = provider_root(&config.base_url, "/openai")?;
    let request = client.get(format!("{root}/models?pageSize=1000")).header("x-goog-api-key", &config.api_key);
    let body = json_ok(request).await?;
    body["models"].is_array().then_some(body)
}

async fn xai_language_models(client: &Client, config: &ProviderConfig) -> Option<Value> {
    let request = providers::authorize(client.get(format!("{}/language-models", config.base_url)), config);
    let body = json_ok(request).await?;
    (body["models"].is_array() || body["data"].is_array()).then_some(body)
}

/// Adds what Ollama knows about each installed model (`/api/show`): capabilities and context length.
async fn ollama_details(client: &Client, config: &ProviderConfig, body: &mut Value) {
    let Some(root) = provider_root(&config.base_url, "/v1") else { return };
    let ids: Vec<String> = body["data"]
        .as_array()
        .map(|items| items.iter().filter_map(|item| item["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let mut jobs = Vec::new();
    for id in ids.into_iter().take(64) {
        let client = client.clone();
        let url = format!("{root}/api/show");
        jobs.push(tauri::async_runtime::spawn(async move {
            let response = client.post(url).json(&json!({"model": id})).timeout(Duration::from_secs(6)).send().await.ok()?;
            let details: Value = response.json().await.ok()?;
            Some((id, details))
        }));
    }
    let mut shown: HashMap<String, Value> = HashMap::new();
    for job in jobs {
        if let Ok(Some((id, details))) = job.await {
            shown.insert(id, details);
        }
    }
    let Some(items) = body["data"].as_array_mut() else { return };
    for item in items {
        let Some(details) = item["id"].as_str().and_then(|id| shown.get(id)) else { continue };
        if details["capabilities"].is_array() {
            item["capabilities"] = details["capabilities"].clone();
        }
        let window = details["model_info"]
            .as_object()
            .and_then(|info| info.iter().find(|(key, _)| key.ends_with(".context_length")).and_then(|(_, value)| value.as_u64()));
        if let Some(window) = window {
            item["context_length"] = json!(window);
        }
    }
}

/// OpenRouter's list for this key: `/models/user` leaves out models the account's privacy settings,
/// provider preferences or guardrails block (the ones that fail with "No endpoints found"). Models
/// without tool calling cannot run Neru's agent, and a key on the free tier cannot call paid models,
/// so both are left out too.
async fn openrouter_for_key(client: &Client, config: &ProviderConfig) -> Option<Vec<ModelInfo>> {
    let request = providers::authorize(client.get(format!("{}/models/user", config.base_url)), config);
    let body = json_ok(request).await.filter(|body| body["data"].is_array())?;
    let key = json_ok(providers::authorize(client.get(format!("{}/key", config.base_url)), config)).await;
    let free_tier = key.as_ref().is_some_and(|key| key["data"]["is_free_tier"].as_bool() == Some(true));
    crate::limits::record_windows(&body, &config.provider_id);
    let mut models = parse_models(&body, &config.provider_id).ok()?;
    models.retain(|info| info.tools && (!free_tier || info.free));
    Some(models)
}

async fn fetch_list(client: &Client, config: &ProviderConfig) -> Result<Vec<ModelInfo>, String> {
    let provider_id = config.provider_id.as_str();
    if provider_id == "gemini" {
        if let Some(body) = gemini_native(client, config).await {
            crate::limits::record_windows(&body, provider_id);
            return parse_models(&body, provider_id);
        }
    }
    if provider_id == "xai" {
        if let Some(body) = xai_language_models(client, config).await {
            crate::limits::record_windows(&body, provider_id);
            return parse_models(&body, provider_id);
        }
    }
    if provider_id == "openrouter" && !config.api_key.is_empty() {
        if let Some(models) = openrouter_for_key(client, config).await {
            return Ok(models);
        }
    }
    let request = providers::models_request(client, config).timeout(Duration::from_secs(15));
    let fetched = match request.send().await {
        Ok(response) => providers::read_response(response).await,
        Err(error) => Err(error.to_string()),
    };
    let mut body = match fetched {
        Ok(body) => body,
        Err(error) if provider_id == "minimax" && (error.contains("HTTP 404") || error.contains("HTTP 405") || error.contains("unexpected")) => {
            return Ok(MINIMAX_MODELS.iter().filter_map(|id| info_from_item(&json!({"id": id}), provider_id)).collect());
        }
        Err(error) => return Err(error),
    };
    if provider_id == "ollama" {
        ollama_details(client, config, &mut body).await;
    }
    crate::limits::record_windows(&body, provider_id);
    let mut models = parse_models(&body, provider_id)?;
    if provider_id == "nvidia" {
        apply_nvidia_checks(&mut models);
    }
    Ok(models)
}

/// NVIDIA's catalog lists far more models than a free key can call: most answer "Function not found
/// for account". Each model was checked with a free key (see `live_probe`); the ones that refused are
/// left out, the ones that answered start confirmed. Models NVIDIA adds later are checked in the
/// background the first time the list is loaded (`verify_new`).
fn apply_nvidia_checks(models: &mut Vec<ModelInfo>) {
    models.retain(|info| !NVIDIA_NOT_SERVED.contains(&info.id.as_str()));
    for info in models.iter_mut() {
        if NVIDIA_SERVED.contains(&info.id.as_str()) {
            info.verified = "ok".into();
            info.verified_reason = Some("Answered a test request with a free key".into());
        }
    }
}

/// NVIDIA models that answered a test request from a free key (checked 2026-09-30).
const NVIDIA_SERVED: &[&str] = &[
    "deepseek-ai/deepseek-v4.1-flash", "google/diffusiongemma-26b-a4b-it", "google/gemma-4-31b-it", "meta/llama-3.2-11b-vision-instruct",
    "meta/muse-glimmer-30b", "nvidia/ising-calibration-1.5-31b", "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning", "nvidia/nemotron-3-super-120b-a12b",
    "nvidia/nemotron-3-ultra-550b-a55b", "nvidia/nemotron-3.5-lightning-30b-a3b", "openai/gpt-oss-20b", "poolside/laguna-xs-2.1", "z-ai/glm-5.3",
    "z-ai/glm-5.3-flash",
];

/// NVIDIA models listed in the catalog that a free key cannot call (checked 2026-09-30).
const NVIDIA_NOT_SERVED: &[&str] = &[
    "01-ai/yi-large", "ai21labs/jamba-1.5-large-instruct", "aisingapore/sea-lion-7b-instruct", "bigcode/starcoder2-15b", "databricks/dbrx-instruct",
    "deepseek-ai/deepseek-coder-6.7b-instruct", "google/codegemma-1.1-7b", "google/codegemma-7b", "google/gemma-2b", "google/gemma-3-12b-it",
    "google/gemma-3-4b-it", "google/recurrentgemma-2b", "ibm/granite-3.0-3b-a800m-instruct", "ibm/granite-3.0-8b-instruct",
    "ibm/granite-34b-code-instruct", "ibm/granite-8b-code-instruct", "meta/codellama-70b", "meta/llama2-70b", "microsoft/phi-3-vision-128k-instruct",
    "microsoft/phi-3.5-moe-instruct", "mistralai/codestral-22b-instruct-v0.1", "mistralai/mistral-7b-instruct-v0.3", "mistralai/mistral-large",
    "mistralai/mistral-large-2-instruct", "mistralai/mixtral-8x22b-v0.1", "moonshotai/kimi-k2.6", "nv-mistralai/mistral-nemo-12b-instruct",
    "nvidia/cosmos-reason2-8b", "nvidia/llama-3.1-nemotron-51b-instruct", "nvidia/llama-3.1-nemotron-70b-instruct",
    "nvidia/llama-3.1-nemotron-ultra-253b-v1", "nvidia/llama3-chatqa-1.5-70b", "nvidia/mistral-nemo-minitron-8b-8k-instruct",
    "nvidia/nemotron-4-340b-instruct", "nvidia/nemotron-nano-3-30b-a3b", "writer/palmyra-creative-122b", "writer/palmyra-fin-70b-32k",
    "writer/palmyra-med-70b", "writer/palmyra-med-70b-32k", "zyphra/zamba2-7b-instruct",
];

static VERIFYING: AtomicBool = AtomicBool::new(false);

/// Checks, one at a time and within the free plan's pace, the models nobody has checked yet, so the
/// next listing drops the ones the provider refuses. Runs once at a time; at most 25 per run.
fn verify_new(config: &ProviderConfig, models: &[ModelInfo]) {
    if config.provider_id != "nvidia" || config.api_key.is_empty() {
        return;
    }
    let unchecked: Vec<String> = models.iter().filter(|info| info.verified == "unknown").map(|info| info.id.clone()).take(25).collect();
    if unchecked.is_empty() || VERIFYING.swap(true, Ordering::SeqCst) {
        return;
    }
    let template = config.clone();
    tauri::async_runtime::spawn(async move {
        let client = Client::new();
        for model in unchecked {
            let config = ProviderConfig { model: model.clone(), api_format: "openai-chat".into(), ..template.clone() };
            let verdict = probe_once(&client, &config).await;
            store(&config, &model, &verdict);
            if matches!(verdict, Verdict::RateLimited(_) | Verdict::BadKey(_)) {
                break;
            }
            tokio::time::sleep(probe_spacing(&config.provider_id)).await;
        }
        VERIFYING.store(false, Ordering::SeqCst);
    });
}

fn resolve(provider_id: String, api_format: String, base_url: String, api_key: String, model: String, app: &AppHandle) -> Result<ProviderConfig, String> {
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    let url = reqwest::Url::parse(&base_url).map_err(|e| e.to_string())?;
    if url.scheme() != "https" && !(url.scheme() == "http" && providers::is_local(&url)) {
        return Err("Use HTTPS, or HTTP on localhost".into());
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Enter a base URL without credentials, query, or fragment".into());
    }
    if !providers::valid_format(&api_format) {
        return Err("Unsupported API format".into());
    }
    let state = app.state::<AppState>();
    let key = if api_key.trim().is_empty() {
        crate::settings::key_for(&*state.provider_keys.lock().map_err(|e| e.to_string())?, &provider_id, &base_url).unwrap_or_default()
    } else {
        api_key.trim().to_string()
    };
    Ok(ProviderConfig { provider_id, api_format, base_url, api_key: key, model: model.trim().to_string() })
}

/// Strongest coding models first, unavailable ones last.
fn sort_for_coding(models: &mut [ModelInfo]) {
    models.sort_by_key(|info| (info.verified == "unavailable", std::cmp::Reverse(crate::fallback::coding_score(&info.id))));
}

/// Classified chat models this key can list, best coding models first.
#[tauri::command]
pub async fn list_models(provider_id: String, api_format: String, base_url: String, api_key: String, app: AppHandle) -> Result<Vec<ModelInfo>, String> {
    let config = resolve(provider_id, api_format, base_url, api_key, String::new(), &app)?;
    let mut models = fetch_list(&Client::new(), &config).await?;
    apply_cache(&config, &mut models);
    verify_new(&config, &models);
    // A model this key's provider refused is not offered at all.
    models.retain(|info| info.verified != "unavailable" || info.id == config.model);
    remember(&config.provider_id, &models);
    sort_for_coding(&mut models);
    Ok(models)
}

/// Model ids only, for the terminal version.
pub async fn list_model_ids(provider_id: String, api_format: String, base_url: String, app: AppHandle) -> Result<Vec<String>, String> {
    let models = list_models(provider_id, api_format, base_url, String::new(), app).await?;
    Ok(models.into_iter().filter(|info| info.verified != "unavailable").map(|info| info.id).collect())
}

// ---------- probing ----------

/// Providers whose own model list already says what is live, or where a test request costs quota that matters.
fn probe_skip(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "openrouter" => Some("OpenRouter lists only models with a live endpoint"),
        "ollama" => Some("Installed on this computer"),
        "local" => Some("Listed by your gateway"),
        _ => None,
    }
}

/// Minimum gap between test requests, so a bulk check stays under the free plan's per-minute cap.
fn probe_spacing(provider_id: &str) -> Duration {
    Duration::from_millis(match provider_id {
        "nvidia" => 1_600,
        "cerebras" => 2_500,
        "groq" => 2_200,
        "mistral" => 1_300,
        "gemini" => 4_200,
        "modelscope" => 1_000,
        "huggingface" => 1_000,
        _ => 400,
    })
}

/// Most test requests one bulk check may send.
fn probe_budget(provider_id: &str) -> usize {
    match provider_id {
        "gemini" => 30,
        "groq" => 40,
        "cerebras" | "mistral" => 40,
        "nvidia" => 150,
        _ => 120,
    }
}

async fn probe_once(client: &Client, config: &ProviderConfig) -> Verdict {
    if let Some(reason) = probe_skip(&config.provider_id) {
        return Verdict::Ok(reason.into());
    }
    let request = providers::probe_request(client, config).timeout(Duration::from_secs(30));
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            classify_probe(status, &body)
        }
        Err(error) if error.is_timeout() => Verdict::Unknown("The provider did not answer in time".into()),
        Err(error) => Verdict::Unknown(format!("Could not reach the provider: {}", error.to_string().chars().take(100).collect::<String>())),
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub model: String,
    /// "ok", "unavailable", "unknown" or "badKey".
    pub status: String,
    pub reason: String,
    pub rate_limited: bool,
    pub cached: bool,
}

fn result_of(model: &str, verdict: &Verdict, cached: bool) -> ProbeResult {
    ProbeResult {
        model: model.to_string(),
        status: verdict.status().into(),
        reason: verdict.reason().to_string(),
        rate_limited: matches!(verdict, Verdict::RateLimited(_)),
        cached,
    }
}

/// Sends a one-token request to see whether this key can call the model. Rate limits and provider
/// trouble leave the model "unknown" instead of marking it unavailable.
#[tauri::command]
pub async fn probe_model(provider_id: String, api_format: String, base_url: String, api_key: String, model: String, force: Option<bool>, app: AppHandle) -> Result<ProbeResult, String> {
    if model.trim().is_empty() {
        return Err("Choose a model".into());
    }
    let config = resolve(provider_id, api_format, base_url, api_key, model, &app)?;
    Ok(health(&config, force.unwrap_or(false)).await)
}

/// Whether this key can call the configured model: a check from the last day when there is one
/// (unless `force`), else one test request. Used at startup, after signing in, and by doctor.
pub async fn health(config: &ProviderConfig, force: bool) -> ProbeResult {
    if !force {
        if let Some(entry) = cached(config, &config.model) {
            return ProbeResult { model: config.model.clone(), status: entry.status, reason: entry.reason, rate_limited: false, cached: true };
        }
    }
    let verdict = probe_once(&Client::new(), config).await;
    store(config, &config.model, &verdict);
    result_of(&config.model, &verdict, false)
}

/// The check from the last day, without sending anything.
pub fn cached_health(config: &ProviderConfig) -> Option<ProbeResult> {
    cached(config, &config.model).map(|entry| ProbeResult { model: config.model.clone(), status: entry.status, reason: entry.reason, rate_limited: false, cached: true })
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CheckTarget {
    pub model: String,
    pub api_format: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Progress {
    run_id: String,
    done: usize,
    total: usize,
    result: Option<ProbeResult>,
    /// Set on the last event of a run.
    finished: bool,
    note: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CheckSummary {
    pub checked: usize,
    pub total: usize,
    pub ok: usize,
    pub unavailable: usize,
    pub unknown: usize,
    pub cancelled: bool,
    pub note: Option<String>,
}

static RUNS: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = LazyLock::new(Default::default);

/// Stops a running availability check after its current requests finish.
#[tauri::command]
pub fn cancel_check_models(run_id: String) {
    if let Ok(runs) = RUNS.lock() {
        if let Some(flag) = runs.get(&run_id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

/// Checks a list of models with small concurrency, one progress event (`models://check`) per model.
/// The provider's per-minute cap and a per-run request budget are respected; results are cached.
#[tauri::command]
pub async fn check_models(run_id: String, provider_id: String, base_url: String, api_key: String, targets: Vec<CheckTarget>, app: AppHandle) -> Result<CheckSummary, String> {
    let template = resolve(provider_id, "openai-chat".into(), base_url, api_key, String::new(), &app)?;
    if let Some(reason) = probe_skip(&template.provider_id) {
        return Ok(CheckSummary { checked: 0, total: 0, ok: 0, unavailable: 0, unknown: 0, cancelled: false, note: Some(reason.into()) });
    }
    let total = targets.len();
    let cancel = Arc::new(AtomicBool::new(false));
    RUNS.lock().map_err(|e| e.to_string())?.insert(run_id.clone(), cancel.clone());

    // Models with a fresh answer need no request; report them straight away.
    let done = Arc::new(AtomicUsize::new(0));
    let mut queue: VecDeque<CheckTarget> = VecDeque::new();
    let (mut ok, mut unavailable, mut unknown) = (0usize, 0usize, 0usize);
    for target in targets {
        match cached(&template, &target.model) {
            Some(entry) => {
                match entry.status.as_str() {
                    "ok" => ok += 1,
                    "unavailable" => unavailable += 1,
                    _ => unknown += 1,
                }
                let count = done.fetch_add(1, Ordering::SeqCst) + 1;
                let result = ProbeResult { model: target.model.clone(), status: entry.status, reason: entry.reason, rate_limited: false, cached: true };
                let _ = app.emit("models://check", Progress { run_id: run_id.clone(), done: count, total, result: Some(result), finished: false, note: None });
            }
            None => queue.push_back(target),
        }
    }
    let budget = probe_budget(&template.provider_id);
    let over_budget = queue.len().saturating_sub(budget);
    queue.truncate(budget);

    let queue = Arc::new(Mutex::new(queue));
    let tally = Arc::new(Mutex::new((ok, unavailable, unknown)));
    let next_slot = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let limited = Arc::new(AtomicUsize::new(0));
    let stopped_by_limit = Arc::new(AtomicBool::new(false));
    let spacing = probe_spacing(&template.provider_id);
    let mut workers = Vec::new();
    for _ in 0..4 {
        let (queue, tally, next_slot, limited, stopped_by_limit, cancel, done, app, template, run_id) =
            (queue.clone(), tally.clone(), next_slot.clone(), limited.clone(), stopped_by_limit.clone(), cancel.clone(), done.clone(), app.clone(), template.clone(), run_id.clone());
        workers.push(tauri::async_runtime::spawn(async move {
            let client = Client::new();
            loop {
                if cancel.load(Ordering::SeqCst) || stopped_by_limit.load(Ordering::SeqCst) {
                    break;
                }
                let Some(target) = queue.lock().ok().and_then(|mut queue| queue.pop_front()) else { break };
                let config = ProviderConfig { api_format: target.api_format.clone(), model: target.model.clone(), ..template.clone() };
                let mut verdict;
                let mut attempts = 0;
                loop {
                    {
                        let mut slot = next_slot.lock().await;
                        let wait = slot.saturating_duration_since(Instant::now());
                        *slot = Instant::now().max(*slot) + spacing;
                        drop(slot);
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                    }
                    if cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    verdict = probe_once(&client, &config).await;
                    attempts += 1;
                    if matches!(verdict, Verdict::RateLimited(_)) && attempts < 2 {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                        continue;
                    }
                    break;
                }
                if matches!(verdict, Verdict::RateLimited(_)) {
                    if limited.fetch_add(1, Ordering::SeqCst) + 1 >= 3 {
                        stopped_by_limit.store(true, Ordering::SeqCst);
                    }
                } else {
                    limited.store(0, Ordering::SeqCst);
                }
                store(&config, &config.model, &verdict);
                if let Ok(mut tally) = tally.lock() {
                    match verdict.status() {
                        "ok" => tally.0 += 1,
                        "unavailable" => tally.1 += 1,
                        _ => tally.2 += 1,
                    }
                }
                let count = done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit("models://check", Progress { run_id: run_id.clone(), done: count, total, result: Some(result_of(&config.model, &verdict, false)), finished: false, note: None });
                if matches!(verdict, Verdict::BadKey(_)) {
                    stopped_by_limit.store(true, Ordering::SeqCst);
                }
            }
        }));
    }
    for worker in workers {
        let _ = worker.await;
    }
    RUNS.lock().ok().map(|mut runs| runs.remove(&run_id));
    let (ok, unavailable, unknown) = *tally.lock().map_err(|e| e.to_string())?;
    let cancelled = cancel.load(Ordering::SeqCst);
    let note = if stopped_by_limit.load(Ordering::SeqCst) {
        Some("Stopped: the provider is rate limiting or rejected the key. Try again in a few minutes.".to_string())
    } else if over_budget > 0 {
        Some(format!("Checked the first {budget}; {over_budget} more can be checked in another run (free plans have request limits)."))
    } else {
        None
    };
    let checked = done.load(Ordering::SeqCst);
    let _ = app.emit("models://check", Progress { run_id, done: checked, total, result: None, finished: true, note: note.clone() });
    Ok(CheckSummary { checked, total, ok, unavailable, unknown, cancelled, note })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(models: &[ModelInfo]) -> Vec<&str> {
        models.iter().map(|info| info.id.as_str()).collect()
    }

    fn find<'a>(models: &'a [ModelInfo], id: &str) -> &'a ModelInfo {
        models.iter().find(|info| info.id == id).unwrap_or_else(|| panic!("{id} was dropped"))
    }

    #[test]
    fn nvidia_drops_models_a_free_key_cannot_call() {
        let mut models: Vec<ModelInfo> = [NVIDIA_SERVED[0], NVIDIA_NOT_SERVED[0], "acme/brand-new-coder"].iter().filter_map(|id| info_from_item(&json!({"id": id}), "nvidia")).collect();
        apply_nvidia_checks(&mut models);
        assert_eq!(ids(&models), vec![NVIDIA_SERVED[0], "acme/brand-new-coder"]);
        assert_eq!(models[0].verified, "ok");
        assert_eq!(models[1].verified, "unknown");
    }

    #[test]
    fn nvidia_list_keeps_chat_models_and_drops_the_rest() {
        let chat = [
            "meta/llama-3.1-70b-instruct", "meta/llama-3.3-70b-instruct", "qwen/qwen3-coder-480b-a35b-instruct", "qwen/qwq-32b",
            "deepseek-ai/deepseek-r1", "deepseek-ai/deepseek-v3.1", "moonshotai/kimi-k2-instruct", "openai/gpt-oss-120b",
            "meta/llama-4-maverick-17b-128e-instruct", "meta/llama-3.2-11b-vision-instruct", "microsoft/phi-4-multimodal-instruct",
            "nvidia/llama-3.1-nemotron-nano-vl-8b-v1", "google/gemma-3-27b-it", "mistralai/mistral-large-2-instruct", "z-ai/glm4.7",
            "minimaxai/minimax-m2", "nvidia/llama-3.3-nemotron-super-49b-v1", "mistralai/codestral-22b-instruct-v0.1",
            "qwen/qwen2.5-coder-32b-instruct", "ibm/granite-3.3-8b-instruct",
        ];
        let not_chat = [
            "nvidia/nv-embed-v1", "nvidia/nv-embedqa-e5-v5", "nvidia/llama-3.2-nv-embedqa-1b-v2", "snowflake/arctic-embed-l", "baai/bge-m3",
            "nvidia/nvclip", "nvidia/llama-3.2-nv-rerankqa-1b-v2", "nvidia/rerank-qa-mistral-4b", "meta/llama-guard-4-12b",
            "nvidia/llama-3.1-nemoguard-8b-content-safety", "nvidia/llama-3.1-nemotron-70b-reward", "nvidia/nemotron-4-340b-reward",
            "nvidia/parakeet-ctc-1.1b-asr", "nvidia/canary-1b-asr", "nvidia/fastpitch-hifigan-tts", "nvidia/riva-translate-4b-instruct",
            "google/paligemma", "google/shieldgemma-9b", "ibm/granite-guardian-3.0-8b", "arc/evo2-40b", "meta/esm2-650m",
            "nvidia/corrdiff", "nvidia/fourcastnet", "deepmind/alphafold2", "nvidia/nemoretriever-parse", "nvidia/nemotron-ocr-v1",
            "microsoft/kosmos-2", "nvidia/neva-22b", "nvidia/vila", "black-forest-labs/flux.1-dev", "stabilityai/stable-diffusion-xl",
            "nvidia/cosmos-predict1-7b", "baidu/paddleocr", "nvidia/gliner-pii", "ipd/rfdiffusion", "mit/boltz2", "nvidia/audio2face-3d",
            "adept/fuyu-8b", "nvidia/studiovoice",
        ];
        let items: Vec<Value> = chat.iter().chain(not_chat.iter()).map(|id| json!({"id": id, "object": "model", "owned_by": "x"})).collect();
        let parsed = parse_models(&json!({"object":"list","data":items}), "nvidia").unwrap();
        assert_eq!(ids(&parsed), chat, "chat models kept, in list order");
        for id in not_chat {
            assert!(!is_chat_id("nvidia", id), "{id} must not be offered");
        }
        assert!(find(&parsed, "meta/llama-3.2-11b-vision-instruct").vision);
        assert!(find(&parsed, "meta/llama-4-maverick-17b-128e-instruct").vision);
        assert!(find(&parsed, "nvidia/llama-3.1-nemotron-nano-vl-8b-v1").vision);
        assert!(!find(&parsed, "meta/llama-3.1-70b-instruct").vision);
        assert!(!find(&parsed, "qwen/qwen3-coder-480b-a35b-instruct").vision);
        assert!(find(&parsed, "deepseek-ai/deepseek-r1").reasoning);
        assert!(find(&parsed, "qwen/qwq-32b").reasoning);
        assert_eq!(find(&parsed, "qwen/qwen3-coder-480b-a35b-instruct").kind, "code");
        assert_eq!(find(&parsed, "meta/llama-3.1-70b-instruct").kind, "chat");
        assert_eq!(find(&parsed, "meta/llama-3.1-70b-instruct").source, "name");
    }

    #[test]
    fn name_rules_cover_the_families_in_the_brief() {
        for id in ["Qwen/Qwen2.5-VL-72B-Instruct", "llava-v1.6", "mistralai/pixtral-12b-2409", "google/gemma-3-27b-it", "meta-llama/llama-4-scout", "gpt-4o-mini", "claude-sonnet-4-5", "gemini-2.5-flash", "zai-org/GLM-4.5V", "o3", "o4-mini"] {
            assert!(traits_from_name(id).vision, "{id} should read images");
        }
        for id in ["gpt-3.5-turbo", "o1-mini", "o3-mini", "llama-3.3-70b-versatile", "deepseek-chat", "qwen3-coder-plus", "gemma-3-1b-it", "glm-4.6", "Qwen/Qwen2.5-Coder-32B-Instruct"] {
            assert!(!traits_from_name(id).vision, "{id} is text only");
        }
        for id in ["deepseek-r1", "qwq-32b", "o3-mini", "deepseek-reasoner", "kimi-k2-thinking", "Qwen/Qwen3-235B-A22B-Thinking-2507", "gpt-oss-120b"] {
            assert!(traits_from_name(id).reasoning, "{id} reasons");
        }
        for id in ["qwen3-coder-480b", "codestral-latest", "devstral-small-2505", "Qwen/Qwen2.5-Coder-32B-Instruct"] {
            assert!(traits_from_name(id).code, "{id} is a code model");
        }
        for id in [
            "text-embedding-3-small", "whisper-large-v3", "whisper-large-v3-turbo", "playai-tts", "dall-e-3", "gpt-image-1", "tts-1-hd", "omni-moderation-latest",
            "gpt-4o-transcribe", "gpt-4o-realtime-preview", "gpt-4o-audio-preview", "sora-2", "meta-llama/llama-prompt-guard-2-86m", "openai/gpt-oss-safeguard-20b",
            "gemini-2.5-flash-image", "imagen-4.0-generate", "veo-3.0", "gemini-embedding-001", "mistral-embed", "mistral-ocr-latest", "mistral-moderation-latest",
            "grok-2-image-1212", "cogview-4", "glm-ocr", "embedding-3", "text-embedding-v3", "wan2.2-t2v", "davinci-002", "gpt-3.5-turbo-instruct", "ft:gpt-4o:acme",
            "computer-use-preview", "qwen-image", "Qwen/Qwen3-Embedding-8B", "BAAI/bge-reranker-v2-m3", "black-forest-labs/FLUX.1-dev",
        ] {
            assert!(!is_chat_id("openai", id), "{id} is not a chat model");
        }
        for id in ["gpt-4o", "gpt-5", "o3", "claude-opus-4-1", "gemini-2.5-pro", "llama-3.3-70b-versatile", "mistral-large-latest", "codestral-latest", "grok-4", "deepseek-chat", "glm-4.6", "kimi-k2-0905-preview", "qwen3-coder-plus", "MiniMax-M2", "Qwen/Qwen3-Coder-480B-A35B-Instruct"] {
            assert!(is_chat_id("openai", id), "{id} is a chat model");
        }
    }

    #[test]
    fn openrouter_metadata_decides_vision_tools_and_free() {
        let body = json!({"data":[
            {"id":"qwen/qwen3-coder:free","name":"Qwen: Qwen3 Coder (free)","context_length":262144,
             "architecture":{"input_modalities":["text"],"output_modalities":["text"]},
             "pricing":{"prompt":"0","completion":"0"},"supported_parameters":["tools","tool_choice","temperature"]},
            {"id":"google/gemma-3-27b-it:free","context_length":131072,
             "architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},
             "pricing":{"prompt":"0","completion":"0"},"supported_parameters":["temperature"]},
            {"id":"openai/gpt-5","context_length":400000,
             "architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]},
             "pricing":{"prompt":"0.00000125","completion":"0.00001"},"supported_parameters":["tools","reasoning","include_reasoning"]},
            {"id":"google/gemini-2.5-flash-image","architecture":{"input_modalities":["text","image"],"output_modalities":["image","text"]}},
            {"id":"black-forest-labs/some-imager","architecture":{"input_modalities":["text"],"output_modalities":["image"]}}
        ]});
        let parsed = parse_models(&body, "openrouter").unwrap();
        assert_eq!(ids(&parsed), ["qwen/qwen3-coder:free", "google/gemma-3-27b-it:free", "openai/gpt-5"]);
        let coder = find(&parsed, "qwen/qwen3-coder:free");
        assert!(coder.free && coder.tools && !coder.vision);
        assert_eq!(coder.context_window, Some(262_144));
        assert_eq!(coder.name.as_deref(), Some("Qwen: Qwen3 Coder (free)"));
        assert_eq!(coder.source, "metadata");
        let gemma = find(&parsed, "google/gemma-3-27b-it:free");
        assert!(gemma.vision && !gemma.tools, "the list says gemma takes no tools");
        assert_eq!(gemma.modalities.input, ["text", "image"]);
        let gpt = find(&parsed, "openai/gpt-5");
        assert!(!gpt.free && gpt.vision && gpt.reasoning && gpt.tools);
        assert!(gpt.modalities.input.contains(&"file".to_string()));
    }

    #[test]
    fn gemini_native_list_drops_models_that_cannot_generate() {
        let body = json!({"models":[
            {"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash","inputTokenLimit":1048576,"supportedGenerationMethods":["generateContent","countTokens"],"thinking":true},
            {"name":"models/gemini-embedding-001","supportedGenerationMethods":["embedContent"]},
            {"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]},
            {"name":"models/imagen-4.0-generate-001","supportedGenerationMethods":["predict"]},
            {"name":"models/gemini-2.5-flash-preview-tts","supportedGenerationMethods":["generateContent"]},
            {"name":"models/aqa","supportedGenerationMethods":["generateAnswer"]},
            {"name":"models/gemma-3-27b-it","inputTokenLimit":131072,"supportedGenerationMethods":["generateContent"]}
        ]});
        let parsed = parse_models(&body, "gemini").unwrap();
        assert_eq!(ids(&parsed), ["gemini-2.5-flash", "gemma-3-27b-it"]);
        let flash = find(&parsed, "gemini-2.5-flash");
        assert!(flash.vision && flash.tools && flash.reasoning);
        assert_eq!(flash.context_window, Some(1_048_576));
        assert_eq!(flash.name.as_deref(), Some("Gemini 2.5 Flash"));
    }

    #[test]
    fn gemini_compatibility_list_falls_back_to_names() {
        let body = json!({"object":"list","data":[
            {"id":"models/gemini-2.5-flash","object":"model","owned_by":"google"},
            {"id":"models/gemini-embedding-001","object":"model"},
            {"id":"models/gemini-2.0-flash-preview-image-generation","object":"model"},
            {"id":"models/imagen-4.0-generate-001","object":"model"}
        ]});
        let parsed = parse_models(&body, "gemini").unwrap();
        assert_eq!(ids(&parsed), ["gemini-2.5-flash"]);
        assert!(parsed[0].vision && parsed[0].reasoning);
    }

    #[test]
    fn groq_list_drops_speech_and_safety_models() {
        let body = json!({"object":"list","data":[
            {"id":"llama-3.3-70b-versatile","object":"model","owned_by":"Meta","active":true,"context_window":131072},
            {"id":"openai/gpt-oss-120b","object":"model","owned_by":"OpenAI","active":true,"context_window":131072},
            {"id":"meta-llama/llama-4-scout-17b-16e-instruct","object":"model","active":true,"context_window":131072},
            {"id":"whisper-large-v3","object":"model","active":true,"context_window":448},
            {"id":"playai-tts","object":"model","active":true},
            {"id":"meta-llama/llama-guard-4-12b","object":"model","active":true},
            {"id":"meta-llama/llama-prompt-guard-2-22m","object":"model","active":true},
            {"id":"llama3-8b-8192","object":"model","active":false},
            {"id":"compound-beta","object":"model","active":true}
        ]});
        let parsed = parse_models(&body, "groq").unwrap();
        assert_eq!(ids(&parsed), ["llama-3.3-70b-versatile", "openai/gpt-oss-120b", "meta-llama/llama-4-scout-17b-16e-instruct", "compound-beta"]);
        assert!(find(&parsed, "openai/gpt-oss-120b").reasoning);
        assert!(find(&parsed, "meta-llama/llama-4-scout-17b-16e-instruct").vision);
        assert!(!find(&parsed, "compound-beta").tools);
        assert_eq!(find(&parsed, "llama-3.3-70b-versatile").context_window, Some(131_072));
    }

    #[test]
    fn mistral_capabilities_decide_chat_vision_and_tools() {
        let body = json!({"object":"list","data":[
            {"id":"mistral-large-latest","name":"mistral-large-2512","capabilities":{"completion_chat":true,"function_calling":true,"vision":true,"fine_tuning":true},"max_context_length":262144},
            {"id":"codestral-latest","capabilities":{"completion_chat":true,"completion_fim":true,"function_calling":true,"vision":false},"max_context_length":256000},
            {"id":"mistral-embed","capabilities":{"completion_chat":false,"function_calling":false,"vision":false}},
            {"id":"mistral-moderation-latest","capabilities":{"completion_chat":false}},
            {"id":"mistral-ocr-latest","capabilities":{"completion_chat":false,"vision":true}},
            {"id":"open-mistral-nemo","capabilities":{"completion_chat":true,"function_calling":true,"vision":false},"max_context_length":131072}
        ]});
        let parsed = parse_models(&body, "mistral").unwrap();
        assert_eq!(ids(&parsed), ["mistral-large-latest", "codestral-latest", "open-mistral-nemo"]);
        let large = find(&parsed, "mistral-large-latest");
        assert!(large.vision && large.tools);
        assert_eq!(large.context_window, Some(262_144));
        assert_eq!(large.source, "metadata");
        assert!(!find(&parsed, "codestral-latest").vision);
        assert_eq!(find(&parsed, "codestral-latest").kind, "code");
    }

    #[test]
    fn huggingface_router_reports_serving_status_and_modalities() {
        let body = json!({"object":"list","data":[
            {"id":"openai/gpt-oss-120b","architecture":{"input_modalities":["text"],"output_modalities":["text"]},
             "providers":[{"provider":"cerebras","status":"live","context_length":131072,"supports_tools":true},{"provider":"groq","status":"live","context_length":131072,"supports_tools":true}]},
            {"id":"Qwen/Qwen2.5-VL-7B-Instruct","architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},
             "providers":[{"provider":"hyperbolic","status":"live","context_length":32768,"supports_tools":false}]},
            {"id":"acme/retired-model","architecture":{"input_modalities":["text"],"output_modalities":["text"]},
             "providers":[{"provider":"novita","status":"error","context_length":8192}]},
            {"id":"black-forest-labs/FLUX.1-dev","architecture":{"input_modalities":["text"],"output_modalities":["image"]},"providers":[{"provider":"fal-ai","status":"live"}]}
        ]});
        let parsed = parse_models(&body, "huggingface").unwrap();
        assert_eq!(ids(&parsed), ["openai/gpt-oss-120b", "Qwen/Qwen2.5-VL-7B-Instruct"]);
        assert!(find(&parsed, "openai/gpt-oss-120b").tools);
        assert!(find(&parsed, "Qwen/Qwen2.5-VL-7B-Instruct").vision);
        assert!(!find(&parsed, "Qwen/Qwen2.5-VL-7B-Instruct").tools);
        assert_eq!(find(&parsed, "openai/gpt-oss-120b").context_window, Some(131_072));
    }

    #[test]
    fn xai_anthropic_ollama_and_moonshot_metadata() {
        let xai = json!({"models":[
            {"id":"grok-4","input_modalities":["text","image"],"output_modalities":["text"]},
            {"id":"grok-code-fast-1","input_modalities":["text"],"output_modalities":["text"]},
            {"id":"grok-imagine-image","input_modalities":["text"],"output_modalities":["image"]}
        ]});
        let parsed = parse_models(&xai, "xai").unwrap();
        assert_eq!(ids(&parsed), ["grok-4", "grok-code-fast-1"]);
        assert!(find(&parsed, "grok-4").vision && !find(&parsed, "grok-code-fast-1").vision);

        let anthropic = json!({"data":[
            {"type":"model","id":"claude-sonnet-4-5-20250929","display_name":"Claude Sonnet 4.5","max_input_tokens":200000,"capabilities":{"image_input":{"supported":true},"thinking":{"supported":true}}},
            {"type":"model","id":"claude-3-5-haiku-20241022","display_name":"Claude Haiku 3.5"}
        ]});
        let parsed = parse_models(&anthropic, "anthropic").unwrap();
        assert!(find(&parsed, "claude-sonnet-4-5-20250929").reasoning && find(&parsed, "claude-sonnet-4-5-20250929").vision);
        assert_eq!(find(&parsed, "claude-sonnet-4-5-20250929").name.as_deref(), Some("Claude Sonnet 4.5"));
        assert_eq!(find(&parsed, "claude-sonnet-4-5-20250929").context_window, Some(200_000));
        assert!(find(&parsed, "claude-3-5-haiku-20241022").vision);

        let ollama = json!({"data":[
            {"id":"qwen3-coder:30b","capabilities":["completion","tools"],"context_length":262144},
            {"id":"llama3.2-vision:11b","capabilities":["completion","vision"]},
            {"id":"nomic-embed-text:latest","capabilities":["embedding"]},
            {"id":"deepseek-r1:8b","capabilities":["completion","thinking"]}
        ]});
        let parsed = parse_models(&ollama, "ollama").unwrap();
        assert_eq!(ids(&parsed), ["qwen3-coder:30b", "llama3.2-vision:11b", "deepseek-r1:8b"]);
        assert!(find(&parsed, "qwen3-coder:30b").tools && !find(&parsed, "qwen3-coder:30b").vision);
        assert!(find(&parsed, "llama3.2-vision:11b").vision && !find(&parsed, "llama3.2-vision:11b").tools);
        assert!(find(&parsed, "deepseek-r1:8b").reasoning);

        let moonshot = json!({"data":[
            {"id":"kimi-k2-0905-preview","context_length":262144,"supports_image_in":false,"supports_video_in":false,"supports_reasoning":false},
            {"id":"kimi-k2.5","context_length":262144,"supports_image_in":true,"supports_video_in":true,"supports_reasoning":true}
        ]});
        let parsed = parse_models(&moonshot, "kimi").unwrap();
        assert!(!find(&parsed, "kimi-k2-0905-preview").vision);
        assert!(find(&parsed, "kimi-k2.5").vision && find(&parsed, "kimi-k2.5").reasoning);
    }

    #[test]
    fn openai_list_keeps_only_callable_chat_models() {
        let body = json!({"data":[
            {"id":"gpt-5"},{"id":"gpt-4o"},{"id":"o3-mini"},{"id":"text-embedding-3-large"},{"id":"whisper-1"},
            {"id":"dall-e-3"},{"id":"tts-1"},{"id":"omni-moderation-latest"},{"id":"gpt-4o-mini-tts"},{"id":"gpt-image-1"},
            {"id":"gpt-4o-realtime-preview"},{"id":"davinci-002"},{"id":"gpt-3.5-turbo-instruct"},{"id":"sora-2"}
        ]});
        let parsed = parse_models(&body, "openai").unwrap();
        assert_eq!(ids(&parsed), ["gpt-5", "gpt-4o", "o3-mini"]);
        assert!(!find(&parsed, "o3-mini").vision && find(&parsed, "o3-mini").reasoning);
    }

    #[test]
    fn model_list_needs_a_data_array_and_removes_duplicates() {
        assert!(parse_models(&json!({"error":"nope"}), "nvidia").is_err());
        let body = json!({"data":[{"id":"grok-4","active":true},{"id":"grok-4"},{"id":"grok-2-image","active":true},{"id":"llama-3.1-8b","active":false}]});
        assert_eq!(ids(&parse_models(&body, "xai").unwrap()), ["grok-4"]);
        let opencode = json!({"data":[{"id":"gemini-3-pro"},{"id":"jev-x"},{"id":"qwen3-coder"}]});
        assert_eq!(ids(&parse_models(&opencode, "opencode").unwrap()), ["qwen3-coder"]);
    }

    #[test]
    fn probe_answers_are_classified_without_blaming_busy_models() {
        assert!(matches!(classify_probe(200, "{}"), Verdict::Ok(_)));
        // NVIDIA: listed but not deployed for this account.
        let nvidia = r#"{"status":404,"title":"Not Found","detail":"Function '0f8f6b2e-1234': Not found for account 'abcd'"}"#;
        assert!(matches!(classify_probe(404, nvidia), Verdict::Unavailable(_)));
        assert!(matches!(classify_probe(404, "404 page not found"), Verdict::Unknown(_)));
        let nvidia_400 = r#"{"error":{"message":"The model `foo/bar` is not supported by the provider."}}"#;
        assert!(matches!(classify_probe(400, nvidia_400), Verdict::Unavailable(_)));
        assert!(matches!(classify_probe(400, r#"{"error":{"message":"The model gpt-9 does not exist or you do not have access to it."}}"#), Verdict::Unavailable(_)));
        assert!(matches!(classify_probe(403, r#"{"detail":"Forbidden"}"#), Verdict::Unavailable(_)));
        assert!(matches!(classify_probe(403, r#"{"error":{"message":"Invalid API key"}}"#), Verdict::BadKey(_)));
        assert!(matches!(classify_probe(401, "{}"), Verdict::BadKey(_)));
        // Transient trouble says nothing about the model.
        assert!(matches!(classify_probe(429, r#"{"error":{"message":"Rate limit reached"}}"#), Verdict::RateLimited(_)));
        assert!(matches!(classify_probe(503, "Service Unavailable"), Verdict::Unknown(_)));
        assert!(matches!(classify_probe(500, "internal error"), Verdict::Unknown(_)));
        assert!(matches!(classify_probe(402, "{}"), Verdict::Unknown(_)));
        assert!(matches!(classify_probe(400, r#"{"error":{"message":"messages must not be empty"}}"#), Verdict::Unknown(_)));
        // A refused test parameter proves the model exists.
        let param = r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead."}}"#;
        assert!(matches!(classify_probe(400, param), Verdict::Ok(_)));
        assert_eq!(classify_probe(429, "{}").status(), "unknown");
        assert_eq!(classify_probe(404, "{}").status(), "unavailable");
    }

    #[test]
    fn failed_chat_requests_are_read_the_same_way() {
        assert!(matches!(classify_error_text("Provider HTTP 404 Not Found: Function 'x': Not found for account 'y'"), Some(Verdict::Unavailable(_))));
        assert!(matches!(classify_error_text("Provider HTTP 429 Too Many Requests: slow down"), Some(Verdict::RateLimited(_))));
        assert!(classify_error_text("The model did not start answering within 75 seconds").is_none());
    }

    #[test]
    fn free_tiers_locked_to_their_own_app_say_so() {
        let body = r#"{"error":{"message":"Free tier can only be used from within OpenCode","type":"forbidden"}}"#;
        let Verdict::Unavailable(reason) = classify_probe(403, body) else { panic!("a locked free tier is unavailable") };
        assert!(reason.starts_with("Its free tier only works inside OpenCode,"), "{reason}");
        let Some(Verdict::Unavailable(reason)) = classify_error_text("Provider HTTP 403 Forbidden: Free tier can only be used from within OpenCode") else { panic!() };
        assert!(reason.contains("inside OpenCode,"), "{reason}");
        assert!(client_locked_reason("Forbidden").contains("the provider's own app"));
        // Other 403s keep their meaning.
        assert!(matches!(classify_probe(403, r#"{"error":{"message":"Invalid API key"}}"#), Verdict::BadKey(_)));
        assert!(matches!(classify_probe(402, r#"{"error":{"message":"Insufficient balance"}}"#), Verdict::Unknown(reason) if reason.contains("Insufficient balance")));
    }

    #[test]
    fn cache_scope_never_holds_the_key() {
        let config = ProviderConfig {
            provider_id: "nvidia".into(),
            api_format: "openai-chat".into(),
            base_url: "https://integrate.api.nvidia.com/v1/".into(),
            model: "m".into(),
            api_key: "nvapi-secret-value".into(),
        };
        let one = scope(&config);
        assert!(!one.contains("secret"));
        assert!(one.starts_with("nvidia\nhttps://integrate.api.nvidia.com/v1\n"));
        let other = ProviderConfig { api_key: "nvapi-another".into(), ..config.clone() };
        assert_ne!(one, scope(&other));
    }

    #[test]
    fn unavailable_answers_are_remembered_and_expire_by_scope() {
        let config = ProviderConfig {
            provider_id: "test-provider-cache".into(),
            api_format: "openai-chat".into(),
            base_url: "https://cache.example/v1".into(),
            model: "acme/gone".into(),
            api_key: "k-cache-test".into(),
        };
        assert!(!known_unavailable(&config, "acme/gone"));
        let mut models = vec![info_from_item(&json!({"id":"acme/gone"}), "nvidia").unwrap()];
        note_failure(&config, "Provider HTTP 404 Not Found: Function 'a': Not found for account 'b'");
        apply_cache(&config, &mut models);
        assert_eq!(models[0].verified, "unavailable");
        assert!(known_unavailable(&config, "acme/gone"));
        let other_key = ProviderConfig { api_key: "different".into(), ..config.clone() };
        assert!(!known_unavailable(&other_key, "acme/gone"));
        // Rate limits are not stored.
        note_failure(&other_key, "Provider HTTP 429 Too Many Requests");
        assert!(cached(&other_key, "acme/gone").is_none());
        if let Ok(mut cache) = CACHE.lock() {
            cache.remove(&scope(&config));
            save_cache(&cache);
        }
    }
}

#[cfg(test)]
mod live_probe {
    use super::*;

    /// Checks every model a saved key lists and writes the verdicts to NERU_PROBE_OUT (JSON).
    /// NERU_PROBE_PROVIDER picks the saved key. Run: cargo test --lib live_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_saved_key() {
        let (Ok(provider), Ok(out)) = (std::env::var("NERU_PROBE_PROVIDER"), std::env::var("NERU_PROBE_OUT")) else { return };
        let mut current = ProviderConfig { provider_id: String::new(), api_format: String::new(), base_url: String::new(), api_key: String::new(), model: String::new() };
        let mut keys = HashMap::new();
        crate::settings::load(&mut current, &mut keys, &mut Default::default());
        let (id, key) = keys.iter().find(|(id, _)| id.starts_with(&format!("{provider}\n"))).expect("no saved key for that provider");
        let base_url = id.split_once('\n').unwrap().1.to_string();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let client = Client::new();
            let config = ProviderConfig { provider_id: provider.clone(), api_format: "openai-chat".into(), base_url, api_key: key.clone(), model: String::new() };
            let models = fetch_list(&client, &config).await.expect("model list");
            eprintln!("{} chat models listed", models.len());
            let mut results = Vec::new();
            for (index, info) in models.iter().enumerate() {
                let probe = ProviderConfig { model: info.id.clone(), ..config.clone() };
                let mut verdict = Verdict::Unknown(String::new());
                for attempt in 0..3 {
                    verdict = probe_once(&client, &probe).await;
                    if !matches!(verdict, Verdict::RateLimited(_) | Verdict::Unknown(_)) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(if matches!(verdict, Verdict::RateLimited(_)) { 30 } else { 5 } * (attempt + 1))).await;
                }
                eprintln!("{:>3}/{} {:<12} {} {}", index + 1, models.len(), verdict.status(), info.id, verdict.reason());
                results.push(json!({"id": info.id, "status": verdict.status(), "reason": verdict.reason(), "tools": info.tools, "vision": info.vision}));
                tokio::time::sleep(probe_spacing(&provider)).await;
            }
            std::fs::write(&out, serde_json::to_string_pretty(&results).unwrap()).unwrap();
        });
    }
    /// Compares OpenRouter's public list with the per-account one. NERU_PROBE_OUT gets the ids.
    #[test]
    #[ignore]
    fn openrouter_lists() {
        let Ok(out) = std::env::var("NERU_PROBE_OUT") else { return };
        let mut current = ProviderConfig { provider_id: String::new(), api_format: String::new(), base_url: String::new(), api_key: String::new(), model: String::new() };
        let mut keys = HashMap::new();
        crate::settings::load(&mut current, &mut keys, &mut Default::default());
        let (id, key) = keys.iter().find(|(id, _)| id.starts_with("openrouter\n")).expect("no OpenRouter key");
        let config = ProviderConfig { provider_id: "openrouter".into(), api_format: "openai-chat".into(), base_url: id.split_once('\n').unwrap().1.into(), api_key: key.clone(), model: String::new() };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let client = Client::new();
            let public = parse_models(&providers::read_response(providers::models_request(&client, &config).send().await.unwrap()).await.unwrap(), "openrouter").unwrap();
            let key_info = json_ok(providers::authorize(client.get(format!("{}/key", config.base_url)), &config)).await;
            let user = openrouter_for_key(&client, &config).await.expect("models/user");
            let ids: Vec<&str> = user.iter().map(|info| info.id.as_str()).collect();
            eprintln!("public (tools) {}, public free {}, for this key {}, key {:?}", public.len(), public.iter().filter(|m| m.free).count(), user.len(), key_info.map(|k| k["data"]["is_free_tier"].clone()));
            eprintln!("openrouter/free listed: {}", ids.contains(&"openrouter/free"));
            std::fs::write(&out, serde_json::to_string_pretty(&ids).unwrap()).unwrap();
        });
    }
}
