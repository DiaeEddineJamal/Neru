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
                            call.name.push_str(name);
                        }
                        if let Some(arguments) = part["function"]["arguments"].as_str() {
                            call.arguments.push_str(arguments);
                        }
                    }
                }
                Ok(self.text_delta(delta["content"].as_str()))
            }
        }
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
        Ok(message)
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
    fn stream_errors_surface() {
        let mut stream = StreamAccumulator::new(CHAT);
        assert!(
            stream
                .push_bytes(b"data: {\"error\":{\"message\":\"rate limited\"}}\n")
                .is_err()
        );
    }
}
