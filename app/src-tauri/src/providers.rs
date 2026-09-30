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
    let mut request = client
        .post(format!("{}{}", config.base_url, path))
        .json(&payload);
    if config.api_format == ANTHROPIC {
        request = request.header("anthropic-version", "2023-06-01");
    }
    authorized(request, config)
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
    let messages = without_images_for_text_models(config, public_messages(messages));
    let (path, mut payload) = match config.api_format.as_str() {
        ANTHROPIC => (
            "/messages",
            anthropic_payload(&config.model, &messages, tools),
        ),
        RESPONSES => (
            "/responses",
            responses_payload(&config.model, &messages, tools),
        ),
        _ => ("/chat/completions", openai_chat_payload(config, &messages, tools)),
    };
    if let Some(effort) = effort {
        apply_effort(&mut payload, config, effort);
    }
    let mut request = client
        .post(format!("{}{}", config.base_url, path))
        .json(&payload);
    if config.api_format == ANTHROPIC {
        request = request.header("anthropic-version", "2023-06-01");
    }
    authorized(request, config)
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
    let spare = matches!(
        config.provider_id.as_str(),
        "gemini" | "groq" | "cerebras" | "mistral" | "nvidia" | "huggingface" | "xai" | "modelscope"
    );
    if let Some(object) = payload.as_object_mut() {
        if spare {
            object.remove("stream_options");
        }
        if config.provider_id == "gemini" {
            object.remove("parallel_tool_calls");
        }
    }
    payload
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
    let named = lower
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

fn ephemeral() -> Value {
    json!({"type":"ephemeral"})
}

/// Claude models reached through OpenRouter cache only what is marked, like the native API.
/// Other OpenRouter models (OpenAI, DeepSeek, Gemini, Kimi, Qwen) cache a repeated prefix on
/// their own, which works because Neru only ever appends to the conversation.
fn wants_cache_marks(config: &ProviderConfig) -> bool {
    config.provider_id == "openrouter" && config.model.starts_with("anthropic/")
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

pub fn response_error(status: reqwest::StatusCode, body: &Value) -> String {
    let message = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or("request failed");
    format!(
        "Provider HTTP {status}: {}",
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
        assert!(payload.get("stream_options").is_none());
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
