use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};

use crate::ProviderConfig;

pub const CHAT: &str = "openai-chat";
pub const RESPONSES: &str = "openai-responses";
pub const ANTHROPIC: &str = "anthropic";

pub fn valid_format(format: &str) -> bool {
    matches!(format, CHAT | RESPONSES | ANTHROPIC)
}

/// One client for every model request, so connections (and their TLS handshakes) are reused
/// across rounds and sessions. TCP keepalive stops routers and proxies from silently dropping
/// a connection while a model thinks for a long time before its first token.
pub fn http() -> Client {
    static CLIENT: std::sync::LazyLock<Client> = std::sync::LazyLock::new(|| {
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .tcp_keepalive(std::time::Duration::from_secs(15))
            .tcp_nodelay(true)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .build()
            .unwrap_or_default()
    });
    CLIENT.clone()
}

pub fn is_local(url: &reqwest::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

fn authorized(request: RequestBuilder, config: &ProviderConfig) -> RequestBuilder {
    if config.api_key.is_empty() {
        return request;
    }
    if config.api_format == ANTHROPIC && config.provider_id == "anthropic" {
        request.header("x-api-key", &config.api_key)
    } else {
        request.bearer_auth(&config.api_key)
    }
}

pub fn models_request(client: &Client, config: &ProviderConfig) -> RequestBuilder {
    let path = match config.provider_id.as_str() {
        "local" => "/models?available=true",
        "openrouter" => "/models?supported_parameters=tools",
        // Anthropic pages its list at 20 models.
        "anthropic" => "/models?limit=1000",
        _ => "/models",
    };
    let mut request = client.get(format!("{}{}", config.base_url, path));
    if config.api_format == ANTHROPIC {
        request = request.header("anthropic-version", "2023-06-01");
    }
    authorized(request, config)
}

/// A one-token request to a model, built like a real chat request (same URL, headers and protocol per
/// format) but without streaming or tools, to see whether the provider will serve the model.
pub fn probe_request(client: &Client, config: &ProviderConfig) -> RequestBuilder {
    let model = if config.provider_id == "gemini" {
        config.model.trim_start_matches("models/")
    } else {
        config.model.as_str()
    };
    let (path, payload) = match config.api_format.as_str() {
        ANTHROPIC => (
            "/messages",
            json!({"model":model,"max_tokens":1,"messages":[{"role":"user","content":"Hi"}]}),
        ),
        RESPONSES => (
            "/responses",
            json!({"model":model,"input":"Hi","max_output_tokens":16,"store":false}),
        ),
        _ => (
            "/chat/completions",
            json!({"model":model,"messages":[{"role":"user","content":"Hi"}],"max_tokens":8,"stream":false}),
        ),
    };
    post_json(client, config, path, &payload)
}

fn post_json(client: &Client, config: &ProviderConfig, path: &str, payload: &Value) -> RequestBuilder {
    let mut request = client.post(format!("{}{}", config.base_url, path)).json(payload);
    if config.api_format == ANTHROPIC {
        request = request.header("anthropic-version", "2023-06-01");
    }
    authorized(request, config)
}

/// Providers that serve requests without a key, at low rate limits. OVHcloud documents anonymous
/// access (2 requests a minute per IP and model, checked 2026-11-01); LLM7's docs now ask for a free
/// token, so it is no longer here.
pub fn keyless(provider_id: &str) -> bool {
    provider_id == "ovhcloud"
}

/// The tool test: a short request with one trivial tool (`get_time`, no arguments) whose answer should
/// be a call to it. `forced` asks for a tool call outright (`tool_choice`), which a few providers refuse.
pub fn tool_probe_request(client: &Client, config: &ProviderConfig, forced: bool) -> RequestBuilder {
    let model = if config.provider_id == "gemini" { config.model.trim_start_matches("models/") } else { config.model.as_str() };
    let prompt = "What time is it? Call the get_time tool.";
    let description = "Returns the current time.";
    let schema = json!({"type":"object","properties":{}});
    let (path, mut payload) = match config.api_format.as_str() {
        ANTHROPIC => (
            "/messages",
            json!({"model":model,"max_tokens":256,"messages":[{"role":"user","content":prompt}],"tools":[{"name":"get_time","description":description,"input_schema":schema}]}),
        ),
        RESPONSES => (
            "/responses",
            json!({"model":model,"input":prompt,"max_output_tokens":256,"store":false,"tools":[{"type":"function","name":"get_time","description":description,"parameters":schema}]}),
        ),
        _ => (
            "/chat/completions",
            json!({"model":model,"messages":[{"role":"user","content":prompt}],"max_tokens":256,"stream":false,"tools":[{"type":"function","function":{"name":"get_time","description":description,"parameters":schema}}]}),
        ),
    };
    if forced {
        payload["tool_choice"] = if config.api_format == ANTHROPIC { json!({"type":"any"}) } else { json!("required") };
    }
    post_json(client, config, path, &payload)
}

/// Replaces image parts with a note when the provider said the model cannot read images, so a
/// text-only model still answers instead of rejecting the whole request.
fn without_images_for_text_models(config: &ProviderConfig, messages: Vec<Value>) -> Vec<Value> {
    if !crate::models::certainly_text_only(&config.provider_id, &config.model) {
        return messages;
    }
    messages
        .into_iter()
        .map(|mut message| {
            if let Some(parts) = message["content"].as_array_mut() {
                for part in parts.iter_mut() {
                    if part["type"] == "image_url" {
                        *part = json!({"type":"text","text":"[An image was attached, but this model cannot read images.]"});
                    }
                }
            }
            message
        })
        .collect()
}

pub fn authorize(request: RequestBuilder, config: &ProviderConfig) -> RequestBuilder {
    authorized(request, config)
}

pub fn chat_request(
    client: &Client,
    config: &ProviderConfig,
    messages: &[Value],
    tools: &Value,
    effort: Option<&str>,
) -> RequestBuilder {
    // Every caller sends what it builds here, so this is where a task counts its requests.
    crate::taskstats::request(config);
    // The leading system message asked for its own cache breakpoint (a Chat project's block).
    let cache_break = messages
        .first()
        .filter(|message| message["role"] == "system" && message["_cache_break"] == true)
        .and_then(|message| message["content"].as_str())
        .map(str::to_string);
    let messages = without_images_for_text_models(config, public_messages(messages));
    // A model without native tool calls gets the tools in its prompt instead (toolproto.rs).
    let text_protocol = tools.as_array().is_some_and(|list| !list.is_empty()) && crate::toolproto::uses_text(config);
    let no_tools = json!([]);
    let (messages, tools) = if text_protocol {
        (crate::toolproto::to_text_protocol(&messages, tools.as_array().map_or(&[][..], Vec::as_slice)), &no_tools)
    } else {
        (messages, tools)
    };
    let session = session_key(&messages);
    if let Some(root) = ollama_root(config) {
        let payload = ollama_payload(config, &messages, tools);
        crate::fallback::note_sent(config);
        return client.post(format!("{root}/api/chat")).json(&payload);
    }
    let (path, mut payload) = match config.api_format.as_str() {
        ANTHROPIC => {
            let mut payload = anthropic_payload(&config.model, &messages, tools);
            if let Some(leading) = cache_break.as_deref().filter(|_| !text_protocol) {
                split_system_cache(&mut payload, leading);
            }
            ("/messages", payload)
        }
        RESPONSES => (
            "/responses",
            responses_payload(&config.model, &messages, tools),
        ),
        _ => ("/chat/completions", openai_chat_payload(config, &messages, tools)),
    };
    if text_protocol {
        if let Some(object) = payload.as_object_mut() {
            object.remove("tools");
            object.remove("tool_choice");
        }
    }
    if let Some(effort) = effort {
        apply_effort(&mut payload, config, effort);
    }
    apply_sticky_routing(&mut payload, config, &session);
    let mut request = client
        .post(format!("{}{}", config.base_url, path))
        .json(&payload);
    if config.api_format == ANTHROPIC {
        request = request.header("anthropic-version", "2023-06-01");
    }
    // Starts the clock for the model's latency (fallback.rs health).
    crate::fallback::note_sent(config);
    if config.provider_id == "openrouter" && !session.is_empty() {
        request = request.header("x-session-id", &session);
    }
    authorized(request, config)
}

/// A stable id for one conversation: its system prompt and first request, which stay byte-for-byte the
/// same for a session (agent.rs keeps them so). Empty for a conversation without either.
pub fn session_key(messages: &[Value]) -> String {
    use std::hash::{Hash, Hasher};
    let system = messages.iter().find(|message| message["role"] == "system").map(|message| message["content"].to_string());
    let first = messages.iter().find(|message| message["role"] == "user").map(|message| message["content"].to_string());
    if system.is_none() && first.is_none() {
        return String::new();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    system.hash(&mut hasher);
    first.hash(&mut hasher);
    format!("neru-{:016x}", hasher.finish())
}

/// Keeps a conversation on the same servers so their prompt cache stays warm: OpenRouter routes by
/// `session_id`, OpenAI (and hosts that mirror it) by `prompt_cache_key`.
fn apply_sticky_routing(payload: &mut Value, config: &ProviderConfig, session: &str) {
    if session.is_empty() || !payload.is_object() {
        return;
    }
    match config.provider_id.as_str() {
        "openrouter" => payload["session_id"] = json!(session),
        "openai" | "xai" | "deepseek" | "kimi" => payload["prompt_cache_key"] = json!(session),
        _ => {}
    }
}

// ---------- Ollama ----------

/// The Ollama server root for a model on Ollama, whose native API sets the context size and keeps the
/// model loaded (the OpenAI-compatible endpoint can do neither).
fn ollama_root(config: &ProviderConfig) -> Option<String> {
    if config.provider_id != "ollama" {
        return None;
    }
    config.base_url.trim_end_matches('/').strip_suffix("/v1").map(str::to_string)
}

/// Context size for an Ollama request: room for the prompt, the tools and a reply, in steps (8K, 16K,
/// 32K…) so it changes rarely (each change reloads the model), at most what the model supports.
pub fn ollama_num_ctx(model: &str, prompt_tokens: usize) -> usize {
    let wanted = (prompt_tokens + 4_096).max(8_192).next_power_of_two();
    let cap = crate::limits::window(model).unwrap_or(32_768).max(2_048);
    wanted.min(cap)
}

fn ollama_payload(config: &ProviderConfig, messages: &[Value], tools: &Value) -> Value {
    let converted: Vec<Value> = messages
        .iter()
        .map(|message| {
            let role = message["role"].as_str().unwrap_or("user");
            let mut text = Vec::new();
            let mut images = Vec::new();
            for (part, image) in content_parts(&message["content"]) {
                if let Some(part) = part {
                    text.push(part);
                } else if let Some((_, data)) = image.as_deref().and_then(data_url) {
                    images.push(data.to_string());
                }
            }
            let mut out = json!({"role": role, "content": text.join("\n")});
            if !images.is_empty() {
                out["images"] = json!(images);
            }
            if let Some(calls) = message["tool_calls"].as_array() {
                out["tool_calls"] = json!(calls
                    .iter()
                    .map(|call| {
                        let arguments = call["function"]["arguments"].as_str().and_then(|raw| serde_json::from_str::<Value>(raw).ok()).unwrap_or_else(|| json!({}));
                        json!({"function": {"name": call["function"]["name"], "arguments": arguments}})
                    })
                    .collect::<Vec<_>>());
            }
            out
        })
        .collect();
    let prompt = crate::usage::estimate_messages(messages) as usize + tools.to_string().len() / 4;
    let mut payload = json!({
        "model": config.model,
        "messages": converted,
        "stream": true,
        "keep_alive": "30m",
        "options": {"num_ctx": ollama_num_ctx(&config.model, prompt)},
    });
    if tools.as_array().is_some_and(|list| !list.is_empty()) {
        payload["tools"] = tools.clone();
    }
    payload
}

/// Whether the model takes a reasoning-effort setting without breaking multi-turn tool use.
/// Anthropic models are left out: extended thinking requires replaying signed thinking blocks
/// on every tool turn, which Neru does not keep yet.
pub fn supports_effort(config: &ProviderConfig) -> bool {
    let model = config.model.to_lowercase();
    let name = model.rsplit('/').next().unwrap_or(&model);
    let reasoning = ["o1", "o3", "o4", "gpt-5", "codex"]
        .iter()
        .any(|prefix| name.starts_with(prefix));
    match config.api_format.as_str() {
        ANTHROPIC => false,
        _ if config.provider_id == "openrouter" => !model.starts_with("anthropic/"),
        _ => reasoning,
    }
}

fn apply_effort(payload: &mut Value, config: &ProviderConfig, effort: &str) {
    if !matches!(effort, "low" | "medium" | "high") || !supports_effort(config) {
        return;
    }
    if config.api_format == RESPONSES || config.provider_id == "openrouter" {
        payload["reasoning"] = json!({ "effort": effort });
    } else {
        payload["reasoning_effort"] = json!(effort);
    }
}

/// Chat Completions body. Free hosts that reject OpenAI-only fields get a smaller payload.
fn openai_chat_payload(config: &ProviderConfig, messages: &[Value], tools: &Value) -> Value {
    let model = if config.provider_id == "gemini" {
        config.model.trim_start_matches("models/")
    } else {
        config.model.as_str()
    };
    let mut messages = messages.to_vec();
    if wants_cache_marks(config) {
        mark_cache_points(&mut messages);
    }
    let spare_usage = !accepts_stream_options(config);
    let mut payload = json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "tool_choice": "auto",
        "stream": true,
        "stream_options": {"include_usage": true}
    });
    // NVIDIA NIM and similar hosts default to ~1k output tokens, which cuts a file write off
    // mid-JSON. Ask for room to write whole files unless this model rejected the value before.
    if matches!(config.provider_id.as_str(), "nvidia" | "huggingface" | "modelscope") {
        let wanted = (context_window(&config.model) / 4).clamp(4_096, 16_384);
        let cap = output_cap(&config.model).unwrap_or(wanted);
        if cap > 0 && tools.as_array().is_some_and(|tools| !tools.is_empty()) {
            payload["max_tokens"] = json!(cap.min(wanted));
        }
    }
    if let Some(object) = payload.as_object_mut() {
        if spare_usage {
            object.remove("stream_options");
        }
        if config.provider_id == "gemini" {
            object.remove("parallel_tool_calls");
        }
    }
    payload
}

/// Providers that reject `stream_options`, learned from their errors at run time.
static NO_STREAM_OPTIONS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Whether to ask for token usage in the stream (`stream_options.include_usage`): it tells Neru the real
/// prompt and cached tokens. Hosts that reject the field, or answer usage without it (Groq sends
/// `x_groq.usage`), go without; a host that turns out to reject it is remembered.
fn accepts_stream_options(config: &ProviderConfig) -> bool {
    // Gemini takes it and streams usage (checked 2026-11-01); these hosts were seen rejecting it or
    // answer usage without it.
    let known_spare = matches!(config.provider_id.as_str(), "groq" | "cerebras" | "mistral" | "nvidia" | "huggingface" | "xai" | "modelscope");
    !known_spare && !NO_STREAM_OPTIONS.lock().is_ok_and(|list| list.contains(&config.provider_id))
}

/// Learns from an error that rejected `stream_options`. Returns true when the request should be retried
/// without it.
pub fn stream_options_rejected(config: &ProviderConfig, error: &str) -> bool {
    if !error.to_lowercase().contains("stream_options") || !accepts_stream_options(config) {
        return false;
    }
    if let Ok(mut list) = NO_STREAM_OPTIONS.lock() {
        list.push(config.provider_id.clone());
    }
    true
}

static OUTPUT_CAPS: std::sync::Mutex<Option<std::collections::HashMap<String, usize>>> = std::sync::Mutex::new(None);

fn output_cap(model: &str) -> Option<usize> {
    OUTPUT_CAPS.lock().ok()?.as_ref()?.get(model).copied()
}

/// Learns from an error that rejected `max_tokens`: keeps the limit it names, or drops the field
/// (stored as 0). Returns true when the request should be retried.
pub fn max_tokens_rejected(model: &str, error: &str) -> bool {
    let lower = error.to_lowercase();
    if !lower.contains("max_tokens") && !lower.contains("max tokens") && !lower.contains("max_completion_tokens") {
        return false;
    }
    // The provider's error code ("(code 1214)", response_error) is not a token count.
    let named = lower
        .split(" (code ")
        .next()
        .unwrap_or_default()
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse::<usize>().ok())
        .filter(|n| (256..=200_000).contains(n))
        .min();
    let Ok(mut caps) = OUTPUT_CAPS.lock() else { return false };
    let caps = caps.get_or_insert_with(Default::default);
    let next = match (caps.get(model).copied(), named) {
        (Some(0), _) => return false,
        (Some(current), Some(n)) if n < current => n,
        (Some(_), _) => 0,
        (None, Some(n)) => n,
        (None, None) => 0,
    };
    caps.insert(model.to_string(), next);
    true
}

fn public_messages(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            let mut copy = message.clone();
            if let Some(object) = copy.as_object_mut() {
                object.remove("_prompt_tokens");
                object.remove("_completion_tokens");
                object.remove("_cached_tokens");
                object.remove("_map");
                object.remove("_map_noted");
                object.remove("_cache_break");
            }
            copy
        })
        .collect()
}

/// Prompt tokens reported by a Chat Completions, Responses, or Anthropic payload.
pub fn prompt_tokens(body: &Value) -> Option<usize> {
    body["usage"]["prompt_tokens"]
        .as_u64()
        .or_else(|| body["usage"]["input_tokens"].as_u64())
        .map(|tokens| tokens as usize)
}

/// Context window for a model id: what the provider's model list reported, else a guess by family.
pub fn context_window(model: &str) -> usize {
    if let Some(window) = crate::limits::window(model) {
        return window;
    }
    let model = model.to_lowercase();
    let has = |part: &str| model.contains(part);
    if has("gemini") || has("gpt-4.1") || has("[1m]") {
        1_000_000
    } else if has("gpt-5") || has("codex") {
        400_000
    } else if has("claude") || has("o3") || has("o4") || has("o1") {
        200_000
    } else if has("qwen3-coder") || has("kimi") {
        256_000
    } else if has("deepseek") || has("gpt-4o") || has("llama") || has("mistral") {
        128_000
    } else {
        128_000
    }
}

/// Text and `data:` image parts of a message, in Chat Completions shape.
fn content_parts(content: &Value) -> Vec<(Option<String>, Option<String>)> {
    match content {
        Value::String(text) if !text.is_empty() => vec![(Some(text.clone()), None)],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str().map(|text| (Some(text.to_string()), None)),
                Some("image_url") => part["image_url"]["url"]
                    .as_str()
                    .map(|url| (None, Some(url.to_string()))),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// Splits `data:<mime>;base64,<data>` into its media type and payload.
fn data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn system_text(messages: &[Value]) -> String {
    messages
        .iter()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn responses_payload(model: &str, messages: &[Value], tools: &Value) -> Value {
    let mut input = Vec::new();
    for message in messages {
        match message["role"].as_str().unwrap_or("") {
            "user" | "assistant" => {
                let role = message["role"].as_str().unwrap();
                if let Some(content) = message["content"].as_str().filter(|text| !text.is_empty()) {
                    input.push(json!({"role":role,"content":content}));
                } else if message["content"].is_array() {
                    let parts: Vec<Value> = content_parts(&message["content"])
                        .into_iter()
                        .map(|(text, image)| match (text, image) {
                            (Some(text), _) => json!({"type": if role == "user" { "input_text" } else { "output_text" }, "text": text}),
                            (_, Some(url)) => json!({"type":"input_image","image_url":url}),
                            _ => Value::Null,
                        })
                        .filter(|part| !part.is_null())
                        .collect();
                    if !parts.is_empty() {
                        input.push(json!({"role":role,"content":parts}));
                    }
                }
                if let Some(calls) = message["tool_calls"].as_array() {
                    for call in calls {
                        input.push(json!({"type":"function_call","call_id":call["id"],"name":call["function"]["name"],"arguments":call["function"]["arguments"]}));
                    }
                }
            }
            "tool" => input.push(json!({"type":"function_call_output","call_id":message["tool_call_id"],"output":message["content"]})),
            _ => {}
        }
    }
    let response_tools: Vec<Value> = tools.as_array().into_iter().flatten().map(|tool| {
        json!({"type":"function","name":tool["function"]["name"],"description":tool["function"]["description"],"parameters":tool["function"]["parameters"]})
    }).collect();
    json!({"model":model,"instructions":system_text(messages),"input":input,"tools":response_tools,"tool_choice":"auto","parallel_tool_calls":false,"store":false,"stream":true})
}

fn push_anthropic(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if let Some(last) = messages.last_mut() {
        if last["role"] == role {
            if let Some(content) = last["content"].as_array_mut() {
                content.extend(blocks);
                return;
            }
        }
    }
    messages.push(json!({"role":role,"content":blocks}));
}

fn anthropic_payload(model: &str, messages: &[Value], tools: &Value) -> Value {
    let mut converted = Vec::new();
    for message in messages {
        match message["role"].as_str().unwrap_or("") {
            "user" | "assistant" => {
                let role = message["role"].as_str().unwrap();
                let mut blocks = Vec::new();
                for (text, image) in content_parts(&message["content"]) {
                    if let Some(text) = text {
                        blocks.push(json!({"type":"text","text":text}));
                    } else if let Some((mime, data)) = image.as_deref().and_then(data_url) {
                        blocks.push(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}));
                    }
                }
                if let Some(calls) = message["tool_calls"].as_array() {
                    for call in calls {
                        let input = call["function"]["arguments"]
                            .as_str()
                            .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                            .unwrap_or_else(|| json!({}));
                        blocks.push(json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],"input":input}));
                    }
                }
                if !blocks.is_empty() {
                    push_anthropic(&mut converted, role, blocks);
                }
            }
            "tool" => push_anthropic(
                &mut converted,
                "user",
                vec![
                    json!({"type":"tool_result","tool_use_id":message["tool_call_id"],"content":message["content"]}),
                ],
            ),
            _ => {}
        }
    }
    let mut anthropic_tools: Vec<Value> = tools.as_array().into_iter().flatten().map(|tool| {
        json!({"name":tool["function"]["name"],"description":tool["function"]["description"],"input_schema":tool["function"]["parameters"]})
    }).collect();
    // Whole files arrive as tool input, so leave room for them; older Claude 3 models cap lower.
    let max_tokens = if model.contains("claude-3-5") { 8_192 } else if model.contains("claude-3-") { 4_096 } else { 16_000 };
    let max_tokens = output_cap(model).filter(|cap| *cap > 0).map_or(max_tokens, |cap| cap.min(max_tokens));
    // Prompt caching, as Claude Code does it: the tools, the system prompt and the conversation
    // so far are cached, so each agent round only pays (in time and tokens) for what is new.
    if let Some(last) = anthropic_tools.last_mut() {
        last["cache_control"] = ephemeral();
    }
    if let Some(block) = converted.last_mut().and_then(|message| message["content"].as_array_mut()).and_then(|blocks| blocks.last_mut()) {
        block["cache_control"] = ephemeral();
    }
    let system = system_text(messages);
    let mut payload = json!({"model":model,"max_tokens":max_tokens,"messages":converted,"tools":anthropic_tools,"tool_choice":{"type":"auto"},"stream":true});
    if !system.is_empty() {
        payload["system"] = json!([{"type":"text","text":system,"cache_control":ephemeral()}]);
    }
    payload
}

/// Gives the leading system message its own cache breakpoint: Anthropic's system becomes the stable
/// leading block (base prompt, profile, Chat project) and the rest (the mode line, notes), each marked.
/// With the tools and the newest message that makes four breakpoints, Anthropic's maximum.
/// OpenRouter's Claude route needs nothing: each system message stays its own message there and
/// `mark_cache_points` already marks the leading one.
fn split_system_cache(payload: &mut Value, leading: &str) {
    let Some(text) = payload["system"][0]["text"].as_str().map(str::to_string) else { return };
    let Some(rest) = text.strip_prefix(leading).map(|rest| rest.trim_start_matches("\n\n").to_string()) else { return };
    if rest.trim().is_empty() || leading.trim().is_empty() {
        return;
    }
    payload["system"] = json!([
        {"type":"text","text":leading,"cache_control":ephemeral()},
        {"type":"text","text":rest,"cache_control":ephemeral()},
    ]);
}

fn ephemeral() -> Value {
    json!({"type":"ephemeral"})
}

/// Claude models reached through OpenRouter cache only what is marked, like the native API, and so do
/// Qwen models on Alibaba's DashScope (explicit cache, much cheaper than its implicit one). Other models
/// cache a repeated prefix on their own, which works because Neru only ever appends to the conversation.
fn wants_cache_marks(config: &ProviderConfig) -> bool {
    (config.provider_id == "openrouter" && config.model.starts_with("anthropic/"))
        || config.provider_id == "qwen"
        || config.base_url.contains("dashscope")
}

/// Marks the system prompt and the newest user or tool message as cache points.
fn mark_cache_points(messages: &mut [Value]) {
    let as_parts = |message: &mut Value| {
        if let Some(text) = message["content"].as_str().map(str::to_string) {
            message["content"] = json!([{"type":"text","text":text}]);
        }
        if let Some(last) = message["content"].as_array_mut().and_then(|parts| parts.last_mut()) {
            if last["type"] == "text" {
                last["cache_control"] = ephemeral();
            }
        }
    };
    if let Some(system) = messages.iter_mut().find(|message| message["role"] == "system") {
        as_parts(system);
    }
    if let Some(latest) = messages.iter_mut().rev().find(|message| message["role"] == "user" || message["role"] == "tool") {
        as_parts(latest);
    }
}

pub fn normalize_message(format: &str, body: &Value) -> Result<Value, String> {
    match format {
        ANTHROPIC => {
            let blocks = body["content"]
                .as_array()
                .ok_or("Provider returned no content")?;
            let mut text = Vec::new();
            let mut calls = Vec::new();
            for block in blocks {
                match block["type"].as_str().unwrap_or("") {
                    "text" => { if let Some(value) = block["text"].as_str() { text.push(value); } }
                    "tool_use" => calls.push(json!({"id":block["id"],"type":"function","function":{"name":block["name"],"arguments":block["input"].to_string()}})),
                    _ => {}
                }
            }
            let mut message = json!({"role":"assistant","content":text.join("\n")});
            if !calls.is_empty() {
                message["tool_calls"] = json!(calls);
            }
            Ok(message)
        }
        RESPONSES => {
            let output = body["output"]
                .as_array()
                .ok_or("Provider returned no output")?;
            let mut text = Vec::new();
            let mut calls = Vec::new();
            for item in output {
                match item["type"].as_str().unwrap_or("") {
                    "message" => {
                        if let Some(parts) = item["content"].as_array() {
                            for part in parts {
                                if let Some(value) = part["text"].as_str() { text.push(value); }
                            }
                        }
                    }
                    "function_call" => calls.push(json!({"id":item["call_id"],"type":"function","function":{"name":item["name"],"arguments":item["arguments"]}})),
                    _ => {}
                }
            }
            let mut message = json!({"role":"assistant","content":text.join("\n")});
            if !calls.is_empty() {
                message["tool_calls"] = json!(calls);
            }
            Ok(message)
        }
        _ => {
            let message = body["choices"][0]["message"].clone();
            if message.is_object() {
                Ok(message)
            } else {
                Err("Provider returned no assistant message".into())
            }
        }
    }
}

/// "Provider HTTP 429 Too Many Requests: <message> (code 1113)". The provider's own error code is kept
/// when it sends one: Z.AI's numeric codes and OpenAI's `insufficient_quota` say more than the status.
pub fn response_error(status: reqwest::StatusCode, body: &Value) -> String {
    let message = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or("request failed");
    let code = match &body["error"]["code"] {
        Value::String(code) if !code.is_empty() && !message.contains(code.as_str()) => format!(" (code {code})"),
        Value::Number(code) => format!(" (code {code})"),
        _ => String::new(),
    };
    format!(
        "Provider HTTP {status}: {}{code}",
        message.chars().take(300).collect::<String>()
    )
}

pub async fn read_response(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    let body: Value = serde_json::from_str(&text).map_err(|_| {
        if status.is_success() {
            "Provider returned invalid JSON".to_string()
        } else {
            format!(
                "Provider HTTP {status}: {}",
                text.chars().take(300).collect::<String>()
            )
        }
    })?;
    if !status.is_success() {
        return Err(response_error(status, &body));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange() -> Vec<Value> {
        vec![
            json!({"role":"system","content":"Be careful"}),
            json!({"role":"user","content":"Inspect the project"}),
            json!({"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"list_directory","arguments":"{\"path\":\"\"}"}}]}),
            json!({"role":"tool","tool_call_id":"call_1","content":"src/"}),
        ]
    }

    #[test]
    fn project_block_gets_its_own_breakpoint_within_four() {
        let leading = "You are Neru.\n\n# Project: Thesis\n<project_knowledge>\nbody\n</project_knowledge>";
        let mut messages = vec![
            json!({"role":"system","content":leading,"_cache_break":true}),
            json!({"role":"system","content":"Current mode: Chat."}),
            json!({"role":"user","content":"Hello"}),
        ];
        let public = public_messages(&messages);
        assert!(public[0].get("_cache_break").is_none(), "private fields never reach a provider");
        let tools = json!([{"type":"function","function":{"name":"search_project_knowledge","description":"d","parameters":{"type":"object"}}}]);
        let mut payload = anthropic_payload("claude-test", &public, &tools);
        split_system_cache(&mut payload, leading);
        assert_eq!(payload["system"][0]["text"], leading);
        assert_eq!(payload["system"][1]["text"], "Current mode: Chat.");
        assert_eq!(payload["system"][0]["cache_control"]["type"], "ephemeral");
        let marks = payload.to_string().matches("cache_control").count();
        assert!(marks <= 4, "Anthropic allows at most four breakpoints, got {marks}");
        // The same project block on a later turn gives the same leading bytes.
        messages.push(json!({"role":"assistant","content":"Hi"}));
        messages.push(json!({"role":"user","content":"More"}));
        let mut later = anthropic_payload("claude-test", &public_messages(&messages), &tools);
        split_system_cache(&mut later, leading);
        assert_eq!(later["system"][0], payload["system"][0]);
    }

    #[test]
    fn anthropic_keeps_tool_use_and_result_together() {
        let body = anthropic_payload("claude-test", &exchange(), &json!([]));
        assert_eq!(body["system"][0]["text"], "Be careful");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][2]["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call_1");
    }

    #[test]
    fn responses_keeps_function_call_ids() {
        let body = responses_payload("gpt-test", &exchange(), &json!([]));
        assert_eq!(body["input"][1]["type"], "function_call");
        assert_eq!(body["input"][2]["call_id"], "call_1");
    }

    #[test]
    fn normalizes_native_tool_calls() {
        let anthropic = json!({"content":[{"type":"tool_use","id":"tool_1","name":"read_file","input":{"path":"a.rs"}}]});
        let responses = json!({"output":[{"type":"function_call","call_id":"call_2","name":"read_file","arguments":"{\"path\":\"a.rs\"}"}]});
        assert_eq!(
            normalize_message(ANTHROPIC, &anthropic).unwrap()["tool_calls"][0]["id"],
            "tool_1"
        );
        assert_eq!(
            normalize_message(RESPONSES, &responses).unwrap()["tool_calls"][0]["id"],
            "call_2"
        );
    }

    #[test]
    fn uses_the_right_endpoint_and_authentication() {
        let client = Client::new();
        let mut config = ProviderConfig {
            provider_id: "anthropic".into(),
            api_format: ANTHROPIC.into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key: "example".into(),
            model: "example".into(),
        };
        let request = chat_request(&client, &config, &exchange(), &json!([]), None)
            .build()
            .unwrap();
        assert_eq!(request.url().path(), "/v1/messages");
        assert_eq!(request.headers()["x-api-key"], "example");
        assert_eq!(request.headers()["anthropic-version"], "2023-06-01");
        config.provider_id = "opencode".into();
        let request = chat_request(&client, &config, &exchange(), &json!([]), None)
            .build()
            .unwrap();
        assert_eq!(request.headers()["authorization"], "Bearer example");
        config.api_format = RESPONSES.into();
        let request = chat_request(&client, &config, &exchange(), &json!([]), None)
            .build()
            .unwrap();
        assert_eq!(request.url().path(), "/v1/responses");
    }

    #[test]
    fn images_convert_for_every_format() {
        let messages = vec![
            json!({"role":"system","content":"Be careful"}),
            json!({"role":"user","content":[{"type":"text","text":"What is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}),
        ];
        let anthropic = anthropic_payload("claude-test", &messages, &json!([]));
        assert_eq!(anthropic["messages"][0]["content"][1]["source"]["media_type"], "image/png");
        assert_eq!(anthropic["messages"][0]["content"][1]["source"]["data"], "AAAA");
        let responses = responses_payload("gpt-test", &messages, &json!([]));
        assert_eq!(responses["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(responses["input"][0]["content"][1]["image_url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn effort_only_reaches_models_that_take_it() {
        let mut config = ProviderConfig {
            provider_id: "openai".into(),
            api_format: "openai-chat".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-5.2".into(),
            api_key: "k".into(),
        };
        let mut payload = json!({});
        apply_effort(&mut payload, &config, "high");
        assert_eq!(payload["reasoning_effort"], "high");
        config.model = "gpt-4o".into();
        let mut payload = json!({});
        apply_effort(&mut payload, &config, "high");
        assert!(payload.get("reasoning_effort").is_none());
        config.api_format = "anthropic".into();
        config.model = "claude-opus-5".into();
        assert!(!supports_effort(&config));
        assert_eq!(context_window("anthropic/claude-sonnet-5.5"), 200_000);
    }

    #[test]
    fn free_hosts_omit_fields_they_reject() {
        let config = ProviderConfig {
            provider_id: "gemini".into(),
            api_format: CHAT.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
            model: "models/gemini-2.5-flash".into(),
            api_key: "k".into(),
        };
        let payload = openai_chat_payload(&config, &exchange(), &json!([]));
        assert_eq!(payload["model"], "gemini-2.5-flash");
        assert_eq!(payload["stream_options"]["include_usage"], true, "Gemini streams usage when asked");
        assert!(payload.get("parallel_tool_calls").is_none());
        assert_eq!(payload["tools"], json!([]));
    }

    #[test]
    fn probe_requests_use_the_same_endpoints_as_chat_without_streaming() {
        let client = Client::new();
        let mut config = ProviderConfig {
            provider_id: "nvidia".into(),
            api_format: CHAT.into(),
            base_url: "https://integrate.api.nvidia.com/v1".into(),
            model: "meta/llama-3.1-70b-instruct".into(),
            api_key: "k".into(),
        };
        let request = probe_request(&client, &config).build().unwrap();
        assert_eq!(request.url().path(), "/v1/chat/completions");
        assert_eq!(request.headers()["authorization"], "Bearer k");
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 8);
        assert!(body.get("tools").is_none());
        config.provider_id = "gemini".into();
        config.model = "models/gemini-2.5-flash".into();
        let request = probe_request(&client, &config).build().unwrap();
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["model"], "gemini-2.5-flash");
        config.provider_id = "anthropic".into();
        config.api_format = ANTHROPIC.into();
        config.base_url = "https://api.anthropic.com/v1".into();
        let request = probe_request(&client, &config).build().unwrap();
        assert_eq!(request.url().path(), "/v1/messages");
        assert_eq!(request.headers()["x-api-key"], "k");
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 1);
        config.provider_id = "openai".into();
        config.api_format = RESPONSES.into();
        config.base_url = "https://api.openai.com/v1".into();
        let request = probe_request(&client, &config).build().unwrap();
        assert_eq!(request.url().path(), "/v1/responses");
    }

    #[test]
    fn sessions_stick_to_one_route_for_prompt_caching() {
        let client = Client::new();
        let mut config = ProviderConfig { provider_id: "openrouter".into(), api_format: CHAT.into(), base_url: "https://openrouter.ai/api/v1".into(), model: "qwen/qwen3-coder:free".into(), api_key: "k".into() };
        let request = chat_request(&client, &config, &exchange(), &json!([]), None).build().unwrap();
        let session = request.headers()["x-session-id"].to_str().unwrap().to_string();
        assert!(session.starts_with("neru-"));
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["session_id"], session);
        // Later rounds of the same conversation keep the id.
        let mut longer = exchange();
        longer.push(json!({"role":"assistant","content":"Done"}));
        assert_eq!(session_key(&longer), session);
        config.provider_id = "openai".into();
        config.base_url = "https://api.openai.com/v1".into();
        let request = chat_request(&client, &config, &exchange(), &json!([]), None).build().unwrap();
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["prompt_cache_key"], session);
        assert!(request.headers().get("x-session-id").is_none());
        // DashScope caches only marked prefixes.
        let qwen = ProviderConfig { provider_id: "qwen".into(), base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1".into(), model: "qwen3-coder-plus".into(), ..config.clone() };
        let payload = openai_chat_payload(&qwen, &exchange(), &json!([]));
        assert_eq!(payload["messages"][0]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn text_protocol_models_get_tools_in_the_prompt() {
        let config = ProviderConfig { provider_id: "test-text-tools".into(), api_format: CHAT.into(), base_url: "https://x.example/v1".into(), model: "no-tools-model".into(), api_key: "k".into() };
        let list = crate::models::parse_models(&json!({"data":[{"id":"no-tools-model","supported_parameters":["temperature"]}]}), "test-text-tools").unwrap();
        crate::models::remember("test-text-tools", &list);
        let tools: Value = serde_json::from_str(crate::tools::READ_TOOLS).unwrap();
        let request = chat_request(&Client::new(), &config, &exchange(), &tools, None).build().unwrap();
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert!(body.get("tools").is_none() && body.get("tool_choice").is_none());
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("<tool_call>"));
        assert!(body["messages"].as_array().unwrap().iter().all(|message| message["role"] != "tool"));
    }

    #[test]
    fn ollama_uses_its_native_api_with_a_sized_context() {
        let config = ProviderConfig { provider_id: "ollama".into(), api_format: CHAT.into(), base_url: "http://localhost:11434/v1".into(), model: "test-ollama:7b".into(), api_key: String::new() };
        let messages = vec![json!({"role":"user","content":[{"type":"text","text":"What is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}), exchange()[2].clone()];
        let request = chat_request(&Client::new(), &config, &messages, &json!([]), None).build().unwrap();
        assert_eq!(request.url().as_str(), "http://localhost:11434/api/chat");
        let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["keep_alive"], "30m");
        assert_eq!(body["options"]["num_ctx"], 8_192);
        assert_eq!(body["messages"][0]["images"][0], "AAAA");
        assert_eq!(body["messages"][1]["tool_calls"][0]["function"]["arguments"]["path"], "");
        assert!(body.get("tools").is_none());
        crate::limits::record_windows(&json!({"data":[{"id":"test-ollama:7b","context_length":40_960}]}), "ollama");
        assert_eq!(ollama_num_ctx("test-ollama:7b", 30_000), 40_960, "capped by what the model supports");
        assert_eq!(ollama_num_ctx("test-ollama:7b", 9_000), 16_384, "sized in steps so the model is not reloaded every round");
    }

    #[test]
    fn hosts_that_reject_stream_options_are_learned() {
        let config = ProviderConfig { provider_id: "test-stream-options".into(), api_format: CHAT.into(), base_url: "https://x.example/v1".into(), model: "m".into(), api_key: "k".into() };
        assert_eq!(openai_chat_payload(&config, &exchange(), &json!([]))["stream_options"]["include_usage"], true);
        assert!(!stream_options_rejected(&config, "Provider HTTP 400: max_tokens too large"));
        assert!(stream_options_rejected(&config, "Provider HTTP 400: Unrecognized request argument supplied: stream_options"));
        assert!(openai_chat_payload(&config, &exchange(), &json!([])).get("stream_options").is_none());
        assert!(!stream_options_rejected(&config, "stream_options again"), "retried once only");
    }

    #[test]
    fn anthropic_model_list_asks_for_every_page() {
        let config = ProviderConfig {
            provider_id: "anthropic".into(),
            api_format: ANTHROPIC.into(),
            base_url: "https://api.anthropic.com/v1".into(),
            model: String::new(),
            api_key: "k".into(),
        };
        let request = models_request(&Client::new(), &config).build().unwrap();
        assert_eq!(request.url().query(), Some("limit=1000"));
    }

    #[test]
    fn images_are_replaced_only_when_the_provider_says_the_model_is_text_only() {
        let config = ProviderConfig {
            provider_id: "test-text-only".into(),
            api_format: CHAT.into(),
            base_url: "https://x.example/v1".into(),
            model: "plain-model".into(),
            api_key: "k".into(),
        };
        let messages = vec![json!({"role":"user","content":[{"type":"text","text":"What is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]})];
        let unchanged = without_images_for_text_models(&config, messages.clone());
        assert_eq!(unchanged[0]["content"][1]["type"], "image_url");
        let list = crate::models::parse_models(&json!({"data":[{"id":"plain-model","architecture":{"input_modalities":["text"],"output_modalities":["text"]}}]}), "test-text-only").unwrap();
        crate::models::remember("test-text-only", &list);
        let stripped = without_images_for_text_models(&config, messages);
        assert_eq!(stripped[0]["content"][1]["type"], "text");
        assert_eq!(stripped[0]["content"][0]["text"], "What is this?");
    }
}
