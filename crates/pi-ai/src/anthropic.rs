//! Anthropic Messages provider (used by MiniMax at https://api.minimax.io/anthropic).

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::provider::{ChatStream, LlmProvider, ProviderError};
use crate::sse::SseStream;
use crate::types::*;

// ─── Provider struct ─────────────────────────────────────────────────────────

pub struct AnthropicMessagesProvider {
    api_key: String,
    client: reqwest::Client,
}

impl AnthropicMessagesProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

// ─── Message conversion ──────────────────────────────────────────────────────

/// Convert our internal `Message` list to the Anthropic wire format.
///
/// Multiple consecutive `ToolResult` messages are collapsed into a single
/// `{ "role": "user", "content": [{ "type": "tool_result", … }] }` entry.
pub fn messages_to_anthropic(messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        match &messages[i] {
            Message::User { content } => {
                let blocks: Vec<Value> = content
                    .iter()
                    .filter_map(content_block_to_anthropic)
                    .collect();
                let value = if blocks.len() == 1 {
                    if let Some(Value::Object(ref o)) = blocks.first() {
                        if let Some(Value::String(t)) = o.get("text") {
                            // Simple text — send as string.
                            out.push(json!({ "role": "user", "content": t }));
                            i += 1;
                            continue;
                        }
                    }
                    Value::Array(blocks)
                } else {
                    Value::Array(blocks)
                };
                out.push(json!({ "role": "user", "content": value }));
                i += 1;
            }

            Message::Assistant { content, .. } => {
                let blocks: Vec<Value> = content
                    .iter()
                    .filter_map(assistant_content_block_to_anthropic)
                    .collect();
                out.push(json!({ "role": "assistant", "content": blocks }));
                i += 1;
            }

            Message::ToolResult { .. } => {
                // Collect consecutive ToolResult messages.
                let mut tool_results: Vec<Value> = Vec::new();
                while i < messages.len() {
                    if let Message::ToolResult {
                        tool_call_id,
                        content,
                        is_error,
                        ..
                    } = &messages[i]
                    {
                        let result_content: Vec<Value> = content
                            .iter()
                            .filter_map(content_block_to_anthropic)
                            .collect();

                        let content_value = if result_content.len() == 1 {
                            if let Some(Value::Object(ref o)) = result_content.first() {
                                if let Some(Value::String(t)) = o.get("text") {
                                    Value::String(t.clone())
                                } else {
                                    Value::Array(result_content)
                                }
                            } else {
                                Value::Array(result_content)
                            }
                        } else {
                            Value::Array(result_content)
                        };

                        let mut entry = json!({
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content_value
                        });
                        if *is_error {
                            entry["is_error"] = json!(true);
                        }
                        tool_results.push(entry);
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(json!({ "role": "user", "content": tool_results }));
            }
        }
    }

    out
}

fn content_block_to_anthropic(block: &ContentBlock) -> Option<Value> {
    match block {
        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentBlock::Image { data, mime_type } => Some(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": mime_type,
                "data": data
            }
        })),
        _ => None,
    }
}

fn assistant_content_block_to_anthropic(block: &ContentBlock) -> Option<Value> {
    match block {
        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentBlock::Thinking {
            thinking,
            signature,
            redacted,
        } => {
            if *redacted {
                Some(json!({ "type": "redacted_thinking" }))
            } else {
                Some(json!({
                    "type": "thinking",
                    "thinking": thinking,
                    "signature": signature
                }))
            }
        }
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
        } => Some(json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": arguments
        })),
        _ => None,
    }
}

fn tools_to_anthropic(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters
            })
        })
        .collect()
}

// ─── LlmProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for AnthropicMessagesProvider {
    fn name(&self) -> &str {
        "minimax"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatStream, ProviderError> {
        let base_url = &request.model.base_url;
        let url = format!("{base_url}/v1/messages");

        let messages = messages_to_anthropic(&request.messages);
        let max_tokens = request.max_tokens.unwrap_or(request.model.max_tokens);

        let mut body = json!({
            "model": request.model.id,
            "messages": messages,
            "max_tokens": max_tokens,
            "stream": true,
        });

        // System prompt with cache_control.
        if let Some(sys) = &request.system_prompt {
            body["system"] = json!([{
                "type": "text",
                "text": sys,
                "cache_control": { "type": "ephemeral" }
            }]);
        }

        // Tools.
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(tools_to_anthropic(&request.tools));
        }

        // Thinking / reasoning budget.
        if request.model.reasoning {
            let budget = thinking_budget(&request.thinking_level);
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        }

        let mut req = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header(
                "anthropic-beta",
                "fine-grained-tool-streaming-2025-05-14,interleaved-thinking-2025-05-14",
            );

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

        let input_cost = request.model.cost.input;
        let output_cost = request.model.cost.output;
        let cache_read_cost = request.model.cost.cache_read;
        let cache_write_cost = request.model.cost.cache_write;

        let stream = AnthropicStreamParser::new(
            sse,
            input_cost,
            output_cost,
            cache_read_cost,
            cache_write_cost,
        );
        Ok(Box::pin(stream))
    }
}

fn thinking_budget(level: &Option<ThinkingLevel>) -> u64 {
    match level {
        None => 8192,
        Some(ThinkingLevel::Minimal) => 1024,
        Some(ThinkingLevel::Low) => 4096,
        Some(ThinkingLevel::Medium) => 8192,
        Some(ThinkingLevel::High) => 16384,
        Some(ThinkingLevel::Xhigh) => 32768,
    }
}

// ─── Streaming state machine ─────────────────────────────────────────────────
//
// Anthropic SSE uses typed events (message_start, content_block_start, etc.).
// We maintain per-block state to accumulate tool input JSON before emitting
// ToolCallDone on content_block_stop.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use pin_project_lite::pin_project;

#[derive(Debug)]
enum AnthropicBlock {
    None,
    Text,
    Thinking,
    ToolCall {
        id: String,
        name: String,
        args_buf: String,
    },
}

pin_project! {
    struct AnthropicStreamParser<S> {
        #[pin]
        inner: S,
        current_block: AnthropicBlock,
        pending: VecDeque<Result<ChatEvent, ProviderError>>,
        stop_reason: Option<StopReason>,
        usage: Usage,
        input_cost: f64,
        output_cost: f64,
        cache_read_cost: f64,
        cache_write_cost: f64,
        done: bool,
    }
}

impl<S> AnthropicStreamParser<S>
where
    S: futures::Stream<Item = Result<crate::sse::SseEvent, crate::sse::SseError>>,
{
    fn new(
        inner: S,
        input_cost: f64,
        output_cost: f64,
        cache_read_cost: f64,
        cache_write_cost: f64,
    ) -> Self {
        Self {
            inner,
            current_block: AnthropicBlock::None,
            pending: VecDeque::new(),
            stop_reason: None,
            usage: Usage::default(),
            input_cost,
            output_cost,
            cache_read_cost,
            cache_write_cost,
            done: false,
        }
    }

    fn process_event(
        event_type: Option<&str>,
        data: &str,
        current_block: &mut AnthropicBlock,
        pending: &mut VecDeque<Result<ChatEvent, ProviderError>>,
        stop_reason: &mut Option<StopReason>,
        usage: &mut Usage,
    ) {
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                pending.push_back(Err(ProviderError::Json(e)));
                return;
            }
        };

        match event_type {
            Some("message_start") => {
                // Capture initial usage (input tokens).
                if let Some(u) = v["message"]["usage"].as_object() {
                    usage.input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    usage.cache_read = u
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    usage.cache_write = u
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                }
                pending.push_back(Ok(ChatEvent::Start));
            }

            Some("content_block_start") => {
                let block_type = v["content_block"]["type"].as_str().unwrap_or("");
                match block_type {
                    "text" => {
                        pending.push_back(Ok(ChatEvent::TextStart));
                        *current_block = AnthropicBlock::Text;
                    }
                    "thinking" => {
                        pending.push_back(Ok(ChatEvent::ThinkingStart));
                        *current_block = AnthropicBlock::Thinking;
                    }
                    "tool_use" => {
                        let id = v["content_block"]["id"].as_str().unwrap_or("").to_owned();
                        let name = v["content_block"]["name"].as_str().unwrap_or("").to_owned();
                        pending.push_back(Ok(ChatEvent::ToolCallStart {
                            id: id.clone(),
                            name: name.clone(),
                        }));
                        *current_block = AnthropicBlock::ToolCall {
                            id,
                            name,
                            args_buf: String::new(),
                        };
                    }
                    _ => {}
                }
            }

            Some("content_block_delta") => {
                let delta = &v["delta"];
                let delta_type = delta["type"].as_str().unwrap_or("");

                match delta_type {
                    "text_delta" => {
                        if let Some(text) = delta["text"].as_str() {
                            pending.push_back(Ok(ChatEvent::TextDelta {
                                text: text.to_owned(),
                            }));
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta["thinking"].as_str() {
                            pending.push_back(Ok(ChatEvent::ThinkingDelta {
                                text: text.to_owned(),
                            }));
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta["partial_json"].as_str() {
                            if let AnthropicBlock::ToolCall { id, name, args_buf } = current_block {
                                args_buf.push_str(partial);
                                pending.push_back(Ok(ChatEvent::ToolCallDelta {
                                    id: id.clone(),
                                    name: name.clone(),
                                    arguments_delta: partial.to_owned(),
                                }));
                            }
                        }
                    }
                    _ => {}
                }
            }

            Some("content_block_stop") => {
                // Close current block.
                match current_block {
                    AnthropicBlock::None => {}
                    AnthropicBlock::Text => {
                        pending.push_back(Ok(ChatEvent::TextEnd));
                        *current_block = AnthropicBlock::None;
                    }
                    AnthropicBlock::Thinking => {
                        pending.push_back(Ok(ChatEvent::ThinkingEnd));
                        *current_block = AnthropicBlock::None;
                    }
                    AnthropicBlock::ToolCall { id, name, args_buf } => {
                        let arguments: Value =
                            serde_json::from_str(args_buf).unwrap_or(Value::Null);
                        pending.push_back(Ok(ChatEvent::ToolCallDone {
                            id: id.clone(),
                            name: name.clone(),
                            arguments,
                        }));
                        *current_block = AnthropicBlock::None;
                    }
                }
            }

            Some("message_delta") => {
                // Stop reason + output usage.
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    *stop_reason = Some(match reason {
                        "end_turn" => StopReason::Stop,
                        "max_tokens" => StopReason::Length,
                        "tool_use" => StopReason::ToolUse,
                        _ => StopReason::Stop,
                    });
                }
                if let Some(u) = v["usage"].as_object() {
                    usage.output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }

            Some("message_stop") | None => {}
            _ => {} // ignore unknown event types
        }
    }

    fn build_done(
        stop_reason: StopReason,
        usage: &Usage,
        input_cost: f64,
        output_cost: f64,
        cache_read_cost: f64,
        cache_write_cost: f64,
    ) -> ChatEvent {
        let cost_in = (usage.input as f64 / 1_000_000.0) * input_cost;
        let cost_out = (usage.output as f64 / 1_000_000.0) * output_cost;
        let cost_cr = (usage.cache_read as f64 / 1_000_000.0) * cache_read_cost;
        let cost_cw = (usage.cache_write as f64 / 1_000_000.0) * cache_write_cost;
        let total = cost_in + cost_out + cost_cr + cost_cw;
        ChatEvent::Done {
            stop_reason,
            usage: Usage {
                total_tokens: usage.input + usage.output,
                ..usage.clone()
            },
            cost: Cost {
                input: cost_in,
                output: cost_out,
                cache_read: cost_cr,
                cache_write: cost_cw,
                total,
            },
        }
    }
}

impl<S> futures::Stream for AnthropicStreamParser<S>
where
    S: futures::Stream<Item = Result<crate::sse::SseEvent, crate::sse::SseError>>,
{
    type Item = Result<ChatEvent, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            if let Some(ev) = this.pending.pop_front() {
                return Poll::Ready(Some(ev));
            }

            if *this.done {
                return Poll::Ready(None);
            }

            match this.inner.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    // Stream ended.
                    let stop_reason = this.stop_reason.take().unwrap_or(StopReason::Stop);
                    let done_event = Self::build_done(
                        stop_reason,
                        this.usage,
                        *this.input_cost,
                        *this.output_cost,
                        *this.cache_read_cost,
                        *this.cache_write_cost,
                    );
                    this.pending.push_back(Ok(done_event));
                    *this.done = true;
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(ProviderError::Sse(e.to_string()))));
                }
                Poll::Ready(Some(Ok(sse_event))) => {
                    Self::process_event(
                        sse_event.event_type.as_deref(),
                        &sse_event.data,
                        this.current_block,
                        this.pending,
                        this.stop_reason,
                        this.usage,
                    );
                    continue;
                }
            }
        }
    }
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

    // ── messages_to_anthropic ────────────────────────────────────────────────

    #[test]
    fn user_plain_text() {
        let messages = vec![Message::User {
            content: vec![text_block("hello")],
        }];
        let result = messages_to_anthropic(&messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[0]["content"], "hello");
    }

    #[test]
    fn assistant_with_text() {
        let messages = vec![Message::Assistant {
            content: vec![text_block("I'll help")],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_anthropic(&messages);
        assert_eq!(result[0]["role"], "assistant");
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "I'll help");
    }

    #[test]
    fn assistant_with_tool_call() {
        let messages = vec![Message::Assistant {
            content: vec![tool_block("tc-1", "bash", json!({"cmd": "ls"}))],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_anthropic(&messages);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_use");
        assert_eq!(blocks[0]["id"], "tc-1");
        assert_eq!(blocks[0]["name"], "bash");
        assert_eq!(blocks[0]["input"]["cmd"], "ls");
    }

    #[test]
    fn single_tool_result() {
        let messages = vec![Message::ToolResult {
            tool_call_id: "tc-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![text_block("output")],
            is_error: false,
        }];
        let result = messages_to_anthropic(&messages);
        assert_eq!(result[0]["role"], "user");
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "tc-1");
        assert_eq!(blocks[0]["content"], "output");
    }

    #[test]
    fn multiple_tool_results_collapsed() {
        let messages = vec![
            Message::ToolResult {
                tool_call_id: "tc-1".to_owned(),
                tool_name: "bash".to_owned(),
                content: vec![text_block("result1")],
                is_error: false,
            },
            Message::ToolResult {
                tool_call_id: "tc-2".to_owned(),
                tool_name: "read_file".to_owned(),
                content: vec![text_block("result2")],
                is_error: false,
            },
        ];
        let result = messages_to_anthropic(&messages);
        // Both tool results should be collapsed into a single user message.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["tool_use_id"], "tc-1");
        assert_eq!(blocks[1]["tool_use_id"], "tc-2");
    }

    #[test]
    fn tool_result_is_error_flag() {
        let messages = vec![Message::ToolResult {
            tool_call_id: "tc-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![text_block("error text")],
            is_error: true,
        }];
        let result = messages_to_anthropic(&messages);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["is_error"], true);
    }

    #[test]
    fn tools_to_anthropic_structure() {
        let tools = vec![ToolDefinition {
            name: "read_file".to_owned(),
            description: "Read a file".to_owned(),
            parameters: json!({ "type": "object", "properties": {} }),
        }];
        let result = tools_to_anthropic(&tools);
        assert_eq!(result[0]["name"], "read_file");
        assert_eq!(result[0]["description"], "Read a file");
        assert!(result[0]["input_schema"].is_object());
    }

    #[test]
    fn image_block_in_user_message() {
        let messages = vec![Message::User {
            content: vec![
                text_block("look at this"),
                ContentBlock::Image {
                    data: "base64data".to_owned(),
                    mime_type: "image/jpeg".to_owned(),
                },
            ],
        }];
        let result = messages_to_anthropic(&messages);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/jpeg");
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn thinking_block_in_assistant_is_included() {
        let messages = vec![Message::Assistant {
            content: vec![
                ContentBlock::Thinking {
                    thinking: "Let me reason...".to_owned(),
                    signature: Some("sig123".to_owned()),
                    redacted: false,
                },
                text_block("My answer"),
            ],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_anthropic(&messages);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(
            blocks.len(),
            2,
            "Thinking block should be included in assistant content"
        );
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["thinking"], "Let me reason...");
        assert_eq!(blocks[0]["signature"], "sig123");
        assert_eq!(blocks[1]["type"], "text");
    }

    #[test]
    fn redacted_thinking_block_in_assistant() {
        let messages = vec![Message::Assistant {
            content: vec![ContentBlock::Thinking {
                thinking: String::new(),
                signature: None,
                redacted: true,
            }],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_anthropic(&messages);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(
            blocks.len(),
            1,
            "Redacted thinking block should be included"
        );
        assert_eq!(
            blocks[0]["type"], "redacted_thinking",
            "Redacted thinking should use 'redacted_thinking' type"
        );
        // Should NOT have thinking/signature fields
        assert!(
            blocks[0].get("thinking").is_none() || blocks[0]["thinking"].is_null(),
            "Redacted thinking should not expose thinking content"
        );
    }

    #[test]
    fn tool_result_with_is_error_true() {
        let messages = vec![Message::ToolResult {
            tool_call_id: "tc-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![text_block("command not found")],
            is_error: true,
        }];
        let result = messages_to_anthropic(&messages);
        let blocks = result[0]["content"].as_array().unwrap();
        assert_eq!(
            blocks[0]["is_error"], true,
            "ToolResult with is_error=true should set is_error field"
        );
        assert_eq!(blocks[0]["content"], "command not found");
    }

    #[test]
    fn empty_system_prompt_handling() {
        // Empty system prompt — the provider code sends it wrapped in array.
        // This test verifies messages_to_anthropic works; the system field is
        // handled separately in the provider, not in messages_to_anthropic.
        // We verify messages_to_anthropic doesn't crash on edge inputs.
        let messages = vec![Message::User {
            content: vec![text_block("hello")],
        }];
        let result = messages_to_anthropic(&messages);
        assert_eq!(result.len(), 1);
        // An empty system prompt string in the provider should still build a
        // JSON system array. We verify the conversion at message level here.
        assert_eq!(result[0]["content"], "hello");
    }

    #[test]
    fn multiple_consecutive_user_messages() {
        // No assistant message between two user messages — both should be preserved
        let messages = vec![
            Message::User {
                content: vec![text_block("first user message")],
            },
            Message::User {
                content: vec![text_block("second user message (no assistant between)")],
            },
        ];
        let result = messages_to_anthropic(&messages);
        assert_eq!(
            result.len(),
            2,
            "Two consecutive user messages should produce 2 entries"
        );
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[1]["role"], "user");
        assert_eq!(result[0]["content"], "first user message");
        assert_eq!(
            result[1]["content"],
            "second user message (no assistant between)"
        );
    }
}
