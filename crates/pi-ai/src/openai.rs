//! OpenAI Chat Completions provider (used by GitHub Copilot).

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use crate::provider::{ChatStream, LlmProvider, ProviderError};
use crate::sse::SseStream;
use crate::types::*;

// ─── Provider struct ─────────────────────────────────────────────────────────

pub struct OpenAiCompletionsProvider {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiCompletionsProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

// ─── Message conversion ──────────────────────────────────────────────────────

/// Convert our internal `Message` slice into the OpenAI wire format.
pub fn messages_to_openai(messages: &[Message], system_prompt: Option<&str>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();

    if let Some(sys) = system_prompt {
        out.push(json!({ "role": "system", "content": sys }));
    }

    for msg in messages {
        match msg {
            Message::User { content } => {
                // Build a content array supporting text and image blocks.
                let parts: Vec<Value> = content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => {
                            Some(json!({ "type": "text", "text": text }))
                        }
                        ContentBlock::Image { data, mime_type } => Some(json!({
                            "type": "image_url",
                            "image_url": { "url": format!("data:{mime_type};base64,{data}") }
                        })),
                        _ => None,
                    })
                    .collect();

                let content_value = if parts.len() == 1 {
                    if let Some(Value::Object(ref o)) = parts.first() {
                        if let Some(Value::String(t)) = o.get("text") {
                            // Plain text — send as a string for backwards compat.
                            out.push(json!({ "role": "user", "content": t }));
                            continue;
                        }
                    }
                    Value::Array(parts)
                } else {
                    Value::Array(parts)
                };

                out.push(json!({ "role": "user", "content": content_value }));
            }

            Message::Assistant { content, .. } => {
                let text: String = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");

                let tool_calls: Vec<Value> = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": arguments.to_string()
                            }
                        })),
                        _ => None,
                    })
                    .collect();

                let mut obj = json!({ "role": "assistant", "content": text });
                if !tool_calls.is_empty() {
                    obj["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(obj);
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
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": text
                }));
            }
        }
    }

    out
}

/// Convert our `ToolDefinition` list to the OpenAI wire format.
fn tools_to_openai(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters
                }
            })
        })
        .collect()
}

// ─── Stream state machine ────────────────────────────────────────────────────

#[derive(Debug)]
enum CurrentBlock {
    None,
    Text,
    Thinking,
    ToolCall {
        id: String,
        name: String,
        args_buf: String,
    },
}

// ─── LlmProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OpenAiCompletionsProvider {
    fn name(&self) -> &str {
        "github-copilot"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatStream, ProviderError> {
        let base_url = &request.model.base_url;
        let url = format!("{base_url}/chat/completions");

        // Determine initiator based on the last message role.
        let initiator = match request.messages.last() {
            Some(Message::User { .. }) => "user",
            _ => "agent",
        };

        let messages = messages_to_openai(&request.messages, request.system_prompt.as_deref());
        let max_tokens = request.max_tokens.unwrap_or(request.model.max_tokens);

        let mut body = json!({
            "model": request.model.id,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
            "max_completion_tokens": max_tokens,
        });

        if !request.tools.is_empty() {
            body["tools"] = Value::Array(tools_to_openai(&request.tools));
            body["tool_choice"] = json!("auto");
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
            let events = parse_sse_chunk(sse_result, input_cost, output_cost);
            futures::stream::iter(events)
        });

        // Wrap in state machine to emit Start/End block events.
        let stateful = BlockStateMachine::new(stream);

        Ok(Box::pin(stateful))
    }
}

// ─── SSE chunk parser ────────────────────────────────────────────────────────

fn parse_sse_chunk(
    result: Result<crate::sse::SseEvent, crate::sse::SseError>,
    input_cost_per_mtok: f64,
    output_cost_per_mtok: f64,
) -> Vec<Result<RawChatEvent, ProviderError>> {
    let event = match result {
        Ok(e) => e,
        Err(e) => return vec![Err(ProviderError::Sse(e.to_string()))],
    };

    let v: Value = match serde_json::from_str(&event.data) {
        Ok(v) => v,
        Err(e) => return vec![Err(ProviderError::Json(e))],
    };

    let mut events = Vec::new();

    // Usage chunk (may appear alongside choices or alone).
    if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
        let input = usage["prompt_tokens"].as_u64().unwrap_or(0);
        let output = usage["completion_tokens"].as_u64().unwrap_or(0);
        let usage_obj = Usage {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            total_tokens: input + output,
        };
        let cost = Cost {
            input: (input as f64 / 1_000_000.0) * input_cost_per_mtok,
            output: (output as f64 / 1_000_000.0) * output_cost_per_mtok,
            cache_read: 0.0,
            cache_write: 0.0,
            total: 0.0,
        };
        let cost = Cost {
            total: cost.input + cost.output,
            ..cost
        };

        // Check if choices is present and non-empty; if not, this is the final usage-only chunk.
        let choices_empty = v
            .get("choices")
            .and_then(|c| c.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true);

        if choices_empty {
            events.push(Ok(RawChatEvent::Done {
                stop_reason: StopReason::Stop,
                usage: usage_obj,
                cost,
            }));
            return events;
        }
    }

    let choices = match v["choices"].as_array() {
        Some(c) if !c.is_empty() => c,
        _ => return events,
    };

    let choice = &choices[0];
    let delta = &choice["delta"];

    // Finish reason
    if let Some(reason_str) = choice["finish_reason"].as_str() {
        if !reason_str.is_empty() && reason_str != "null" {
            let stop_reason = match reason_str {
                "stop" => StopReason::Stop,
                "length" => StopReason::Length,
                "tool_calls" => StopReason::ToolUse,
                _ => StopReason::Stop,
            };
            events.push(Ok(RawChatEvent::FinishReason(stop_reason)));
        }
    }

    // Thinking / reasoning
    if let Some(thinking) = delta["reasoning_content"]
        .as_str()
        .or_else(|| delta["reasoning"].as_str())
    {
        if !thinking.is_empty() {
            events.push(Ok(RawChatEvent::ThinkingDelta {
                text: thinking.to_owned(),
            }));
        }
    }

    // Text content
    if let Some(text) = delta["content"].as_str() {
        if !text.is_empty() {
            events.push(Ok(RawChatEvent::TextDelta {
                text: text.to_owned(),
            }));
        }
    }

    // Tool calls
    if let Some(tool_calls) = delta["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_owned();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_owned();
            let args_delta = tc["function"]["arguments"]
                .as_str()
                .unwrap_or("")
                .to_owned();

            if !id.is_empty() || !name.is_empty() {
                events.push(Ok(RawChatEvent::ToolCallHeader { id, name }));
            }
            if !args_delta.is_empty() {
                events.push(Ok(RawChatEvent::ToolCallArgsDelta { args_delta }));
            }
        }
    }

    events
}

// ─── Raw event (before block-start/end wrapping) ────────────────────────────

#[derive(Debug)]
enum RawChatEvent {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolCallHeader {
        id: String,
        name: String,
    },
    ToolCallArgsDelta {
        args_delta: String,
    },
    FinishReason(StopReason),
    Done {
        stop_reason: StopReason,
        usage: Usage,
        cost: Cost,
    },
}

// ─── Block state machine ─────────────────────────────────────────────────────
//
// Wraps the raw stream and emits proper Start/End events around content blocks.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use pin_project_lite::pin_project;

pin_project! {
    struct BlockStateMachine<S> {
        #[pin]
        inner: S,
        current: CurrentBlock,
        pending: VecDeque<Result<ChatEvent, ProviderError>>,
        pending_stop: Option<StopReason>,
        usage: Option<Usage>,
        cost: Option<Cost>,
        done: bool,
    }
}

impl<S> BlockStateMachine<S>
where
    S: futures::Stream<Item = Result<RawChatEvent, ProviderError>>,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            current: CurrentBlock::None,
            pending: VecDeque::new(),
            pending_stop: None,
            usage: None,
            cost: None,
            done: false,
        }
    }

    /// Close the current block (emit End event) and transition.
    fn close_current(
        current: &mut CurrentBlock,
        pending: &mut VecDeque<Result<ChatEvent, ProviderError>>,
    ) {
        match current {
            CurrentBlock::None => {}
            CurrentBlock::Text => {
                pending.push_back(Ok(ChatEvent::TextEnd));
                *current = CurrentBlock::None;
            }
            CurrentBlock::Thinking => {
                pending.push_back(Ok(ChatEvent::ThinkingEnd));
                *current = CurrentBlock::None;
            }
            CurrentBlock::ToolCall { id, name, args_buf } => {
                // Emit ToolCallDone with parsed arguments.
                let arguments: serde_json::Value =
                    serde_json::from_str(args_buf).unwrap_or(serde_json::Value::Null);
                pending.push_back(Ok(ChatEvent::ToolCallDone {
                    id: id.clone(),
                    name: name.clone(),
                    arguments,
                }));
                *current = CurrentBlock::None;
            }
        }
    }

    fn handle_raw(
        current: &mut CurrentBlock,
        pending: &mut VecDeque<Result<ChatEvent, ProviderError>>,
        pending_stop: &mut Option<StopReason>,
        usage: &mut Option<Usage>,
        cost: &mut Option<Cost>,
        raw: RawChatEvent,
    ) {
        match raw {
            RawChatEvent::TextDelta { text } => {
                // Ensure we're in a text block.
                match current {
                    CurrentBlock::Text => {}
                    _ => {
                        Self::close_current(current, pending);
                        pending.push_back(Ok(ChatEvent::TextStart));
                        *current = CurrentBlock::Text;
                    }
                }
                pending.push_back(Ok(ChatEvent::TextDelta { text }));
            }

            RawChatEvent::ThinkingDelta { text } => {
                match current {
                    CurrentBlock::Thinking => {}
                    _ => {
                        Self::close_current(current, pending);
                        pending.push_back(Ok(ChatEvent::ThinkingStart));
                        *current = CurrentBlock::Thinking;
                    }
                }
                pending.push_back(Ok(ChatEvent::ThinkingDelta { text }));
            }

            RawChatEvent::ToolCallHeader { id, name } => {
                // New tool call — close any existing block.
                match current {
                    CurrentBlock::ToolCall {
                        id: cur_id,
                        name: cur_name,
                        ..
                    } if cur_id == &id || id.is_empty() => {
                        // Continuation of the same tool call; headers may be empty
                        // on follow-up chunks.
                        let _ = (cur_id, cur_name); // suppress unused warning
                    }
                    _ => {
                        Self::close_current(current, pending);
                        let emit_id = if id.is_empty() {
                            uuid::Uuid::new_v4().to_string()
                        } else {
                            id.clone()
                        };
                        let emit_name = name.clone();
                        pending.push_back(Ok(ChatEvent::ToolCallStart {
                            id: emit_id.clone(),
                            name: emit_name.clone(),
                        }));
                        *current = CurrentBlock::ToolCall {
                            id: emit_id,
                            name: emit_name,
                            args_buf: String::new(),
                        };
                    }
                }
            }

            RawChatEvent::ToolCallArgsDelta { args_delta } => {
                if let CurrentBlock::ToolCall { id, name, args_buf } = current {
                    args_buf.push_str(&args_delta);
                    pending.push_back(Ok(ChatEvent::ToolCallDelta {
                        id: id.clone(),
                        name: name.clone(),
                        arguments_delta: args_delta,
                    }));
                }
            }

            RawChatEvent::FinishReason(reason) => {
                *pending_stop = Some(reason);
            }

            RawChatEvent::Done {
                stop_reason,
                usage: u,
                cost: c,
            } => {
                // Prefer a finish_reason from a previous chunk if available.
                if pending_stop.is_none() {
                    *pending_stop = Some(stop_reason);
                }
                *usage = Some(u);
                *cost = Some(c);
            }
        }
    }
}

impl<S> futures::Stream for BlockStateMachine<S>
where
    S: futures::Stream<Item = Result<RawChatEvent, ProviderError>>,
{
    type Item = Result<ChatEvent, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // Drain any buffered events first.
            if let Some(ev) = this.pending.pop_front() {
                return Poll::Ready(Some(ev));
            }

            if *this.done {
                return Poll::Ready(None);
            }

            // Pull from inner stream.
            match this.inner.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    // Stream ended — close any open block and emit Done.
                    Self::close_current(this.current, this.pending);

                    let stop_reason = this.pending_stop.take().unwrap_or(StopReason::Stop);
                    let usage = this.usage.take().unwrap_or_default();
                    let cost = this.cost.take().unwrap_or_default();
                    this.pending.push_back(Ok(ChatEvent::Done {
                        stop_reason,
                        usage,
                        cost,
                    }));
                    *this.done = true;
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(Some(Ok(raw))) => {
                    Self::handle_raw(
                        this.current,
                        this.pending,
                        this.pending_stop,
                        this.usage,
                        this.cost,
                        raw,
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

    // ── message_to_openai ────────────────────────────────────────────────────

    #[test]
    fn user_message_plain_text() {
        let messages = vec![Message::User {
            content: vec![text_block("hello")],
        }];
        let result = messages_to_openai(&messages, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[0]["content"], "hello");
    }

    #[test]
    fn system_prompt_is_first() {
        let messages = vec![Message::User {
            content: vec![text_block("hi")],
        }];
        let result = messages_to_openai(&messages, Some("Be helpful"));
        assert_eq!(result[0]["role"], "system");
        assert_eq!(result[0]["content"], "Be helpful");
        assert_eq!(result[1]["role"], "user");
    }

    #[test]
    fn assistant_message_with_text() {
        let messages = vec![Message::Assistant {
            content: vec![text_block("sure")],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_openai(&messages, None);
        assert_eq!(result[0]["role"], "assistant");
        assert_eq!(result[0]["content"], "sure");
    }

    #[test]
    fn assistant_message_with_tool_call() {
        let messages = vec![Message::Assistant {
            content: vec![tool_block("tc-1", "bash", json!({"cmd": "ls"}))],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_openai(&messages, None);
        let tc = &result[0]["tool_calls"][0];
        assert_eq!(tc["id"], "tc-1");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "bash");
    }

    #[test]
    fn tool_result_message() {
        let messages = vec![Message::ToolResult {
            tool_call_id: "tc-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![text_block("file1\nfile2")],
            is_error: false,
        }];
        let result = messages_to_openai(&messages, None);
        assert_eq!(result[0]["role"], "tool");
        assert_eq!(result[0]["tool_call_id"], "tc-1");
        assert_eq!(result[0]["content"], "file1\nfile2");
    }

    #[test]
    fn tools_to_openai_structure() {
        let tools = vec![ToolDefinition {
            name: "read_file".to_owned(),
            description: "Read a file".to_owned(),
            parameters: json!({ "type": "object", "properties": {} }),
        }];
        let result = tools_to_openai(&tools);
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["function"]["name"], "read_file");
        assert_eq!(result[0]["function"]["description"], "Read a file");
    }

    #[test]
    fn image_in_user_message() {
        let messages = vec![Message::User {
            content: vec![
                text_block("describe this"),
                ContentBlock::Image {
                    data: "abc123".to_owned(),
                    mime_type: "image/png".to_owned(),
                },
            ],
        }];
        let result = messages_to_openai(&messages, None);
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn empty_message_list_produces_only_system_if_present() {
        let result = messages_to_openai(&[], Some("Be helpful"));
        assert_eq!(
            result.len(),
            1,
            "Empty messages with system prompt should produce 1 entry"
        );
        assert_eq!(result[0]["role"], "system");

        let result_no_sys = messages_to_openai(&[], None);
        assert_eq!(
            result_no_sys.len(),
            0,
            "Empty messages with no system prompt should produce 0 entries"
        );
    }

    #[test]
    fn assistant_message_with_empty_content_vec() {
        // An assistant message with zero content blocks — should still produce a message
        let messages = vec![Message::Assistant {
            content: vec![],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_openai(&messages, None);
        assert_eq!(
            result.len(),
            1,
            "Assistant with empty content should still produce a message"
        );
        assert_eq!(result[0]["role"], "assistant");
        // Content should be empty string (no text blocks)
        assert_eq!(
            result[0]["content"], "",
            "Empty content vec should yield empty string, not null or missing"
        );
    }

    #[test]
    fn tool_result_with_empty_content() {
        // ToolResult with no content blocks — should still produce a tool message
        let messages = vec![Message::ToolResult {
            tool_call_id: "tc-1".to_owned(),
            tool_name: "bash".to_owned(),
            content: vec![],
            is_error: false,
        }];
        let result = messages_to_openai(&messages, None);
        assert_eq!(
            result.len(),
            1,
            "ToolResult with empty content should still produce a message"
        );
        assert_eq!(result[0]["role"], "tool");
        assert_eq!(result[0]["tool_call_id"], "tc-1");
        // Content should be empty string
        assert_eq!(
            result[0]["content"], "",
            "ToolResult with no content blocks should yield empty string"
        );
    }

    #[test]
    fn user_message_with_text_and_image_content_blocks() {
        // When a user message has both text AND image, it should produce an array (not a string)
        let messages = vec![Message::User {
            content: vec![
                text_block("what is this?"),
                ContentBlock::Image {
                    data: "base64data".to_owned(),
                    mime_type: "image/jpeg".to_owned(),
                },
            ],
        }];
        let result = messages_to_openai(&messages, None);
        assert_eq!(result[0]["role"], "user");
        let content = result[0]["content"]
            .as_array()
            .expect("Mixed text+image user message should produce an array content");
        assert_eq!(content.len(), 2, "Should have 2 content blocks");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
    }

    #[test]
    fn very_long_tool_call_arguments_over_10kb() {
        // Arguments JSON > 10KB — should serialize correctly without truncation
        let large_value: String = "a".repeat(10 * 1024 + 1);
        let args = json!({ "data": large_value });
        let messages = vec![Message::Assistant {
            content: vec![tool_block("tc-big", "process", args.clone())],
            stop_reason: None,
            usage: None,
            error_message: None,
        }];
        let result = messages_to_openai(&messages, None);
        let tc = &result[0]["tool_calls"][0];
        let args_str = tc["function"]["arguments"]
            .as_str()
            .expect("arguments should be a string");
        // Re-parse to verify it's valid JSON and contains full data
        let reparsed: Value = serde_json::from_str(args_str)
            .expect("tool call arguments should be valid JSON after serialization");
        assert_eq!(
            reparsed["data"].as_str().unwrap().len(),
            10 * 1024 + 1,
            "Very long tool arguments should not be truncated"
        );
    }
}
