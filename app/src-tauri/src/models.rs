//! What each provider can actually serve. Turns a provider's `/models` answer into classified records
//! (text only, text + vision, reasoning, code, tools), drops ids that are not chat models, and checks
//! with a one-token request whether a model is callable with the user's key.
//!
//! Classification uses the provider's own metadata when the list carries it (OpenRouter modalities,
//! Mistral capabilities, Ollama capabilities, Hugging Face serving status…) and falls back to name
//! rules for lists that are only ids (NVIDIA, OpenAI, Groq, Cerebras…).

use std::{
    collections::{HashMap, HashSet, VecDeque},
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
    /// "native" when the model takes a tool list, "text" when Neru describes the tools in the prompt and
    /// reads `<tool_call>` blocks back (models served without tool calling).
    pub protocol: String,
    pub reasoning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<usize>,
    pub free: bool,
    /// "ok" (a test request succeeded), "unavailable" (the provider refused it) or "unknown".
    pub verified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_reason: Option<String>,
    /// "ok" (a test request with one tool came back with a tool call), "no" (the model takes no tools, or
    /// the provider refused them) or "unknown" (not tested yet, or the test was inconclusive).
    pub tools_verified: String,
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
    // Google shut the Gemini 2.0 models down.
    if provider_id == "gemini" && id.to_ascii_lowercase().starts_with("gemini-2.0") {
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
    let free = if provider_id == "gemini" {
        gemini_free(&lower)
    } else {
        lower.ends_with(":free") || lower == "openrouter/free" || (zero_price(&item["pricing"]["prompt"]) && zero_price(&item["pricing"]["completion"]))
    };
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
        protocol: if tools { "native" } else { "text" }.into(),
        reasoning,
        context_window: item_window(item),
        free,
        verified: "unknown".into(),
        verified_reason: None,
        tools_verified: if tools { "unknown" } else { "no" }.into(),
        source: if from_metadata { "metadata" } else { "name" }.into(),
    })
}

/// Gemini models on Google's free tier (checked 2026-11-01). Gemini 3.1 Pro and the Gemini 4 "argon"
/// models are paid only; Gemma 4 is free. A test request that answers still marks a model free.
const GEMINI_FREE: &[&str] = &[
    "gemini-3.8-flash", "gemini-3.6-flash", "gemini-3.5-flash-lite", "gemini-3.1-flash-lite", "gemini-3-flash-preview", "gemini-2.5-pro",
    "gemini-2.5-flash", "gemini-2.5-flash-lite",
];

/// Whether a Gemini id (lowercase) is on the free tier: a listed id, a dated variant of one, or Gemma 4.
fn gemini_free(lower: &str) -> bool {
    let lower = lower.trim_start_matches("models/");
    lower.starts_with("gemma-4") || GEMINI_FREE.iter().any(|free| lower == *free || lower.strip_prefix(free).is_some_and(|rest| rest.starts_with('-')))
}

/// Long enough to hold a coding session's system prompt and tools, and the model passed the tool test
/// (or has not taken it yet). Code mode's automatic switch only picks models that fit.
pub fn fits_code(info: &ModelInfo) -> bool {
    info.context_window.is_none_or(|window| window >= 32_000) && info.tools_verified != "no"
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

/// Per model: (reads images, said by the provider rather than guessed, takes a tool list).
static KNOWN: LazyLock<Mutex<HashMap<String, (bool, bool, bool)>>> = LazyLock::new(Default::default);

fn known_key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}\n{model}")
}

/// Per model: whether it [`fits_code`].
static FITS: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

pub fn remember(provider_id: &str, models: &[ModelInfo]) {
    if let Ok(mut known) = KNOWN.lock() {
        for info in models {
            known.insert(known_key(provider_id, &info.id), (info.vision, info.source == "metadata", info.tools));
        }
    }
    if let Ok(mut fits) = FITS.lock() {
        for info in models {
            fits.insert(known_key(provider_id, &info.id), fits_code(info));
        }
    }
}

/// Whether Code mode may switch to this model: true unless a listing showed it under 32K context or
/// without working tool calls.
pub fn fits_code_id(provider_id: &str, model: &str) -> bool {
    FITS.lock().ok().and_then(|fits| fits.get(&known_key(provider_id, model)).copied()).unwrap_or(true)
}

/// Whether to send images to this model: what the provider's model list said, else a guess from
/// its name.
pub fn reads_images(provider_id: &str, model: &str) -> bool {
    let known = KNOWN.lock().ok().and_then(|known| known.get(&known_key(provider_id, model)).copied());
    match known {
        Some((vision, _, _)) => vision,
        None => traits_from_name(model).vision,
    }
}

/// True when the provider itself said this model cannot read images.
pub fn certainly_text_only(provider_id: &str, model: &str) -> bool {
    KNOWN
        .lock()
        .ok()
        .and_then(|known| known.get(&known_key(provider_id, model)).copied())
        .is_some_and(|(vision, from_provider, _)| !vision && from_provider)
}

/// Whether a listed model takes a tool list, when Neru has seen it in a model list.
pub fn supports_tools(provider_id: &str, model: &str) -> Option<bool> {
    KNOWN.lock().ok().and_then(|known| known.get(&known_key(provider_id, model)).map(|(_, _, tools)| *tools))
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
    "not deployed", "model access denied", "no endpoints found", "only supports interactions api", "no longer available to new users",
];

/// A test parameter Neru sent was refused; the model itself exists.
const PARAMETER_ISSUE: &[&str] = &[
    "max_tokens", "max_completion_tokens", "max_output_tokens", "unsupported parameter", "unsupported value", "temperature",
    "stream_options", "tool_choice", "unrecognized request argument", "unknown parameter", "extra inputs are not permitted",
];

const BAD_KEY: &[&str] = &[
    "invalid api key", "invalid_api_key", "incorrect api key", "invalid token", "authentication", "unauthenticated", "api key not valid", "api_key_invalid", "expired",
    "has been denied access",
];

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

pub const NO_FREE_QUOTA: &str = "No free quota for this model on this key (Google's free tier does not include it)";
const DAILY_LIMIT: &str = "Today's quota for this model is used up; it is checked again when the quota resets";
/// A billing or credit error: the key works, but the provider wants money for this model.
pub const NO_CREDIT: &str = "This key has no credit for this model (the provider bills it). Add credit, or pick a free model";
/// The key's subscription (a coding plan, an enterprise package) does not cover this model.
pub const NOT_IN_PLAN: &str = "This key's plan does not include this model";
const PLAN_LIMIT: &str = "This key's plan has used up its allowance for now; it is checked again after it resets";

/// The provider's own error code: `error.code` in a JSON body, or "(code 1113)" in a chat error
/// (`providers::response_error`).
fn error_code(body: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        match &value["error"]["code"] {
            Value::String(code) if !code.is_empty() => return Some(code.clone()),
            Value::Number(code) => return Some(code.to_string()),
            _ => {}
        }
    }
    let start = body.find("(code ")? + 6;
    body[start..].split(')').next().map(|code| code.trim().to_string()).filter(|code| !code.is_empty())
}

/// Z.AI's numeric error codes (https://docs.z.ai/api-reference/api-code, checked 2026-11-01). They come
/// with HTTP 429 for billing as well as for limits, so the code decides, not the status.
fn zai_verdict(code: &str, said: &str) -> Option<Verdict> {
    let code: u32 = code.parse().ok()?;
    Some(match code {
        1000..=1005 => Verdict::BadKey("The provider rejected the API key".into()),
        // 1113 "Insufficient balance or no resource package", 1309 "GLM Coding Plan package has expired".
        1113 | 1309 => Verdict::Unavailable(NO_CREDIT.into()),
        // 1211 "Unknown Model", 1221 "API has been taken offline", 1222 "API does not exist".
        1211 | 1221 | 1222 => Verdict::Unavailable(format!("Not served by this provider ({said})")),
        // 1220 no permission, 1311 "plan does not yet include access", 1314/1315 enterprise-only keys.
        1220 | 1311 | 1314 | 1315 => Verdict::Unavailable(NOT_IN_PLAN.into()),
        1302 => Verdict::RateLimited("Rate limited right now; try again in a minute".into()),
        // 1308 "Usage limit reached… will reset at", 1310 weekly or monthly limit, 1316-1321 plan windows
        // (their "insufficient balance for extra usage" variants are caught as credit errors first).
        1308 | 1310 | 1316..=1321 => Verdict::RateLimited(PLAN_LIMIT.into()),
        1261 => Verdict::Unknown("The provider said the test prompt is too long for this model".into()),
        1200 | 1230 | 1234 | 1305 => Verdict::Unknown("The provider is busy or had a problem; try again later".into()),
        _ => return None,
    })
}

/// A Gemini 429 about a free-tier quota whose limit is 0: the model is not on the free tier at all, so
/// waiting will not help ("…generate_content_free_tier_requests, limit: 0", `"quotaValue": "0"`).
fn no_free_quota(lower: &str) -> bool {
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    let zero = compact.contains("\"quotavalue\":\"0\"")
        || compact.match_indices("limit:0").any(|(at, marker)| !compact[at + marker.len()..].starts_with(|c: char| c.is_ascii_digit() || c == '.'));
    zero && (compact.contains("free_tier") || compact.contains("freetier"))
}

/// Google refused the key's Cloud project, not the model ("Consumer … has been denied access",
/// PERMISSION_DENIED for the project).
fn project_denied(lower: &str) -> bool {
    lower.contains("has been denied access") || (lower.contains("permission_denied") && lower.contains("project"))
}

/// Turns an HTTP answer to the test request into what it says about the model, for any provider.
#[cfg(test)]
pub fn classify_probe(status: u16, body: &str) -> Verdict {
    classify_probe_for("", status, body)
}

/// [`classify_probe`] with the provider's own error codes (Z.AI's) read as well. Billing and credit
/// errors (HTTP 402, "insufficient balance", "insufficient_quota", "requires a paid plan"…) make the
/// model unavailable on this key: waiting will not help, and they are not rate limits.
pub fn classify_probe_for(provider_id: &str, status: u16, body: &str) -> Verdict {
    let lower = body.to_ascii_lowercase();
    let said = excerpt(body);
    let said_or = |fallback: &str| if said.is_empty() { fallback.to_string() } else { said.clone() };
    let strong = STRONG_UNAVAILABLE.iter().any(|marker| lower.contains(marker))
        || (lower.contains("function") && lower.contains("not found"));
    let failed = !(200..=299).contains(&status) && status != 401;
    if failed && !no_free_quota(&lower) && crate::fallback::needs_credits(&format!("http {status} {lower}")) {
        return Verdict::Unavailable(NO_CREDIT.into());
    }
    if failed && crate::fallback::plan_excludes(&lower) {
        return Verdict::Unavailable(NOT_IN_PLAN.into());
    }
    if let Some(verdict) = error_code(body).filter(|_| failed && provider_id == "zai").and_then(|code| zai_verdict(&code, &said_or(&format!("HTTP {status}")))) {
        return verdict;
    }
    match status {
        200..=299 => Verdict::Ok("Answered a test request".into()),
        401 => Verdict::BadKey("The provider rejected the API key".into()),
        429 if no_free_quota(&lower) => Verdict::Unavailable(NO_FREE_QUOTA.into()),
        429 if crate::fallback::is_daily(&lower) || lower.contains("perday") => Verdict::RateLimited(DAILY_LIMIT.into()),
        429 => Verdict::RateLimited("Rate limited right now; try again in a minute".into()),
        403 if project_denied(&lower) => {
            Verdict::BadKey("Google denied this key's Cloud project access to the API. Check the project in Google AI Studio, or create a new key".into())
        }
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
#[cfg(test)]
pub fn classify_error_text(error: &str) -> Option<Verdict> {
    classify_error_text_for("", error)
}

fn classify_error_text_for(provider_id: &str, error: &str) -> Option<Verdict> {
    let lower = error.to_ascii_lowercase();
    let start = lower.find("http ")? + 5;
    let status: u16 = lower[start..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some(classify_probe_for(provider_id, status, error))
}

/// The provider refused the tool list itself ("tools are not supported", "No endpoints found that
/// support tool use").
fn tools_refused(lower: &str) -> bool {
    (lower.contains("tool") || lower.contains("function"))
        && ["not supported", "unsupported", "does not support", "not support", "not enabled", "not available", "that support tool"].iter().any(|marker| lower.contains(marker))
}

/// What a reply to the tool test says: "ok" when it holds a tool call, "no" when the provider refused
/// the tool or the model answered in text instead, "unknown" for limits, cut-off replies and unclear errors.
pub fn classify_tool_probe(status: u16, body: &str) -> &'static str {
    let lower = body.to_ascii_lowercase();
    match status {
        200..=299 => {
            let Ok(value) = serde_json::from_str::<Value>(body) else { return "unknown" };
            let message = &value["choices"][0]["message"];
            let called = message["tool_calls"].as_array().is_some_and(|calls| !calls.is_empty())
                || message["function_call"].is_object()
                || value["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_use"))
                || value["output"].as_array().is_some_and(|items| items.iter().any(|item| item["type"] == "function_call"));
            let cut = value["choices"][0]["finish_reason"] == "length" || value["stop_reason"] == "max_tokens" || value["status"] == "incomplete";
            if called {
                "ok"
            } else if cut {
                "unknown"
            } else {
                "no"
            }
        }
        // A refused `tool_choice` says nothing about tools themselves; the test is sent again without it.
        _ if lower.contains("tool_choice") => "unknown",
        400 | 404 | 405 | 415 | 422 | 500 if tools_refused(&lower) => "no",
        _ => "unknown",
    }
}

// ---------- cache of probe results ----------

const DAY: u64 = 24 * 3600;
/// "Not found" or "not supported" failures within a day that hide a model even when the provider's
/// wording was not clear enough to say so the first time.
const STRIKES: u32 = 3;
/// A model's tool test is stored next to its own verdict, under its id plus this suffix.
const TOOLS_SUFFIX: &str = "\u{0}tools";

/// How long a verdict lasts before the model is checked again: a week for a model that answered, two
/// for one the provider refused (they rarely come back sooner), a month for a tool test. Strikes last
/// a day.
fn ttl(status: &str) -> u64 {
    match status {
        "ok" => 7 * DAY,
        "unavailable" => 14 * DAY,
        "tools:ok" | "tools:no" => 30 * DAY,
        _ => DAY,
    }
}

/// When a verdict stored at `at` expires. Gemini's spent daily quotas, and models with no free quota
/// on the key, are checked again after Google's quota day turns over (midnight Pacific time, see
/// `limits::next_day`), since a free tier can change then.
fn expiry(provider_id: &str, status: &str, reason: &str, at: u64) -> u64 {
    if provider_id == "gemini" && (status == "limited" || reason == NO_FREE_QUOTA) {
        crate::limits::next_day("gemini", at)
    } else if reason == NO_CREDIT || reason == NOT_IN_PLAN {
        // Credit can be topped up and plans change: check again after a day, not two weeks.
        at + DAY
    } else {
        at + ttl(status)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Entry {
    status: String,
    reason: String,
    at: u64,
    /// When the verdict expires (Unix seconds). 0 in entries written before expiries were stored: those
    /// last [`ttl`] from `at`.
    #[serde(default)]
    until: u64,
}

impl Entry {
    fn expires(&self) -> u64 {
        if self.until > 0 { self.until } else { self.at + ttl(&self.status) }
    }
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
    let now = now();
    for entries in scopes.values_mut() {
        // Gateway 404s were once stored as "unavailable"; they said nothing about the model.
        entries.retain(|_, entry| entry.expires() > now && !entry.reason.contains("404 page not found"));
    }
    scopes.retain(|_, entries| !entries.is_empty());
    scopes
}

fn save_cache(scopes: &Scopes) {
    // Tests must not touch the user's data folder.
    if cfg!(test) {
        return;
    }
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

fn cached(config: &ProviderConfig, key: &str) -> Option<Entry> {
    let cache = CACHE.lock().ok()?;
    let entry = cache.get(&scope(config))?.get(key)?;
    (entry.expires() > now()).then(|| entry.clone())
}

/// A verdict that needs no new request yet: the model answered, was refused, or spent its Gemini day.
fn settled(entry: &Entry) -> bool {
    matches!(entry.status.as_str(), "ok" | "unavailable" | "limited")
}

/// The tool test's stored answer for a model: "ok" or "no".
fn cached_tools(config: &ProviderConfig, model: &str) -> Option<String> {
    cached(config, &format!("{model}{TOOLS_SUFFIX}")).and_then(|entry| entry.status.strip_prefix("tools:").map(str::to_string))
}

fn put(config: &ProviderConfig, key: &str, status: &str, reason: &str) {
    let at = now();
    let entry = Entry { status: status.into(), reason: reason.into(), at, until: expiry(&config.provider_id, status, reason, at) };
    if let Ok(mut cache) = CACHE.lock() {
        cache.entry(scope(config)).or_default().insert(key.to_string(), entry);
        save_cache(&cache);
    }
}

fn store(config: &ProviderConfig, model: &str, verdict: &Verdict) {
    let status = match verdict {
        Verdict::Ok(_) => "ok",
        Verdict::Unavailable(_) => "unavailable",
        // A spent Gemini day: checking again before midnight Pacific only spends more quota.
        Verdict::RateLimited(reason) if config.provider_id == "gemini" && reason == DAILY_LIMIT => "limited",
        _ => return,
    };
    put(config, model, status, verdict.reason());
}

/// Stores a tool test's answer; "unknown" is not stored, so the test runs again next time.
fn store_tools(config: &ProviderConfig, model: &str, tools: &str) {
    if matches!(tools, "ok" | "no") {
        put(config, &format!("{model}{TOOLS_SUFFIX}"), &format!("tools:{tools}"), "Tool test");
    }
}

/// True when a check or a request showed this model answers on this key.
pub fn verified_ok(config: &ProviderConfig, model: &str) -> bool {
    cached(config, model).is_some_and(|entry| entry.status == "ok")
}

/// True when a recent check found this model unavailable.
#[cfg(test)]
fn known_unavailable(config: &ProviderConfig, model: &str) -> bool {
    cached(config, model).is_some_and(|entry| entry.status == "unavailable")
}

/// Records a chat request that failed with "this model does not exist / is not supported". Clear answers
/// hide the model at once; vaguer "not found" or "unsupported" failures replace an "ok" verdict (so the
/// next listing checks the model again) and hide it after three in a day.
pub fn note_failure(config: &ProviderConfig, error: &str) {
    if let Some(verdict @ Verdict::Unavailable(_)) = classify_error_text_for(&config.provider_id, error) {
        store(config, &config.model, &verdict);
        return;
    }
    let lower = error.to_lowercase();
    let vague = ["not found", "not supported", "unsupported", "http 404", "does not support"].iter().any(|marker| lower.contains(marker));
    if !vague || crate::fallback::is_rate_limited(error) {
        return;
    }
    let Ok(mut cache) = CACHE.lock() else { return };
    let entries = cache.entry(scope(config)).or_default();
    let at = now();
    let strikes = entries
        .get(&config.model)
        .filter(|entry| entry.at + DAY > at)
        .and_then(|entry| entry.status.strip_prefix("strike:").and_then(|count| count.parse::<u32>().ok()))
        .unwrap_or(0)
        + 1;
    let entry = if strikes >= STRIKES {
        Entry { status: "unavailable".into(), reason: format!("Failed {STRIKES} times in a day: {}", excerpt(error)), at, until: at + ttl("unavailable") }
    } else {
        Entry { status: format!("strike:{strikes}"), reason: excerpt(error), at, until: at + DAY }
    };
    entries.insert(config.model.clone(), entry);
    save_cache(&cache);
}

fn apply_cache(config: &ProviderConfig, models: &mut [ModelInfo]) {
    for info in models {
        if let Some(entry) = cached(config, &info.id).filter(|entry| matches!(entry.status.as_str(), "ok" | "unavailable")) {
            // A Gemini model that answers on this key is free here, whatever the allowlist says.
            if entry.status == "ok" && config.provider_id == "gemini" {
                info.free = true;
            }
            info.verified = entry.status;
            info.verified_reason = Some(entry.reason);
        }
        if info.tools {
            if let Some(tools) = cached_tools(config, &info.id) {
                info.tools_verified = tools;
            }
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
    remember(&config.provider_id, &models);
    // Models without tool calling run on the text protocol (toolproto.rs); the switch ranks them lower.
    models.into_iter().filter(|info| info.verified != "unavailable").map(|info| info.id).collect()
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

/// Cloudflare Workers AI lists its models on the account (`/ai/models/search`), not on the
/// OpenAI-compatible path; text-generation models are turned into a `/models`-shaped list.
async fn cloudflare_models(client: &Client, config: &ProviderConfig) -> Option<Value> {
    let root = provider_root(&config.base_url, "/ai/v1")?;
    let request = providers::authorize(client.get(format!("{root}/ai/models/search?task=Text%20Generation&per_page=200")), config);
    let body = json_ok(request).await?;
    Some(cloudflare_list(&body))
}

fn cloudflare_list(body: &Value) -> Value {
    let data: Vec<Value> = body["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item["name"].as_str()?;
            let property = |id: &str| item["properties"].as_array().into_iter().flatten().find(|p| p["property_id"] == id).map(|p| p["value"].clone());
            let mut entry = json!({"id": name, "capabilities": {"function_calling": property("function_calling").is_some_and(|v| v == true || v == "true")}});
            if let Some(window) = property("context_window").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))) {
                entry["context_window"] = json!(window);
            }
            Some(entry)
        })
        .collect();
    json!({"data": data})
}

/// OpenRouter's list for this key: `/models/user` leaves out models the account's privacy settings,
/// provider preferences or guardrails block (the ones that fail with "No endpoints found"). A key on the
/// free tier cannot call paid models, so those are left out too; models without tool calling stay, on
/// the text protocol.
async fn openrouter_for_key(client: &Client, config: &ProviderConfig) -> Option<Vec<ModelInfo>> {
    let request = providers::authorize(client.get(format!("{}/models/user", config.base_url)), config);
    let body = json_ok(request).await.filter(|body| body["data"].is_array())?;
    let key = json_ok(providers::authorize(client.get(format!("{}/key", config.base_url)), config)).await;
    let free_tier = key.as_ref().is_some_and(|key| key["data"]["is_free_tier"].as_bool() == Some(true));
    crate::limits::record_windows(&body, &config.provider_id);
    let mut models = parse_models(&body, &config.provider_id).ok()?;
    models.retain(|info| !free_tier || info.free);
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
    if provider_id == "cloudflare" {
        if let Some(body) = cloudflare_models(client, config).await {
            crate::limits::record_windows(&body, provider_id);
            return parse_models(&body, provider_id);
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
        apply_nvidia_checks(&mut models, &merge_nvidia(remote_nvidia_list(client).await.as_ref()));
    }
    Ok(models)
}

/// NVIDIA's catalog lists far more models than a free key can call: most answer "Function not found
/// for account". Each model was checked with a free key (see `live_probe`); the ones that refused are
/// left out, the ones that answered start confirmed. Models NVIDIA adds later are checked in the
/// background the first time the list is loaded (`auto_check`).
fn apply_nvidia_checks(models: &mut Vec<ModelInfo>, lists: &NvidiaLists) {
    models.retain(|info| !lists.not_served.contains(&info.id));
    for info in models.iter_mut() {
        if lists.served.contains(&info.id) {
            info.verified = "ok".into();
            info.verified_reason = Some("Answered a test request with a free key".into());
        }
    }
}

/// When the built-in NVIDIA lists below were checked.
const NVIDIA_CHECKED: &str = "2026-09-30";

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

/// A hosted copy of the NVIDIA lists, so new checks reach users without an app update:
/// `{ "checked": "YYYY-MM-DD", "served": [...], "notServed": [...] }`. Empty until the file is hosted
/// somewhere; until then only the built-in lists are used.
const NVIDIA_LIST_URL: &str = "";

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct NvidiaList {
    checked: String,
    served: Vec<String>,
    not_served: Vec<String>,
}

#[derive(Default, Debug)]
struct NvidiaLists {
    served: HashSet<String>,
    not_served: HashSet<String>,
}

/// The built-in lists with a downloaded one on top: its new ids are added, and where the two disagree
/// the more recent check wins.
fn merge_nvidia(remote: Option<&NvidiaList>) -> NvidiaLists {
    let mut lists = NvidiaLists {
        served: NVIDIA_SERVED.iter().map(|id| id.to_string()).collect(),
        not_served: NVIDIA_NOT_SERVED.iter().map(|id| id.to_string()).collect(),
    };
    let Some(remote) = remote else { return lists };
    let newer = remote.checked.as_str() >= NVIDIA_CHECKED;
    for id in &remote.served {
        if newer || !lists.not_served.contains(id) {
            lists.not_served.remove(id);
            lists.served.insert(id.clone());
        }
    }
    for id in &remote.not_served {
        if newer || !lists.served.contains(id) {
            lists.served.remove(id);
            lists.not_served.insert(id.clone());
        }
    }
    lists
}

#[derive(Serialize, Deserialize)]
struct SavedNvidiaList {
    fetched: u64,
    list: NvidiaList,
}

/// The hosted NVIDIA list: the copy in the data folder while it is less than a day old, else a fresh
/// download (five seconds at most). Any failure falls back to the older copy, or to nothing.
async fn remote_nvidia_list(client: &Client) -> Option<NvidiaList> {
    if NVIDIA_LIST_URL.is_empty() || cfg!(test) {
        return None;
    }
    let path = data_dir().ok()?.join("nvidia-models.json");
    let saved: Option<SavedNvidiaList> = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok());
    if let Some(saved) = saved.as_ref().filter(|saved| now().saturating_sub(saved.fetched) < DAY) {
        return Some(saved.list.clone());
    }
    let response = client.get(NVIDIA_LIST_URL).timeout(Duration::from_secs(5)).send().await.ok().filter(|response| response.status().is_success());
    let fresh = match response {
        Some(response) => response.json::<NvidiaList>().await.ok(),
        None => None,
    };
    match fresh {
        Some(list) => {
            if let Ok(text) = serde_json::to_string(&SavedNvidiaList { fetched: now(), list: list.clone() }) {
                let _ = std::fs::write(&path, text);
            }
            Some(list)
        }
        None => saved.map(|saved| saved.list),
    }
}

/// Runs in the background, by key scope: one automatic check per key at a time.
static AUTO: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Providers whose free plan caps requests per day: their checks go to the strongest coding models first.
fn daily_capped(provider_id: &str) -> bool {
    matches!(provider_id, "gemini" | "groq" | "cerebras" | "mistral" | "sambanova")
}

/// Checks, in the background and within the free plan's pace, every listed model that has no fresh
/// verdict, then gives the models that answered the one-time tool test, so the picker only offers models
/// that work. Each run sends at most `probe_budget` requests, strongest coding models first; the rest are
/// checked the next time the list loads. Progress goes out as `models://check` events with the run id
/// `auto:<provider id>`.
fn auto_check(app: &AppHandle, config: &ProviderConfig, models: &[ModelInfo]) {
    if probe_skip(&config.provider_id).is_some() || (config.api_key.is_empty() && !providers::keyless(&config.provider_id)) {
        return;
    }
    let mut ranked: Vec<&ModelInfo> = models.iter().collect();
    if daily_capped(&config.provider_id) || config.provider_id == "nvidia" {
        ranked.sort_by_key(|info| std::cmp::Reverse(crate::fallback::coding_score(&info.id)));
    }
    let mut budget = probe_budget(&config.provider_id);
    let mut jobs = VecDeque::new();
    for info in ranked {
        let mut job = Job::new(config, &info.id, &config.api_format, info.tools);
        // NVIDIA's built-in list already confirmed these.
        if job.known.is_none() && info.verified == "ok" {
            job.known = Some(Entry { status: "ok".into(), reason: info.verified_reason.clone().unwrap_or_default(), at: now(), until: 0 });
        }
        let cost = job.cost(config);
        if cost == 0 || cost > budget {
            continue;
        }
        budget -= cost;
        jobs.push_back(job);
    }
    if jobs.is_empty() {
        return;
    }
    let key = scope(config);
    if !AUTO.lock().is_ok_and(|mut auto| auto.insert(key.clone())) {
        return;
    }
    let run_id = format!("auto:{}", config.provider_id);
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut runs) = RUNS.lock() {
        runs.insert(run_id.clone(), cancel.clone());
    }
    let total = jobs.len();
    let run = Arc::new(Run { app: app.clone(), run_id: run_id.clone(), template: config.clone(), total, done: Arc::new(AtomicUsize::new(0)), cancel });
    tauri::async_runtime::spawn(async move {
        let outcome = run_jobs(run.clone(), jobs).await;
        let note = outcome.stopped.then(|| STOPPED_NOTE.to_string());
        run.emit(None, true, note);
        RUNS.lock().ok().map(|mut runs| runs.remove(&run_id));
        AUTO.lock().ok().map(|mut auto| auto.remove(&key));
    });
}

pub(crate) fn resolve(provider_id: String, api_format: String, base_url: String, api_key: String, model: String, app: &AppHandle) -> Result<ProviderConfig, String> {
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

/// Classified chat models this key can list, best coding models first. Models without a fresh check
/// are checked in the background (`auto_check`).
#[tauri::command]
pub async fn list_models(provider_id: String, api_format: String, base_url: String, api_key: String, app: AppHandle) -> Result<Vec<ModelInfo>, String> {
    let config = resolve(provider_id, api_format, base_url, api_key, String::new(), &app)?;
    let mut models = fetch_list(&Client::new(), &config).await?;
    apply_cache(&config, &mut models);
    auto_check(&app, &config, &models);
    // Models this key's provider refused stay in the list, marked: the picker hides them behind "Show hidden".
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
        "builtin" => Some("Downloaded in Neru"),
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
        "ovhcloud" | "llm7" => 3_000,
        _ => 400,
    })
}

/// Most test requests one bulk check may send.
fn probe_budget(provider_id: &str) -> usize {
    match provider_id {
        "gemini" => 30,
        "groq" => 40,
        "cerebras" | "mistral" => 40,
        "sambanova" | "llm7" | "ovhcloud" => 20,
        "nvidia" => 150,
        _ => 120,
    }
}

/// The test request, sent once more after a short pause when the first failure was transient (see
/// [`probe_retryable`]), so a brief hiccup does not leave a working key "unknown".
async fn probe_once(client: &Client, config: &ProviderConfig) -> Verdict {
    if let Some(reason) = probe_skip(&config.provider_id) {
        return Verdict::Ok(reason.into());
    }
    let (mut verdict, retry) = probe_attempt(client, config).await;
    if retry {
        tokio::time::sleep(PROBE_RETRY_DELAY).await;
        verdict = probe_attempt(client, config).await.0;
    }
    verdict
}

const PROBE_RETRY_DELAY: Duration = Duration::from_millis(2_500);

/// One test request: its verdict, and whether the failure is worth one retry.
async fn probe_attempt(client: &Client, config: &ProviderConfig) -> (Verdict, bool) {
    let request = providers::probe_request(client, config).timeout(Duration::from_secs(30));
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            let verdict = classify_probe_for(&config.provider_id, status, &body);
            let retry = matches!(verdict, Verdict::Unknown(_)) && probe_retryable(Some(status), &body);
            (verdict, retry)
        }
        Err(error) if error.is_timeout() => (Verdict::Unknown("The provider did not answer in time".into()), true),
        Err(error) => (Verdict::Unknown(format!("Could not reach the provider: {}", error.to_string().chars().take(100).collect::<String>())), true),
    }
}

/// A failure that says nothing about the model and often clears within seconds: no answer (`None`),
/// a timeout, a server error or a gateway's bare 404.
fn probe_retryable(status: Option<u16>, body: &str) -> bool {
    match status {
        None => true,
        Some(408 | 425 | 500..=599) => true,
        Some(404) => body.to_ascii_lowercase().contains("404 page not found"),
        Some(_) => false,
    }
}

/// The tool test: one trivial tool and a prompt that asks for it. "ok", "no" or "unknown" (see
/// [`classify_tool_probe`]). A provider that refuses a forced tool call is asked again without forcing.
async fn tool_probe_once(client: &Client, config: &ProviderConfig) -> &'static str {
    for forced in [true, false] {
        let request = providers::tool_probe_request(client, config, forced).timeout(Duration::from_secs(45));
        let Ok(response) = request.send().await else { return "unknown" };
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        let result = classify_tool_probe(status, &body);
        if result != "unknown" || !forced || !body.to_ascii_lowercase().contains("tool_choice") {
            return result;
        }
    }
    "unknown"
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
    /// The tool test for a model that answered: "ok", "no" or "unknown". Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<String>,
}

fn result_of(model: &str, verdict: &Verdict, cached: bool) -> ProbeResult {
    ProbeResult {
        model: model.to_string(),
        status: verdict.status().into(),
        reason: verdict.reason().to_string(),
        rate_limited: matches!(verdict, Verdict::RateLimited(_)),
        cached,
        tools: None,
    }
}

/// A stored verdict as a check result. A spent Gemini day reads as a rate limit; strikes as unknown.
fn entry_result(model: &str, entry: Entry) -> ProbeResult {
    let status = if matches!(entry.status.as_str(), "ok" | "unavailable") { entry.status } else { "unknown".into() };
    let rate_limited = entry.reason == DAILY_LIMIT;
    ProbeResult { model: model.to_string(), status, reason: entry.reason, rate_limited, cached: true, tools: None }
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

/// Whether this key can call the configured model: a stored check when there is one (unless `force`),
/// else one test request. Used at startup, after signing in, and by doctor.
pub async fn health(config: &ProviderConfig, force: bool) -> ProbeResult {
    if !force {
        if let Some(result) = cached_health(config) {
            return result;
        }
    }
    let verdict = probe_once(&Client::new(), config).await;
    store(config, &config.model, &verdict);
    let mut result = result_of(&config.model, &verdict, false);
    if result.status == "ok" {
        result.tools = cached_tools(config, &config.model);
    }
    result
}

/// The stored check, without sending anything.
pub fn cached_health(config: &ProviderConfig) -> Option<ProbeResult> {
    let entry = cached(config, &config.model).filter(settled)?;
    let mut result = entry_result(&config.model, entry);
    if result.status == "ok" {
        result.tools = cached_tools(config, &config.model);
    }
    Some(result)
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CheckTarget {
    pub model: String,
    pub api_format: String,
    /// Whether the model takes a tool list; the tool test runs only then. Defaults to what the last
    /// listing said, else true.
    #[serde(default)]
    pub tools: Option<bool>,
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

const STOPPED_NOTE: &str = "Stopped: the provider is rate limiting or rejected the key. Try again in a few minutes.";

/// Stops a running availability check (also an automatic `auto:<provider id>` one) after its current
/// requests finish.
#[tauri::command]
pub fn cancel_check_models(run_id: String) {
    if let Ok(runs) = RUNS.lock() {
        if let Some(flag) = runs.get(&run_id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

/// One model in a check: its stored verdict when it has one, and whether it takes a tool list.
struct Job {
    model: String,
    api_format: String,
    takes_tools: bool,
    known: Option<Entry>,
}

impl Job {
    fn new(template: &ProviderConfig, model: &str, api_format: &str, takes_tools: bool) -> Self {
        Job { model: model.into(), api_format: api_format.into(), takes_tools, known: cached(template, model).filter(settled) }
    }

    /// The tool test is still due: the model takes tools, did not fail its own test, and has no tool verdict.
    fn needs_tools(&self, template: &ProviderConfig) -> bool {
        self.takes_tools && self.known.as_ref().is_none_or(|entry| entry.status == "ok") && cached_tools(template, &self.model).is_none()
    }

    /// Requests this job sends at most.
    fn cost(&self, template: &ProviderConfig) -> usize {
        usize::from(self.known.is_none()) + usize::from(self.needs_tools(template))
    }

    /// What the tool test says for this model now: "no" without tools, else the stored answer.
    fn tools(&self, template: &ProviderConfig) -> String {
        if !self.takes_tools {
            return "no".into();
        }
        cached_tools(template, &self.model).unwrap_or_else(|| "unknown".into())
    }
}

/// A check in progress: where its progress events go and what it has done.
struct Run {
    app: AppHandle,
    run_id: String,
    template: ProviderConfig,
    total: usize,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
}

impl Run {
    fn emit(&self, result: Option<ProbeResult>, finished: bool, note: Option<String>) {
        let done = if result.is_some() { self.done.fetch_add(1, Ordering::SeqCst) + 1 } else { self.done.load(Ordering::SeqCst) };
        let _ = self.app.emit("models://check", Progress { run_id: self.run_id.clone(), done, total: self.total, result, finished, note });
    }
}

#[derive(Default)]
struct Outcome {
    ok: usize,
    unavailable: usize,
    unknown: usize,
    stopped: bool,
}

/// Waits for the next free request slot, so the run stays at one request per `spacing`.
async fn take_slot(next_slot: &tokio::sync::Mutex<Instant>, spacing: Duration) {
    let mut slot = next_slot.lock().await;
    let wait = slot.saturating_duration_since(Instant::now());
    *slot = Instant::now().max(*slot) + spacing;
    drop(slot);
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

/// Works through `jobs` with four workers paced by `probe_spacing`, one progress event per model. Stops
/// on a rejected key or three rate limits in a row.
async fn run_jobs(run: Arc<Run>, jobs: VecDeque<Job>) -> Outcome {
    let queue = Arc::new(Mutex::new(jobs));
    let outcome = Arc::new(Mutex::new(Outcome::default()));
    let next_slot = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let limited = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    let spacing = probe_spacing(&run.template.provider_id);
    let mut workers = Vec::new();
    for _ in 0..4 {
        let (run, queue, outcome, next_slot, limited, stopped) = (run.clone(), queue.clone(), outcome.clone(), next_slot.clone(), limited.clone(), stopped.clone());
        workers.push(tauri::async_runtime::spawn(async move {
            let client = Client::new();
            loop {
                if run.cancel.load(Ordering::SeqCst) || stopped.load(Ordering::SeqCst) {
                    break;
                }
                let Some(job) = queue.lock().ok().and_then(|mut queue| queue.pop_front()) else { break };
                let config = ProviderConfig { api_format: job.api_format.clone(), model: job.model.clone(), ..run.template.clone() };
                let needs_tools = job.needs_tools(&run.template);
                let mut result = match job.known.clone() {
                    Some(entry) => entry_result(&job.model, entry),
                    None => {
                        let mut verdict;
                        let mut attempts = 0;
                        loop {
                            take_slot(&next_slot, spacing).await;
                            if run.cancel.load(Ordering::SeqCst) {
                                return;
                            }
                            verdict = probe_once(&client, &config).await;
                            attempts += 1;
                            let daily = matches!(&verdict, Verdict::RateLimited(reason) if reason == DAILY_LIMIT);
                            if matches!(verdict, Verdict::RateLimited(_)) && !daily && attempts < 2 {
                                tokio::time::sleep(Duration::from_secs(20)).await;
                                continue;
                            }
                            break;
                        }
                        // A spent daily quota is per model; it says nothing about the next one.
                        if matches!(&verdict, Verdict::RateLimited(reason) if reason != DAILY_LIMIT) {
                            if limited.fetch_add(1, Ordering::SeqCst) + 1 >= 3 {
                                stopped.store(true, Ordering::SeqCst);
                            }
                        } else {
                            limited.store(0, Ordering::SeqCst);
                        }
                        if matches!(verdict, Verdict::BadKey(_)) {
                            stopped.store(true, Ordering::SeqCst);
                        }
                        store(&config, &config.model, &verdict);
                        result_of(&config.model, &verdict, false)
                    }
                };
                if result.status == "ok" {
                    if needs_tools && !stopped.load(Ordering::SeqCst) && !run.cancel.load(Ordering::SeqCst) {
                        take_slot(&next_slot, spacing).await;
                        let tools = tool_probe_once(&client, &config).await;
                        store_tools(&config, &config.model, tools);
                        result.cached = false;
                    }
                    result.tools = Some(job.tools(&run.template));
                }
                if let Ok(mut outcome) = outcome.lock() {
                    match result.status.as_str() {
                        "ok" => outcome.ok += 1,
                        "unavailable" => outcome.unavailable += 1,
                        _ => outcome.unknown += 1,
                    }
                }
                run.emit(Some(result), false, None);
            }
        }));
    }
    for worker in workers {
        let _ = worker.await;
    }
    let mut outcome = outcome.lock().map(|mut outcome| std::mem::take(&mut *outcome)).unwrap_or_default();
    outcome.stopped = stopped.load(Ordering::SeqCst);
    outcome
}

/// Checks a list of models with small concurrency, one progress event (`models://check`) per model:
/// the one-token test, then the tool test for models that answered and take tools. The provider's
/// per-minute cap and a per-run request budget are respected; results are cached.
#[tauri::command]
pub async fn check_models(run_id: String, provider_id: String, base_url: String, api_key: String, targets: Vec<CheckTarget>, app: AppHandle) -> Result<CheckSummary, String> {
    let template = resolve(provider_id, "openai-chat".into(), base_url, api_key, String::new(), &app)?;
    if let Some(reason) = probe_skip(&template.provider_id) {
        return Ok(CheckSummary { checked: 0, total: 0, ok: 0, unavailable: 0, unknown: 0, cancelled: false, note: Some(reason.into()) });
    }
    let total = targets.len();
    let cancel = Arc::new(AtomicBool::new(false));
    RUNS.lock().map_err(|e| e.to_string())?.insert(run_id.clone(), cancel.clone());
    let run = Arc::new(Run { app, run_id: run_id.clone(), template: template.clone(), total, done: Arc::new(AtomicUsize::new(0)), cancel: cancel.clone() });

    // Models whose stored verdicts cover them need no request; report them straight away.
    let mut queue: VecDeque<Job> = VecDeque::new();
    let mut tally = Outcome::default();
    let mut budget = probe_budget(&template.provider_id);
    let mut over_budget = 0;
    for target in targets {
        let takes_tools = target.tools.or_else(|| supports_tools(&template.provider_id, &target.model)).unwrap_or(true);
        let job = Job::new(&template, &target.model, &target.api_format, takes_tools);
        let cost = job.cost(&template);
        if cost == 0 {
            let Some(entry) = job.known.clone() else { continue };
            let mut result = entry_result(&job.model, entry);
            if result.status == "ok" {
                result.tools = Some(job.tools(&template));
            }
            match result.status.as_str() {
                "ok" => tally.ok += 1,
                "unavailable" => tally.unavailable += 1,
                _ => tally.unknown += 1,
            }
            run.emit(Some(result), false, None);
        } else if cost <= budget {
            budget -= cost;
            queue.push_back(job);
        } else {
            over_budget += 1;
        }
    }
    let checked_budget = probe_budget(&template.provider_id);
    let outcome = run_jobs(run.clone(), queue).await;
    RUNS.lock().ok().map(|mut runs| runs.remove(&run_id));
    let (ok, unavailable, unknown) = (tally.ok + outcome.ok, tally.unavailable + outcome.unavailable, tally.unknown + outcome.unknown);
    let note = if outcome.stopped {
        Some(STOPPED_NOTE.to_string())
    } else if over_budget > 0 {
        Some(format!("Used this run's {checked_budget} test requests; {over_budget} more can be checked in another run (free plans have request limits)."))
    } else {
        None
    };
    run.emit(None, true, note.clone());
    let checked = run.done.load(Ordering::SeqCst);
    Ok(CheckSummary { checked, total, ok, unavailable, unknown, cancelled: cancel.load(Ordering::SeqCst), note })
}

// ---------- allowance ----------

/// What a key may spend, in plain words for the model picker.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Allowance {
    pub free_tier: bool,
    /// Credit left on the key (US dollars) when the key has a limit.
    pub remaining: Option<f64>,
    pub note: String,
}

/// Reads OpenRouter's `/key` answer. A free-tier account gets 50 requests a day on free models; one
/// that bought credits gets 1,000.
fn allowance_from(body: &Value) -> Option<Allowance> {
    let data = &body["data"];
    let free_tier = data["is_free_tier"].as_bool()?;
    let remaining = data["limit_remaining"].as_f64();
    let mut note = if free_tier { "Free tier · 50 requests a day".to_string() } else { "Paid account · 1,000 free-model requests a day".to_string() };
    if let Some(left) = remaining {
        note.push_str(&format!(" · ${left:.2} left on this key"));
    } else if let Some(used) = data["usage"].as_f64().filter(|used| *used > 0.0 && !free_tier) {
        note.push_str(&format!(" · ${used:.2} used"));
    }
    Some(Allowance { free_tier, remaining, note })
}

/// The key's plan and what is left on it, for providers that report it (OpenRouter's `/key`). None for
/// other providers, or when the provider does not answer.
#[tauri::command]
pub async fn provider_allowance(provider_id: String, base_url: String, api_key: String, app: AppHandle) -> Option<Allowance> {
    if provider_id != "openrouter" {
        return None;
    }
    let config = resolve(provider_id, "openai-chat".into(), base_url, api_key, String::new(), &app).ok()?;
    if config.api_key.is_empty() {
        return None;
    }
    let body = json_ok(providers::authorize(Client::new().get(format!("{}/key", config.base_url)), &config)).await?;
    allowance_from(&body)
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
        apply_nvidia_checks(&mut models, &merge_nvidia(None));
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
        // Gemini's agent previews answer only on another API (checked 2026-11-01).
        assert!(matches!(classify_probe(400, r#"{"error":{"message":"This model only supports Interactions API."}}"#), Verdict::Unavailable(_)));
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
        assert!(matches!(classify_probe(402, "{}"), Verdict::Unavailable(_)));
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
        assert_eq!(classify_probe(402, r#"{"error":{"message":"Insufficient balance"}}"#), Verdict::Unavailable(NO_CREDIT.into()));
    }

    #[test]
    fn billing_and_credit_errors_are_clear_verdicts_not_rate_limits() {
        let no_credit = Verdict::Unavailable(NO_CREDIT.into());
        assert_eq!(classify_probe(402, "{}"), no_credit, "HTTP 402 Payment Required");
        assert_eq!(classify_probe(402, r#"{"error":{"message":"This request requires more credits"}}"#), no_credit);
        // OpenAI's spent prepaid balance comes as a 429.
        let openai = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        assert_eq!(classify_probe(429, openai), no_credit);
        assert_eq!(classify_probe(400, r#"{"error":{"message":"Insufficient balance. Please top up."}}"#), no_credit);
        assert_eq!(classify_probe(403, r#"{"error":{"message":"You have no credits left on this account"}}"#), no_credit);
        assert_eq!(classify_probe(403, r#"{"error":{"message":"This model requires a paid plan"}}"#), no_credit);
        assert_eq!(classify_probe(403, r#"{"error":{"message":"This API method requires billing to be enabled"}}"#), no_credit);
        assert_eq!(classify_probe(403, r#"{"message":"Payment required to use this model"}"#), no_credit);
        // Gemini's per-minute 429 also says "billing details", but it is a rate limit.
        let gemini = r#"{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details. Quota exceeded for metric: generate_content_free_tier_requests, limit: 10","status":"RESOURCE_EXHAUSTED"}}"#;
        assert!(matches!(classify_probe(429, gemini), Verdict::RateLimited(_)));
        // The same reading for a failed chat request, and none of them count as a rate limit.
        let chat = "Provider HTTP 402 Payment Required: Insufficient credits";
        assert_eq!(classify_error_text(chat), Some(no_credit.clone()));
        assert!(!crate::fallback::is_rate_limited(chat) && crate::fallback::is_unavailable(chat));
    }

    #[test]
    fn zai_error_codes_get_plain_verdicts() {
        let body = |code: &str, message: &str| format!(r#"{{"error":{{"code":"{code}","message":"{message}"}}}}"#);
        let no_credit = Verdict::Unavailable(NO_CREDIT.into());
        // Z.AI sends its balance error as HTTP 429.
        assert_eq!(classify_probe_for("zai", 429, &body("1113", "Insufficient balance or no resource package. Please recharge.")), no_credit);
        assert_eq!(classify_probe(429, &body("1113", "Insufficient balance or no resource package. Please recharge.")), no_credit, "the wording alone is enough");
        assert_eq!(classify_probe_for("zai", 429, &body("1309", "Your GLM Coding Plan package has expired and is temporarily unavailable.")), no_credit);
        assert!(matches!(classify_probe_for("zai", 400, &body("1211", "Unknown Model, please check the model code.")), Verdict::Unavailable(reason) if reason.starts_with("Not served")));
        assert_eq!(classify_probe_for("zai", 429, &body("1311", "Your current subscription plan does not yet include access to GLM-5")), Verdict::Unavailable(NOT_IN_PLAN.into()));
        assert_eq!(classify_probe_for("zai", 429, &body("1308", "Usage limit reached for 5 hour. Your limit will reset at 2026-11-01 12:00:00")), Verdict::RateLimited(PLAN_LIMIT.into()));
        assert!(matches!(classify_probe_for("zai", 429, &body("1302", "Rate limit reached for requests")), Verdict::RateLimited(_)));
        assert!(matches!(classify_probe_for("zai", 400, &body("1261", "Prompt too long")), Verdict::Unknown(_)));
        assert!(matches!(classify_probe_for("zai", 429, &body("1305", "The service may be temporarily overloaded, please try again later")), Verdict::Unknown(_)));
        assert!(matches!(classify_probe_for("zai", 401, &body("1000", "Authentication Failed")), Verdict::BadKey(_)));
        // A chat error keeps the code (providers::response_error), and the stored verdict hides the model a day.
        assert_eq!(classify_error_text_for("zai", "Provider HTTP 429 Too Many Requests: Your GLM Coding Plan package has expired (code 1309)"), Some(no_credit));
        assert_eq!(expiry("zai", "unavailable", NO_CREDIT, 1_000), 1_000 + DAY);
        // Another provider's numeric codes are not read as Z.AI's.
        assert!(matches!(classify_probe_for("mistral", 400, &body("1308", "Bad request")), Verdict::Unknown(_)));
    }

    #[test]
    fn transient_probe_failures_are_retried_once() {
        assert!(probe_retryable(None, ""), "no answer or a timeout");
        assert!(probe_retryable(Some(503), "Service Unavailable"));
        assert!(probe_retryable(Some(502), "Bad Gateway"));
        assert!(probe_retryable(Some(404), "404 page not found"), "a gateway's bare 404");
        assert!(!probe_retryable(Some(404), r#"{"error":{"message":"model_not_found"}}"#));
        assert!(!probe_retryable(Some(429), "rate limited"), "rate limits are paced, not retried here");
        assert!(!probe_retryable(Some(402), "payment required"));
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
    fn cloudflare_lists_become_models_with_tools_and_windows() {
        let body = json!({"result":[
            {"name":"@cf/openai/gpt-oss-120b","properties":[{"property_id":"context_window","value":"128000"},{"property_id":"function_calling","value":"true"}]},
            {"name":"@cf/meta/llama-3.2-3b-instruct","properties":[{"property_id":"context_window","value":128000}]},
        ]});
        let models = parse_models(&cloudflare_list(&body), "cloudflare").unwrap();
        assert_eq!(models.len(), 2);
        assert!(models[0].tools && models[0].protocol == "native");
        assert_eq!(models[0].context_window, Some(128_000));
        assert!(!models[1].tools && models[1].protocol == "text", "no function calling: the text protocol");
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
        // A vague "not supported" hides a model only the third time in a day.
        let vague = ProviderConfig { model: "acme/flaky".into(), ..other_key.clone() };
        let mut flaky = vec![info_from_item(&json!({"id":"acme/flaky"}), "nvidia").unwrap()];
        for _ in 0..2 {
            note_failure(&vague, "Provider stream error: this endpoint is not supported for the request");
            apply_cache(&vague, &mut flaky);
            assert_eq!(flaky[0].verified, "unknown", "strikes are not verdicts");
        }
        note_failure(&vague, "Provider stream error: this endpoint is not supported for the request");
        apply_cache(&vague, &mut flaky);
        assert_eq!(flaky[0].verified, "unavailable");
        assert!(flaky[0].verified_reason.as_deref().unwrap().starts_with("Failed 3 times"));
        if let Ok(mut cache) = CACHE.lock() {
            cache.remove(&scope(&vague));
        }
        if let Ok(mut cache) = CACHE.lock() {
            cache.remove(&scope(&config));
            save_cache(&cache);
        }
    }

    #[test]
    fn gemini_free_tier_without_quota_is_unavailable_not_busy() {
        let limit_zero = r#"{"error":{"code":429,"message":"You exceeded your current quota. Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 0, model: gemini-3.1-pro","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"generativelanguage.googleapis.com/generate_content_free_tier_requests","quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier","quotaValue":"0"}]}]}}"#;
        assert_eq!(classify_probe(429, limit_zero), Verdict::Unavailable(NO_FREE_QUOTA.into()));
        // A free quota that is merely spent for today is a rate limit, re-checked after the reset.
        let spent = r#"{"error":{"code":429,"message":"Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 250, model: gemini-2.5-flash","details":[{"violations":[{"quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier","quotaValue":"250"}]}]}}"#;
        assert_eq!(classify_probe(429, spent), Verdict::RateLimited(DAILY_LIMIT.into()));
        assert!(matches!(classify_probe(429, r#"{"error":{"message":"limit: 0 requests per minute on paid tier"}}"#), Verdict::RateLimited(_)), "only free-tier quotas say so");
        assert!(matches!(classify_error_text(&format!("Provider HTTP 429 Too Many Requests: {limit_zero}")), Some(Verdict::Unavailable(_))));
    }

    #[test]
    fn a_denied_google_project_is_a_key_problem() {
        let denied = r#"{"error":{"code":403,"message":"Permission denied: Consumer 'api_key:AIza...' has been denied access.","status":"PERMISSION_DENIED"}}"#;
        let Verdict::BadKey(reason) = classify_probe(403, denied) else { panic!("a denied project is the key's problem") };
        assert!(reason.contains("Cloud project"), "{reason}");
        let project = r#"{"error":{"code":403,"message":"Generative Language API has not been used in project 123 before or it is disabled.","status":"PERMISSION_DENIED"}}"#;
        assert!(matches!(classify_probe(403, project), Verdict::BadKey(_)));
        assert!(!crate::fallback::is_forbidden(&format!("Provider HTTP 403 Forbidden: {denied}")), "switching models will not help");
    }

    #[test]
    fn tool_test_replies_are_read_per_format() {
        let openai = r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"get_time","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#;
        assert_eq!(classify_tool_probe(200, openai), "ok");
        let anthropic = r#"{"content":[{"type":"text","text":"Let me check."},{"type":"tool_use","id":"t1","name":"get_time","input":{}}],"stop_reason":"tool_use"}"#;
        assert_eq!(classify_tool_probe(200, anthropic), "ok");
        let responses = r#"{"status":"completed","output":[{"type":"function_call","name":"get_time","arguments":"{}"}]}"#;
        assert_eq!(classify_tool_probe(200, responses), "ok");
        // Answered in words instead of calling the tool.
        assert_eq!(classify_tool_probe(200, r#"{"choices":[{"message":{"content":"It is noon."},"finish_reason":"stop"}]}"#), "no");
        assert_eq!(classify_tool_probe(200, r#"{"content":[{"type":"text","text":"I cannot tell."}],"stop_reason":"end_turn"}"#), "no");
        // Cut off while thinking says nothing.
        assert_eq!(classify_tool_probe(200, r#"{"choices":[{"message":{"content":""},"finish_reason":"length"}]}"#), "unknown");
        // The provider refuses tools for this model.
        assert_eq!(classify_tool_probe(400, r#"{"error":{"message":"This model does not support tools."}}"#), "no");
        assert_eq!(classify_tool_probe(400, r#"{"error":{"message":"Function calling is not enabled for this model"}}"#), "no");
        assert_eq!(classify_tool_probe(404, r#"{"error":{"message":"No endpoints found that support tool use."}}"#), "no");
        // A refused tool_choice, limits and outages are inconclusive.
        assert_eq!(classify_tool_probe(400, r#"{"error":{"message":"tool_choice 'required' is not supported"}}"#), "unknown");
        assert_eq!(classify_tool_probe(429, r#"{"error":{"message":"Rate limit reached"}}"#), "unknown");
        assert_eq!(classify_tool_probe(503, "Service Unavailable"), "unknown");
    }

    #[test]
    fn verdicts_last_by_status() {
        assert_eq!(ttl("ok"), 7 * DAY);
        assert_eq!(ttl("unavailable"), 14 * DAY);
        assert_eq!(ttl("tools:no"), 30 * DAY);
        assert_eq!(ttl("strike:1"), DAY);
        let at = 1_768_478_400;
        assert_eq!(expiry("nvidia", "unavailable", "Not served", at), at + 14 * DAY);
        assert_eq!(expiry("gemini", "ok", "Answered a test request", at), at + 7 * DAY);
        // Old entries without an expiry last their status's lifetime.
        let old = Entry { status: "unavailable".into(), reason: String::new(), at, until: 0 };
        assert_eq!(old.expires(), at + 14 * DAY);
    }

    #[test]
    fn gemini_quota_verdicts_expire_at_midnight_pacific() {
        // 2026-01-15 12:00 UTC is 04:00 PST (UTC-8): next midnight Pacific is 2026-01-16 08:00 UTC.
        let winter = 1_768_478_400;
        assert_eq!(expiry("gemini", "unavailable", NO_FREE_QUOTA, winter), 1_768_550_400);
        assert_eq!(expiry("gemini", "limited", DAILY_LIMIT, winter), 1_768_550_400);
        // 2026-07-15 12:00 UTC is 05:00 PDT (UTC-7): next midnight Pacific is 2026-07-16 07:00 UTC.
        let summer = 1_784_116_800;
        assert_eq!(expiry("gemini", "limited", DAILY_LIMIT, summer), 1_784_185_200);
        // Only Gemini's quota day is Pacific.
        assert_eq!(expiry("groq", "unavailable", NO_FREE_QUOTA, winter), winter + 14 * DAY);
    }

    #[test]
    fn gemini_quota_and_tool_verdicts_are_stored() {
        let config = ProviderConfig {
            provider_id: "gemini".into(),
            api_format: "openai-chat".into(),
            base_url: "https://gemini-cache.example/v1beta/openai".into(),
            model: "gemini-3.1-pro-preview".into(),
            api_key: "k-gemini-cache-test".into(),
        };
        store(&config, "gemini-3.1-pro-preview", &Verdict::Unavailable(NO_FREE_QUOTA.into()));
        store(&config, "gemini-2.5-flash", &Verdict::RateLimited(DAILY_LIMIT.into()));
        store(&config, "gemini-2.5-pro", &Verdict::RateLimited("Rate limited right now; try again in a minute".into()));
        store(&config, "gemini-3.8-flash", &Verdict::Ok("Answered a test request".into()));
        store_tools(&config, "gemini-3.8-flash", "ok");
        store_tools(&config, "gemini-2.5-flash", "unknown");
        assert!(cached(&config, "gemini-3.1-pro-preview").is_some_and(|entry| entry.until <= now() + DAY + 3_600));
        assert_eq!(cached(&config, "gemini-2.5-flash").map(|entry| entry.status).as_deref(), Some("limited"));
        assert!(cached(&config, "gemini-2.5-pro").is_none(), "a per-minute limit is not stored");
        assert_eq!(cached_tools(&config, "gemini-3.8-flash").as_deref(), Some("ok"));
        assert!(cached_tools(&config, "gemini-2.5-flash").is_none());
        let mut models: Vec<ModelInfo> =
            ["gemini-3.8-flash", "gemini-3.1-pro-preview", "gemini-2.5-flash"].iter().filter_map(|id| info_from_item(&json!({"id": id}), "gemini")).collect();
        apply_cache(&config, &mut models);
        assert_eq!((models[0].verified.as_str(), models[0].tools_verified.as_str()), ("ok", "ok"));
        assert_eq!(models[1].verified, "unavailable");
        assert!(!models[1].free);
        assert_eq!(models[2].verified, "unknown", "a spent day is not a verdict on the model");
        let health = cached_health(&ProviderConfig { model: "gemini-2.5-flash".into(), ..config.clone() }).unwrap();
        assert!(health.status == "unknown" && health.rate_limited);
        if let Ok(mut cache) = CACHE.lock() {
            cache.remove(&scope(&config));
        }
    }

    #[test]
    fn gemini_free_allowlist_and_retired_models() {
        let body = json!({"object":"list","data":[
            {"id":"models/gemini-3.8-flash"},{"id":"models/gemini-3.1-pro-preview"},{"id":"models/gemini-2.5-flash-lite"},
            {"id":"models/gemini-2.5-pro"},{"id":"models/gemma-4-27b-it"},{"id":"models/gemini-4-argon-preview"},
            {"id":"models/gemini-2.0-flash"},{"id":"models/gemini-2.0-flash-lite-001"},{"id":"models/gemini-3-flash-preview"}
        ]});
        let parsed = parse_models(&body, "gemini").unwrap();
        assert!(!ids(&parsed).iter().any(|id| id.starts_with("gemini-2.0")), "Gemini 2.0 is shut down");
        for id in ["gemini-3.8-flash", "gemini-2.5-flash-lite", "gemini-2.5-pro", "gemma-4-27b-it", "gemini-3-flash-preview"] {
            assert!(find(&parsed, id).free, "{id} is on the free tier");
        }
        for id in ["gemini-3.1-pro-preview", "gemini-4-argon-preview"] {
            assert!(!find(&parsed, id).free, "{id} is paid only");
        }
        assert!(gemini_free("gemini-2.5-flash-preview-09-2025"));
        assert!(!gemini_free("gemini-2.5-flashy"));
        // Other providers keep their own pricing.
        assert!(!find(&parse_models(&json!({"data":[{"id":"gemini-3.8-flash"}]}), "opencode-like").unwrap(), "gemini-3.8-flash").free);
    }

    #[test]
    fn nvidia_lists_merge_with_a_hosted_copy() {
        let builtin = merge_nvidia(None);
        assert!(builtin.served.contains("z-ai/glm-5.3") && builtin.not_served.contains("moonshotai/kimi-k2.6"));
        let newer = NvidiaList {
            checked: "2026-10-30".into(),
            served: vec!["moonshotai/kimi-k2.6".into(), "acme/new-coder".into()],
            not_served: vec!["z-ai/glm-5.3".into()],
        };
        let merged = merge_nvidia(Some(&newer));
        assert!(merged.served.contains("moonshotai/kimi-k2.6") && !merged.not_served.contains("moonshotai/kimi-k2.6"));
        assert!(merged.not_served.contains("z-ai/glm-5.3") && !merged.served.contains("z-ai/glm-5.3"));
        assert!(merged.served.contains("acme/new-coder"));
        // An older copy only adds what the built-in lists do not know.
        let older = NvidiaList { checked: "2026-08-01".into(), served: vec!["moonshotai/kimi-k2.6".into(), "acme/old-coder".into()], not_served: vec!["acme/gone".into()] };
        let merged = merge_nvidia(Some(&older));
        assert!(merged.not_served.contains("moonshotai/kimi-k2.6") && !merged.served.contains("moonshotai/kimi-k2.6"));
        assert!(merged.served.contains("acme/old-coder") && merged.not_served.contains("acme/gone"));
        let parsed: NvidiaList = serde_json::from_str(r#"{"checked":"2026-11-01","served":["a/b"],"notServed":["c/d"]}"#).unwrap();
        assert_eq!((parsed.served, parsed.not_served), (vec!["a/b".to_string()], vec!["c/d".to_string()]));
    }

    #[test]
    fn code_mode_needs_tools_and_room() {
        let mut info = info_from_item(&json!({"id":"acme/coder","context_length":131072,"supported_parameters":["tools"]}), "openrouter").unwrap();
        assert_eq!(info.tools_verified, "unknown");
        assert!(fits_code(&info));
        info.tools_verified = "no".into();
        assert!(!fits_code(&info), "failed the tool test");
        let small = info_from_item(&json!({"id":"acme/tiny","context_length":8192,"supported_parameters":["tools"]}), "openrouter").unwrap();
        assert!(!fits_code(&small), "under 32K context");
        let unknown_window = info_from_item(&json!({"id":"acme/plain"}), "groq").unwrap();
        assert!(fits_code(&unknown_window));
        let no_tools = info_from_item(&json!({"id":"acme/text","supported_parameters":["temperature"]}), "openrouter").unwrap();
        assert_eq!(no_tools.tools_verified, "no", "a model without tools never takes the test");
        remember("test-fits", &[small, no_tools]);
        assert!(!fits_code_id("test-fits", "acme/tiny") && !fits_code_id("test-fits", "acme/text"));
        assert!(fits_code_id("test-fits", "never-listed"));
    }

    #[test]
    fn openrouter_key_info_becomes_a_plain_allowance() {
        let free = allowance_from(&json!({"data":{"is_free_tier":true,"usage":0,"limit_remaining":null}})).unwrap();
        assert_eq!(free, Allowance { free_tier: true, remaining: None, note: "Free tier · 50 requests a day".into() });
        let paid = allowance_from(&json!({"data":{"is_free_tier":false,"usage":1.5,"limit_remaining":8.5}})).unwrap();
        assert_eq!(paid.remaining, Some(8.5));
        assert_eq!(paid.note, "Paid account · 1,000 free-model requests a day · $8.50 left on this key");
        assert!(allowance_from(&json!({"error":"nope"})).is_none());
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
    /// Whether each saved key's provider accepts `stream_options.include_usage` on a streaming request, and
    /// whether usage then arrives. Prints provider, status and usage only, never a key.
    /// Run: cargo test --lib live_probe::stream_options -- --ignored --nocapture
    #[test]
    #[ignore]
    fn stream_options_saved_keys() {
        let mut current = ProviderConfig { provider_id: String::new(), api_format: String::new(), base_url: String::new(), api_key: String::new(), model: String::new() };
        let mut keys = HashMap::new();
        crate::settings::load(&mut current, &mut keys, &mut Default::default());
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let client = Client::new();
            for (id, key) in &keys {
                let Some((provider, base_url)) = id.split_once('\n') else { continue };
                let config = ProviderConfig { provider_id: provider.into(), api_format: "openai-chat".into(), base_url: base_url.into(), api_key: key.clone(), model: String::new() };
                let model = std::env::var("NERU_PROBE_MODEL").ok().or_else(|| (provider == current.provider_id).then(|| current.model.clone()).filter(|model| !model.is_empty()));
                let model = match model {
                    Some(model) => model,
                    None => fetch_list(&client, &config).await.ok().and_then(|list| list.into_iter().next()).map(|info| info.id).unwrap_or_default(),
                };
                let body = json!({"model": model.trim_start_matches("models/"), "messages":[{"role":"user","content":"Say hi"}], "max_tokens": 16, "stream": true, "stream_options": {"include_usage": true}});
                let request = providers::authorize(client.post(format!("{base_url}/chat/completions")).json(&body), &config);
                match request.send().await {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        let text = response.text().await.unwrap_or_default();
                        let usage = text.lines().any(|line| line.contains("\"usage\"") && !line.contains("\"usage\":null"));
                        eprintln!("{provider:<12} {model:<40} HTTP {status} usage={usage} {}", if status >= 400 { excerpt(&text) } else { String::new() });
                    }
                    Err(error) => eprintln!("{provider:<12} {model:<40} request failed: {}", error.to_string().chars().take(80).collect::<String>()),
                }
            }
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
