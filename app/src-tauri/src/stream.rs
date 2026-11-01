use serde_json::{Value, json};

use crate::providers::{ANTHROPIC, RESPONSES, normalize_message};

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Rebuilds one assistant message from a provider's server-sent events,
/// returning text deltas as they arrive so the UI can render them live.
pub struct StreamAccumulator {
    format: String,
    text: String,
    calls: Vec<(u64, PartialCall)>,
    final_body: Option<Value>,
    buffer: String,
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
    /// Prompt tokens the provider served from its cache.
    cached_tokens: Option<usize>,
    /// Characters of hidden reasoning streamed so far (DeepSeek R1, Qwen thinking, etc.).
    pub reasoning: usize,
    pub reasoning_text: String,
}

impl StreamAccumulator {
    pub fn new(format: &str) -> Self {
        Self {
            format: format.to_string(),
            text: String::new(),
            calls: Vec::new(),
            final_body: None,
            buffer: String::new(),
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
            reasoning: 0,
            reasoning_text: String::new(),
        }
    }

    fn call(&mut self, index: u64) -> &mut PartialCall {
        if let Some(position) = self.calls.iter().position(|(key, _)| *key == index) {
            return &mut self.calls[position].1;
        }
        self.calls.push((index, PartialCall::default()));
        &mut self.calls.last_mut().unwrap().1
    }

    /// Feeds raw bytes from the response body; returns the text deltas they completed.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        let mut deltas = Vec::new();
        while let Some(end) = self.buffer.find('\n') {
            let line = self.buffer[..end].trim_end_matches('\r').to_string();
            self.buffer.drain(..=end);
            let Some(data) = line.strip_prefix("data:") else {
                // Ollama's native /api/chat streams one JSON object per line instead of server-sent events.
                if line.starts_with('{') {
                    if let Ok(event) = serde_json::from_str::<Value>(&line) {
                        if event.get("done").is_some() || event.get("error").is_some() {
                            if let Some(delta) = self.push_ollama(&event)? {
                                deltas.push(delta);
                            }
                        }
                    }
                }
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(delta) = self.push_event(&event)? {
                if !delta.is_empty() {
                    deltas.push(delta);
                }
            }
        }
        Ok(deltas)
    }

    /// One line of Ollama's native stream: text, thinking, whole tool calls, and token counts at the end.
    fn push_ollama(&mut self, event: &Value) -> Result<Option<String>, String> {
        if let Some(error) = event["error"].as_str() {
            return Err(format!("Provider stream error: {}", error.chars().take(300).collect::<String>()));
        }
        let message = &event["message"];
        if let Some(thought) = message["thinking"].as_str().filter(|text| !text.is_empty()) {
            self.reasoning += thought.chars().count();
            self.reasoning_text.push_str(thought);
        }
        for part in message["tool_calls"].as_array().into_iter().flatten() {
            let index = self.calls.len() as u64;
            let call = self.call(index);
            call.name = part["function"]["name"].as_str().unwrap_or("").to_string();
            call.arguments = match &part["function"]["arguments"] {
                Value::String(text) => text.clone(),
                Value::Null => "{}".into(),
                other => other.to_string(),
            };
        }
        if event["done"] == true {
            if let Some(tokens) = event["prompt_eval_count"].as_u64() {
                self.prompt_tokens = Some(tokens as usize);
            }
            if let Some(tokens) = event["eval_count"].as_u64() {
                self.completion_tokens = Some(tokens as usize);
            }
        }
        Ok(self.text_delta(message["content"].as_str().filter(|text| !text.is_empty())))
    }

    fn push_event(&mut self, event: &Value) -> Result<Option<String>, String> {
        self.note_usage(event);
        if let Some(message) = event["error"]["message"].as_str() {
            return Err(format!(
                "Provider stream error: {}",
                message.chars().take(300).collect::<String>()
            ));
        }
        match self.format.as_str() {
            ANTHROPIC => match event["type"].as_str().unwrap_or("") {
                "content_block_start" => {
                    let index = event["index"].as_u64().unwrap_or(0);
                    let block = &event["content_block"];
                    if block["type"] == "tool_use" {
                        let call = self.call(index);
                        call.id = block["id"].as_str().unwrap_or("").to_string();
                        call.name = block["name"].as_str().unwrap_or("").to_string();
                    }
                    Ok(None)
                }
                "content_block_delta" => {
                    let index = event["index"].as_u64().unwrap_or(0);
                    let delta = &event["delta"];
                    match delta["type"].as_str().unwrap_or("") {
                        "text_delta" => Ok(self.text_delta(delta["text"].as_str())),
                        "input_json_delta" => {
                            let partial = delta["partial_json"].as_str().unwrap_or("").to_string();
                            self.call(index).arguments.push_str(&partial);
                            Ok(None)
                        }
                        _ => Ok(None),
                    }
                }
                _ => Ok(None),
            },
            RESPONSES => match event["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => Ok(self.text_delta(event["delta"].as_str())),
                "response.completed" => {
                    self.final_body = Some(event["response"].clone());
                    Ok(None)
                }
                "response.failed" => Err(format!(
                    "Provider stream error: {}",
                    event["response"]["error"]["message"]
                        .as_str()
                        .unwrap_or("response failed")
                )),
                _ => Ok(None),
            },
            _ => {
                let delta = &event["choices"][0]["delta"];
                if let Some(calls) = delta["tool_calls"].as_array() {
                    for (position, part) in calls.iter().enumerate() {
                        let index = part["index"].as_u64().unwrap_or(position as u64);
                        let call = self.call(index);
                        if let Some(id) = part["id"].as_str().filter(|id| !id.is_empty()) {
                            call.id = id.to_string();
                        }
                        if let Some(name) = part["function"]["name"].as_str() {
                            // Some hosts (NVIDIA NIM, several OpenRouter upstreams) repeat the full
                            // name on every chunk; only genuinely split names are concatenated.
                            if call.name.is_empty() || name.starts_with(call.name.as_str()) {
                                call.name = name.to_string();
                            } else if !call.name.ends_with(name) {
                                call.name.push_str(name);
                            }
                        }
                        if let Some(arguments) = part["function"]["arguments"].as_str() {
                            call.arguments.push_str(arguments);
                        }
                    }
                }
                if let Some(thought) = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()) {
                    self.reasoning += thought.chars().count();
                    self.reasoning_text.push_str(thought);
                }
                Ok(self.text_delta(delta["content"].as_str()))
            }
        }
    }

    /// Whether the model has produced anything yet: text, reasoning or a tool call.
    pub fn started(&self) -> bool {
        !self.text.is_empty() || self.reasoning > 0 || !self.calls.is_empty() || self.final_body.is_some()
    }

    /// Tool calls still streaming, as (id, name, arguments so far), so the window can show the
    /// file a model is writing before the call completes.
    pub fn partial_calls(&self) -> Vec<(String, &str, &str)> {
        self.calls
            .iter()
            .filter(|(_, call)| !call.name.is_empty())
            .map(|(index, call)| {
                let id = if call.id.is_empty() { format!("call_{index}") } else { call.id.clone() };
                (id, call.name.as_str(), call.arguments.as_str())
            })
            .collect()
    }

    fn note_usage(&mut self, event: &Value) {
        let tokens = event["usage"]["prompt_tokens"]
            .as_u64()
            .or_else(|| event["usage"]["input_tokens"].as_u64())
            .or_else(|| event["message"]["usage"]["input_tokens"].as_u64())
            .or_else(|| event["response"]["usage"]["input_tokens"].as_u64())
            // Groq streams its usage under x_groq instead of honoring stream_options.
            .or_else(|| event["x_groq"]["usage"]["prompt_tokens"].as_u64());
        if let Some(tokens) = tokens {
            self.prompt_tokens = Some(tokens as usize);
        }
        let output = event["usage"]["completion_tokens"]
            .as_u64()
            .or_else(|| event["usage"]["output_tokens"].as_u64())
            .or_else(|| event["message"]["usage"]["output_tokens"].as_u64())
            .or_else(|| event["response"]["usage"]["output_tokens"].as_u64())
            .or_else(|| event["x_groq"]["usage"]["completion_tokens"].as_u64());
        // Some providers repeat running totals; Anthropic sends one at the start and the total at the end.
        if let Some(output) = output {
            self.completion_tokens = Some(self.completion_tokens.unwrap_or(0).max(output as usize));
        }
        if let Some(usage) = [&event["usage"], &event["message"]["usage"], &event["response"]["usage"], &event["x_groq"]["usage"], &event["usageMetadata"]].into_iter().find(|usage| usage.is_object()) {
            if let Some(cached) = cached_tokens(usage) {
                self.cached_tokens = Some(cached);
                // Anthropic counts cached tokens apart from input_tokens; the prompt is all of them.
                if self.format == ANTHROPIC {
                    let total = usage["input_tokens"].as_u64().unwrap_or(0) + usage["cache_read_input_tokens"].as_u64().unwrap_or(0) + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                    if total > 0 {
                        self.prompt_tokens = Some(total as usize);
                    }
                }
            }
        }
    }

    fn text_delta(&mut self, value: Option<&str>) -> Option<String> {
        let value = value?;
        self.text.push_str(value);
        Some(value.to_string())
    }

    /// Produces the same normalized assistant message the non-streaming path returns.
    pub fn finish(self) -> Result<Value, String> {
        if self.format == RESPONSES {
            if let Some(body) = self.final_body.clone() {
                let mut message = normalize_message(RESPONSES, &body)?;
                if message["content"].as_str().is_none_or(str::is_empty) && !self.text.is_empty() {
                    message["content"] = json!(self.text);
                }
                if let Some(tokens) = self.prompt_tokens.or_else(|| crate::providers::prompt_tokens(&body)) {
                    message["_prompt_tokens"] = json!(tokens);
                }
                if let Some(tokens) = self.completion_tokens.or_else(|| body["usage"]["output_tokens"].as_u64().map(|n| n as usize)) {
                    message["_completion_tokens"] = json!(tokens);
                }
                if let Some(tokens) = self.cached_tokens.or_else(|| cached_tokens(&body["usage"])) {
                    message["_cached_tokens"] = json!(tokens);
                }
                return Ok(message);
            }
        }
        let calls: Vec<Value> = self
            .calls
            .into_iter()
            .filter(|(_, call)| !call.name.is_empty())
            .map(|(index, call)| {
                let id = if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                };
                let arguments = if call.arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    call.arguments
                };
                json!({"id":id,"type":"function","function":{"name":call.name,"arguments":arguments}})
            })
            .collect();
        let mut message = json!({"role":"assistant","content":self.text});
        if !calls.is_empty() {
            message["tool_calls"] = json!(calls);
        }
        if let Some(tokens) = self.prompt_tokens {
            message["_prompt_tokens"] = json!(tokens);
        }
        if let Some(tokens) = self.completion_tokens {
            message["_completion_tokens"] = json!(tokens);
        }
        if let Some(tokens) = self.cached_tokens {
            message["_cached_tokens"] = json!(tokens);
        }
        Ok(message)
    }
}

/// Prompt tokens served from the provider's cache, in each provider's field: OpenAI and most
/// compatible hosts (`prompt_tokens_details.cached_tokens`), the Responses API, Anthropic
/// (`cache_read_input_tokens`), DeepSeek (`prompt_cache_hit_tokens`) and Gemini (`cachedContentTokenCount`).
pub fn cached_tokens(usage: &Value) -> Option<usize> {
    [
        &usage["prompt_tokens_details"]["cached_tokens"],
        &usage["input_tokens_details"]["cached_tokens"],
        &usage["cache_read_input_tokens"],
        &usage["prompt_cache_hit_tokens"],
        &usage["cached_content_token_count"],
        &usage["cachedContentTokenCount"],
    ]
    .into_iter()
    .find_map(Value::as_u64)
    .map(|tokens| tokens as usize)
}

/// Reads a string field from possibly unfinished JSON (tool arguments mid-stream).
/// Returns the decoded text so far and whether the string was closed.
pub fn partial_string_field(json: &str, key: &str) -> Option<(String, bool)> {
    let marker = format!("\"{key}\"");
    let start = json.find(&marker)? + marker.len();
    let rest = json[start..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::with_capacity(rest.len());
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some((out, true)),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{8}'),
                Some('f') => out.push('\u{c}'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() < 4 {
                        break;
                    }
                    // A lone surrogate half is dropped; its partner may still be streaming.
                    if let Some(decoded) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(decoded);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    Some((out, false))
}

/// The name a `<tool_call>` block gets when its JSON could not be read, so the agent can tell the model
/// what was wrong instead of silently dropping the call.
pub const INVALID_TOOL_CALL: &str = "invalid_tool_call";

/// Reads one `<tool_call>` body: `{"name": …, "arguments": {…}}`, also with `parameters`, `args` or
/// `input`, arguments as a JSON string, or stray text around the object.
fn parse_text_call(body: &str) -> Result<(String, String), String> {
    let body = body.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let call = match serde_json::from_str::<Value>(body) {
        Ok(call) => call,
        Err(error) => {
            let (Some(open), Some(close)) = (body.find('{'), body.rfind('}')) else { return Err(error.to_string()) };
            serde_json::from_str::<Value>(&body[open..=close]).map_err(|_| error.to_string())?
        }
    };
    let name = call["name"].as_str().or_else(|| call["function"]["name"].as_str()).filter(|name| !name.trim().is_empty()).ok_or("the object has no \"name\"")?;
    let arguments = ["arguments", "parameters", "args", "input"].iter().map(|key| &call[*key]).find(|value| !value.is_null()).or_else(|| Some(&call["function"]["arguments"])).cloned().unwrap_or(Value::Null);
    let arguments = match arguments {
        Value::String(text) => text,
        Value::Null => "{}".to_string(),
        other => other.to_string(),
    };
    Ok((name.trim().to_string(), arguments))
}

/// Some open models (Qwen, Hermes-style fine-tunes) answer with `<tool_call>{json}</tool_call>`
/// text when the host does not parse tool calls, and models on Neru's text protocol always do.
/// Turns those into real calls; a block that cannot be read becomes an [`INVALID_TOOL_CALL`].
pub fn recover_text_tool_calls(message: &mut Value) {
    if message["tool_calls"].as_array().is_some_and(|calls| !calls.is_empty()) {
        return;
    }
    let Some(text) = message["content"].as_str().map(str::to_string) else { return };
    if !text.contains("<tool_call>") {
        return;
    }
    let mut calls = Vec::new();
    let mut kept = String::new();
    let mut rest = text.as_str();
    while let Some(open) = rest.find("<tool_call>") {
        kept.push_str(&rest[..open]);
        let after = &rest[open + "<tool_call>".len()..];
        let (body, next) = match after.find("</tool_call>") {
            Some(close) => (&after[..close], &after[close + "</tool_call>".len()..]),
            None => (after, ""),
        };
        let (name, arguments) = match parse_text_call(body) {
            Ok(call) => call,
            Err(error) => (INVALID_TOOL_CALL.to_string(), json!({"error": error, "text": body.trim().chars().take(300).collect::<String>()}).to_string()),
        };
        calls.push(json!({"id": format!("call_text_{}", calls.len()), "type": "function", "function": {"name": name, "arguments": arguments}}));
        rest = next;
    }
    kept.push_str(rest);
    if !calls.is_empty() {
        message["content"] = json!(kept.trim());
        message["tool_calls"] = json!(calls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::CHAT;

    #[test]
    fn chat_stream_rebuilds_text_and_split_tool_calls() {
        let mut stream = StreamAccumulator::new(CHAT);
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"pa\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a.rs\\\"}\"}}]}}]}\n",
            "data: [DONE]\n"
        );
        let (first, rest) = body.split_at(30);
        let mut deltas = stream.push_bytes(first.as_bytes()).unwrap();
        deltas.extend(stream.push_bytes(rest.as_bytes()).unwrap());
        assert_eq!(deltas.concat(), "Hello");
        let message = stream.finish().unwrap();
        assert_eq!(message["content"], "Hello");
        assert_eq!(message["tool_calls"][0]["id"], "call_a");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.rs\"}"
        );
    }

    #[test]
    fn anthropic_stream_collects_tool_input() {
        let mut stream = StreamAccumulator::new(ANTHROPIC);
        let body = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Looking\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"search_text\"}}\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\\\"x\\\"}\"}}\n",
        );
        assert_eq!(stream.push_bytes(body.as_bytes()).unwrap(), vec!["Looking"]);
        let message = stream.finish().unwrap();
        assert_eq!(message["tool_calls"][0]["id"], "toolu_1");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"query\":\"x\"}"
        );
    }

    #[test]
    fn responses_stream_uses_completed_response() {
        let mut stream = StreamAccumulator::new(RESPONSES);
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"message\",\"content\":[{\"text\":\"Hi\"}]},{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"git_status\",\"arguments\":\"{}\"}]}}\n",
        );
        assert_eq!(stream.push_bytes(body.as_bytes()).unwrap(), vec!["Hi"]);
        let message = stream.finish().unwrap();
        assert_eq!(message["content"], "Hi");
        assert_eq!(message["tool_calls"][0]["id"], "c1");
    }

    #[test]
    fn repeated_tool_names_are_not_doubled() {
        let mut stream = StreamAccumulator::new(CHAT);
        let first = json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"read_file","arguments":"{\"path\""}}]}}]});
        let second = json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":":\"a\"}"}}]}}]});
        let body = format!("data: {first}\ndata: {second}\n");
        stream.push_bytes(body.as_bytes()).unwrap();
        let message = stream.finish().unwrap();
        assert_eq!(message["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a\"}");
    }

    #[test]
    fn partial_fields_decode_while_streaming() {
        let json = r#"{"path":"index.html","content":"<h1>\"Hi\"</h1>\n<p>café"#;
        assert_eq!(partial_string_field(json, "path"), Some(("index.html".into(), true)));
        assert_eq!(partial_string_field(json, "content"), Some(("<h1>\"Hi\"</h1>\n<p>café".into(), false)));
        assert_eq!(partial_string_field(r#"{"content":"ab\"#, "content"), Some(("ab".into(), false)));
        assert_eq!(partial_string_field(r#"{"path":"a"}"#, "content"), None);
    }

    #[test]
    fn text_tool_calls_are_recovered() {
        let mut message = json!({"role":"assistant","content":"Creating it.\n<tool_call>{\"name\":\"propose_write_file\",\"arguments\":{\"path\":\"a.txt\",\"content\":\"x\"}}</tool_call>"});
        recover_text_tool_calls(&mut message);
        assert_eq!(message["content"], "Creating it.");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "propose_write_file");
    }

    #[test]
    fn malformed_text_tool_calls_become_errors_not_silence() {
        let mut message = json!({"role":"assistant","content":"<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"</tool_call>"});
        recover_text_tool_calls(&mut message);
        assert_eq!(message["tool_calls"][0]["function"]["name"], INVALID_TOOL_CALL);
        let mut lenient = json!({"role":"assistant","content":"<tool_call>\n```json\n{\"name\":\"list_directory\",\"parameters\":{\"path\":\"src\"}}\n```\n</tool_call>"});
        recover_text_tool_calls(&mut lenient);
        assert_eq!(lenient["tool_calls"][0]["function"]["name"], "list_directory");
        assert_eq!(lenient["tool_calls"][0]["function"]["arguments"], "{\"path\":\"src\"}");
    }

    #[test]
    fn cached_tokens_are_read_from_every_provider_shape() {
        let mut stream = StreamAccumulator::new(CHAT);
        stream.push_bytes(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":30,\"prompt_tokens_details\":{\"cached_tokens\":1024}}}\n").unwrap();
        let message = stream.finish().unwrap();
        assert_eq!(message["_prompt_tokens"], 1200);
        assert_eq!(message["_cached_tokens"], 1024);
        let mut anthropic = StreamAccumulator::new(ANTHROPIC);
        anthropic.push_bytes(b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":3000,\"cache_creation_input_tokens\":100}}}\n").unwrap();
        let message = anthropic.finish().unwrap();
        assert_eq!(message["_prompt_tokens"], 3112, "the whole prompt, cached parts included");
        assert_eq!(message["_cached_tokens"], 3000);
        assert_eq!(cached_tokens(&json!({"prompt_cache_hit_tokens": 64})), Some(64));
        assert_eq!(cached_tokens(&json!({"cachedContentTokenCount": 8})), Some(8));
        assert_eq!(cached_tokens(&json!({"prompt_tokens": 8})), None);
    }

    #[test]
    fn ollama_native_lines_stream_text_calls_and_counts() {
        let mut stream = StreamAccumulator::new(CHAT);
        let body = concat!(
            "{\"message\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"done\":false}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"lo\"},\"done\":false}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"a.rs\"}}}]},\"done\":false}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"prompt_eval_count\":900,\"eval_count\":40}\n",
        );
        assert_eq!(stream.push_bytes(body.as_bytes()).unwrap().concat(), "Hello");
        let message = stream.finish().unwrap();
        assert_eq!(message["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a.rs\"}");
        assert_eq!(message["_prompt_tokens"], 900);
        assert_eq!(message["_completion_tokens"], 40);
        assert!(StreamAccumulator::new(CHAT).push_bytes(b"{\"error\":\"model not found\"}\n").is_err());
    }

    #[test]
    fn stream_errors_surface() {
        let mut stream = StreamAccumulator::new(CHAT);
        assert!(
            stream
                .push_bytes(b"data: {\"error\":{\"message\":\"rate limited\"}}\n")
                .is_err()
        );
    }
}
