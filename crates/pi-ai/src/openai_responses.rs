//! OpenAI Responses API provider (used by GitHub Copilot for newer models).
//!
//! Implements `POST {base_url}/responses` — a newer API that replaces Chat
//! Completions for models like gpt-5.4-mini.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use crate::provider::{ChatStream, LlmProvider, ProviderError};
use crate::sse::SseStream;
use crate::types::*;

// ─── Provider struct ─────────────────────────────────────────────────────────

pub struct OpenAiResponsesProvider {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiResponsesProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

// ─── Message conversion ──────────────────────────────────────────────────────

/// Convert our internal `Message` slice into the Responses API `input` format.
///
/// Key differences from Completions:
/// - System prompt is a top-level `{ "role": "system", "content": "..." }` item
/// - User messages use `input_text` type in content array
/// - Assistant tool calls become `function_call` content items (not `tool_calls`)
/// - Tool results become top-level `function_call_output` items (not role=tool messages)
pub fn messages_to_responses(messages: &[Message], system_prompt: Option<&str>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();

    if let Some(sys) = system_prompt {
        out.push(json!({ "role": "system", "content": sys }));
    }

    for msg in messages {
        match msg {
            Message::User { content } => {
                let parts: Vec<Value> = content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => {
                            Some(json!({ "type": "input_text", "text": text }))
                        }
                        ContentBlock::Image { data, mime_type } => Some(json!({
                            "type": "input_image",
                            "detail": "auto",
                            "image_url": format!("data:{mime_type};base64,{data}")
                        })),
                        _ => None,
                    })
                    .collect();

                if parts.is_empty() {
                    continue;
                }

                // Send plain text as a string if there's a single text-only part
                if parts.len() == 1 {
                    if let Some(Value::Object(ref o)) = parts.first() {
                        if o.get("type") == Some(&Value::String("input_text".to_owned())) {
                            if let Some(text) = o.get("text").and_then(|t| t.as_str()) {
                                out.push(json!({ "role": "user", "content": text }));
                                continue;
                            }
                        }
                    }
                }

                out.push(json!({ "role": "user", "content": Value::Array(parts) }));
            }

            Message::Assistant { content, .. } => {
                // Collect assistant content blocks.
                // Text → { "type": "output_text", "text": "..." }
                // ToolCall → { "type": "function_call", "id": "...", "call_id": "...", "name": "...", "arguments": "..." }
                let mut assistant_content: Vec<Value> = Vec::new();

                for block in content {
                    match block {
                        ContentBlock::Text { text } => {
                            if !text.is_empty() {
                                assistant_content.push(json!({
                                    "type": "output_text",
                                    "text": text,
                                    "annotations": []
                                }));
                            }
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            assistant_content.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": arguments.to_string()
                            }));
                        }
                        ContentBlock::Thinking { .. } => {
                            // Thinking blocks are not replayed in Responses API input
                        }
                        ContentBlock::Image { .. } => {
                            // Images in assistant messages are not supported
                        }
                    }
                }

                if assistant_content.is_empty() {
                    continue;
                }

                // Wrap text-only single blocks as a message item; tool calls go top-level
                let has_tool_calls = assistant_content
                    .iter()
                    .any(|c| c["type"] == "function_call");
                let has_text = assistant_content.iter().any(|c| c["type"] == "output_text");

                if has_tool_calls && !has_text {
                    // All function calls go directly into the output array
                    for item in assistant_content {
                        out.push(item);
                    }
                } else if has_tool_calls && has_text {
                    // Mixed: wrap text in a message, then emit function_calls separately
                    let text_parts: Vec<Value> = assistant_content
                        .iter()
                        .filter(|c| c["type"] == "output_text")
                        .cloned()
                        .collect();
                    let tool_parts: Vec<Value> = assistant_content
                        .into_iter()
                        .filter(|c| c["type"] == "function_call")
                        .collect();

                    out.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": text_parts
                    }));
                    for item in tool_parts {
                        out.push(item);
                    }
                } else {
                    // Text only — wrap in a message item
                    out.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": assistant_content
                    }));
                }
            }

            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let text: String = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");

                out.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": text
                }));
            }
        }
    }

    out
}

/// Convert our `ToolDefinition` list to the Responses API wire format.
fn tools_to_responses(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters
            })
        })
        .collect()
}

// ─── LlmProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    fn name(&self) -> &str {
        "github-copilot-responses"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatStream, ProviderError> {
        let base_url = &request.model.base_url;
        let url = format!("{base_url}/responses");

        // Determine initiator based on the last message role.
        let initiator = match request.messages.last() {
            Some(Message::User { .. }) => "user",
            _ => "agent",
        };

        let input = messages_to_responses(&request.messages, request.system_prompt.as_deref());
        let max_tokens = request.max_tokens.unwrap_or(request.model.max_tokens);

        let mut body = json!({
            "model": request.model.id,
            "input": input,
            "stream": true,
            "max_output_tokens": max_tokens,
        });

        if !request.tools.is_empty() {
            body["tools"] = Value::Array(tools_to_responses(&request.tools));
        }

        // Reasoning support: map ThinkingLevel → effort string
        if request.model.reasoning {
            let effort = match request.thinking_level {
                Some(ThinkingLevel::Minimal) => "minimal",
                Some(ThinkingLevel::Low) => "low",
                Some(ThinkingLevel::High) => "high",
                Some(ThinkingLevel::Xhigh) => "xhigh",
                _ => "medium",
            };
            body["reasoning"] = json!({ "effort": effort });
        }

        // Build request with GitHub Copilot headers.
        let mut req = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("X-Initiator", initiator)
            .header("Openai-Intent", "conversation-edits")
            .header("Editor-Version", "vscode/1.107.0")
            .header("Copilot-Integration-Id", "vscode-chat")
            .header("Content-Type", "application/json");

        // Merge any model-level custom headers.
        for (k, v) in &request.model.headers {
            req = req.header(k, v);
        }

        let response = req.json(&body).send().await?;

        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_ms = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| s * 1000);
            return Err(ProviderError::RateLimit { retry_after_ms });
        }
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("HTTP {status}: {body_text}")));
        }

        let byte_stream = response.bytes_stream();
        let sse = SseStream::new(byte_stream);

        // Clone values needed inside the async stream closure.
        let input_cost = request.model.cost.input;
        let output_cost = request.model.cost.output;

        let stream = sse.flat_map(move |sse_result| {
            let events = parse_responses_sse_chunk(sse_result, input_cost, output_cost);
            futures::stream::iter(events)
        });

        Ok(Box::pin(stream))
    }
}

// ─── SSE chunk parser ────────────────────────────────────────────────────────

fn parse_responses_sse_chunk(
    result: Result<crate::sse::SseEvent, crate::sse::SseError>,
    input_cost_per_mtok: f64,
    output_cost_per_mtok: f64,
) -> Vec<Result<ChatEvent, ProviderError>> {
    let event = match result {
        Ok(e) => e,
        Err(e) => return vec![Err(ProviderError::Sse(e.to_string()))],
    };

    let event_type = match event.event_type.as_deref() {
        Some(t) => t,
        None => return vec![],
    };

    let v: Value = match serde_json::from_str(&event.data) {
        Ok(v) => v,
        Err(e) => return vec![Err(ProviderError::Json(e))],
    };

    let mut events: Vec<Result<ChatEvent, ProviderError>> = Vec::new();

    match event_type {
        // ── Text ────────────────────────────────────────────────────────────
        "response.content_part.added" => {
            // A new content part appeared on a message output item.
            // If it's output_text → emit TextStart.
            if v["part"]["type"].as_str() == Some("output_text") {
                events.push(Ok(ChatEvent::TextStart));
            }
        }

        "response.output_text.delta" => {
            if let Some(delta) = v["delta"].as_str() {
                if !delta.is_empty() {
                    events.push(Ok(ChatEvent::TextDelta {
                        text: delta.to_owned(),
                    }));
                }
            }
        }

        "response.output_text.done" => {
            events.push(Ok(ChatEvent::TextEnd));
        }

        // ── Thinking / reasoning ─────────────────────────────────────────
        "response.reasoning_summary_text.delta" | "response.reasoning_summary_part.added" => {
            if let Some(delta) = v["delta"].as_str() {
                if !delta.is_empty() {
                    events.push(Ok(ChatEvent::ThinkingDelta {
                        text: delta.to_owned(),
                    }));
                }
            }
        }

        // ── Tool calls ───────────────────────────────────────────────────
        "response.output_item.added" => {
            // A new output item was added to the response.
            let item_type = v["item"]["type"].as_str().unwrap_or("");
            match item_type {
                "function_call" => {
                    let id = v["item"]["call_id"].as_str().unwrap_or("").to_owned();
                    let name = v["item"]["name"].as_str().unwrap_or("").to_owned();
                    events.push(Ok(ChatEvent::ToolCallStart { id, name }));
                }
                "message" => {
                    // TextStart is emitted later via response.content_part.added
                }
                "reasoning" => {
                    events.push(Ok(ChatEvent::ThinkingStart));
                }
                _ => {}
            }
        }

        "response.function_call_arguments.delta" => {
            let id = v["call_id"].as_str().unwrap_or("").to_owned();
            let name = String::new(); // name not repeated in delta events
            if let Some(delta) = v["delta"].as_str() {
                if !delta.is_empty() {
                    events.push(Ok(ChatEvent::ToolCallDelta {
                        id,
                        name,
                        arguments_delta: delta.to_owned(),
                    }));
                }
            }
        }

        "response.function_call_arguments.done" => {
            let id = v["call_id"].as_str().unwrap_or("").to_owned();
            let name = v["name"].as_str().unwrap_or("").to_owned();
            let args_str = v["arguments"].as_str().unwrap_or("{}");
            let arguments: Value =
                serde_json::from_str(args_str).unwrap_or(Value::Object(Default::default()));
            events.push(Ok(ChatEvent::ToolCallDone {
                id,
                name,
                arguments,
            }));
        }

        // ── Completion ───────────────────────────────────────────────────
        "response.completed" => {
            let usage_val = &v["response"]["usage"];
            let input = usage_val["input_tokens"].as_u64().unwrap_or(0);
            let output = usage_val["output_tokens"].as_u64().unwrap_or(0);
            let cache_read = usage_val["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0);

            let usage = Usage {
                input,
                output,
                cache_read,
                cache_write: 0,
                total_tokens: input + output,
            };
            let cost_input = (input as f64 / 1_000_000.0) * input_cost_per_mtok;
            let cost_output = (output as f64 / 1_000_000.0) * output_cost_per_mtok;
            let cost = Cost {
                input: cost_input,
                output: cost_output,
                cache_read: 0.0,
                cache_write: 0.0,
                total: cost_input + cost_output,
            };

            // Determine stop reason from response status
            let status = v["response"]["status"].as_str().unwrap_or("completed");
            let stop_reason = match status {
                "completed" => StopReason::Stop,
                "incomplete" => StopReason::Length,
                _ => StopReason::Stop,
            };

            events.push(Ok(ChatEvent::Done {
                stop_reason,
                usage,
                cost,
            }));
        }

        // ── Error ────────────────────────────────────────────────────────
        "error" => {
            let msg = format!(
                "Error {}: {}",
                v["code"].as_str().unwrap_or("unknown"),
                v["message"].as_str().unwrap_or("unknown error")
            );
            events.push(Err(ProviderError::Other(msg)));
        }

        "response.failed" => {
            let error = &v["response"]["error"];
            let msg = if !error.is_null() {
                format!(
                    "{}: {}",
                    error["code"].as_str().unwrap_or("unknown"),
                    error["message"].as_str().unwrap_or("no message")
                )
            } else {
                "Response failed".to_owned()
            };
            events.push(Err(ProviderError::Other(msg)));
        }

        // ── Ignore other events ──────────────────────────────────────────
        _ => {}
    }

    events
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_block(s: &str) -> ContentBlock {
        ContentBlock::Text { text: s.to_owned() }
    }

    fn tool_block(id: &str, name: &str, args: Value) -> ContentBlock {
        ContentBlock::ToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            arguments: args,
        }
    }

    // ── messages_to_responses ────────────────────────────────────────────────

    #[test]
    fn user_message_plain_text() {
        let messages = vec![Message::User {
            content: vec![text_block("hello")],
        }];
        let result = messages_to_responses(&messages, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        // Single plain text is sent as a string
        assert_eq!(result[0]["content"], "hello");
    }

    #[test]
    fn system_prompt_is_first() {
        let messages = vec![Message::User {
            content: vec![text_block("hi")],
        }];
        let result = messages_to_responses(&messages, Some("Be helpful"));
        assert_eq!(result[0]["role"], "system");
        assert_eq!(result[0]["content"], "Be helpful");
        assert_eq!(result[1]["role"], "user");
    }

    #[test]
    fn assistant_message_text_only() {
        let messages = vec![Message::Assistant {
            content: vec![text_block("sure")],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_responses(&messages, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "message");
        assert_eq!(result[0]["role"], "assistant");
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "output_text");
        assert_eq!(content[0]["text"], "sure");
    }

    #[test]
    fn assistant_message_tool_call_only() {
        let messages = vec![Message::Assistant {
            content: vec![tool_block("call-1", "bash", json!({"cmd": "ls"}))],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_responses(&messages, None);
        // Tool calls go directly into the output (not wrapped in a message item)
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "function_call");
        assert_eq!(result[0]["call_id"], "call-1");
        assert_eq!(result[0]["name"], "bash");
        // Arguments should be a JSON string
        let args_str = result[0]["arguments"].as_str().unwrap();
        let args: Value = serde_json::from_str(args_str).unwrap();
        assert_eq!(args["cmd"], "ls");
    }

    #[test]
    fn assistant_message_mixed_text_and_tool_call() {
        let messages = vec![Message::Assistant {
            content: vec![
                text_block("I'll run that"),
                tool_block("call-2", "read", json!({"path": "/tmp/f"})),
            ],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_responses(&messages, None);
        // Should produce: 1 message item + 1 function_call item
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["type"], "message");
        assert_eq!(result[1]["type"], "function_call");
    }

    #[test]
    fn tool_result_becomes_function_call_output() {
        let messages = vec![Message::ToolResult {
            tool_call_id: "call-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![text_block("file contents")],
            is_error: false,
        }];
        let result = messages_to_responses(&messages, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "function_call_output");
        assert_eq!(result[0]["call_id"], "call-1");
        assert_eq!(result[0]["output"], "file contents");
    }

    #[test]
    fn user_message_with_image() {
        let messages = vec![Message::User {
            content: vec![
                text_block("describe this"),
                ContentBlock::Image {
                    data: "abc123".to_owned(),
                    mime_type: "image/png".to_owned(),
                },
            ],
        }];
        let result = messages_to_responses(&messages, None);
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[1]["type"], "input_image");
        assert!(content[1]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn empty_messages_with_system_prompt() {
        let result = messages_to_responses(&[], Some("Be helpful"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "system");
    }

    #[test]
    fn empty_messages_without_system_prompt() {
        let result = messages_to_responses(&[], None);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn tool_result_with_empty_content() {
        let messages = vec![Message::ToolResult {
            tool_call_id: "tc-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![],
            is_error: false,
        }];
        let result = messages_to_responses(&messages, None);
        assert_eq!(result[0]["type"], "function_call_output");
        assert_eq!(result[0]["output"], "");
    }

    #[test]
    fn multiple_tool_results_in_sequence() {
        let messages = vec![
            Message::ToolResult {
                tool_call_id: "tc-1".to_owned(),
                tool_name: "bash".to_owned(),
                content: vec![text_block("result 1")],
                is_error: false,
            },
            Message::ToolResult {
                tool_call_id: "tc-2".to_owned(),
                tool_name: "read".to_owned(),
                content: vec![text_block("result 2")],
                is_error: false,
            },
        ];
        let result = messages_to_responses(&messages, None);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["call_id"], "tc-1");
        assert_eq!(result[1]["call_id"], "tc-2");
    }

    #[test]
    fn large_tool_arguments_serialized_correctly() {
        let large_value: String = "x".repeat(10 * 1024 + 1);
        let args = json!({ "data": large_value });
        let messages = vec![Message::Assistant {
            content: vec![tool_block("tc-big", "process", args)],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_responses(&messages, None);
        let args_str = result[0]["arguments"].as_str().unwrap();
        let reparsed: Value = serde_json::from_str(args_str).unwrap();
        assert_eq!(reparsed["data"].as_str().unwrap().len(), 10 * 1024 + 1);
    }
}
