//! Context compaction — summarise old messages to keep the prompt within budget.

use std::sync::Arc;

use futures::StreamExt;

use pi_ai::{ChatEvent, ChatRequest, ChatStream, ContentBlock, LlmProvider, Message, Model};

use crate::hooks::{BoxFuture, TransformContextHook};

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for automatic context compaction.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Number of recent messages to keep untouched.
    pub preserve_count: usize,
    /// Trigger compaction when estimated tokens exceed this fraction of the
    /// available budget (`context_window - max_tokens`).
    pub threshold: f64,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            preserve_count: 10,
            threshold: 0.75,
        }
    }
}

// ─── Token estimation ─────────────────────────────────────────────────────────

/// Estimate the token count for a slice of messages using a chars/4 heuristic.
pub fn estimate_tokens(messages: &[Message]) -> u64 {
    messages.iter().map(estimate_message_tokens).sum()
}

fn estimate_message_tokens(msg: &Message) -> u64 {
    let chars: u64 = match msg {
        Message::User { content } | Message::Assistant { content, .. } => {
            content.iter().map(block_chars).sum()
        }
        Message::ToolResult {
            tool_name, content, ..
        } => tool_name.len() as u64 + content.iter().map(block_chars).sum::<u64>(),
    };
    // chars/4 heuristic, minimum 1 token per message.
    (chars / 4).max(1)
}

fn block_chars(block: &ContentBlock) -> u64 {
    match block {
        ContentBlock::Text { text } => text.len() as u64,
        ContentBlock::Thinking { thinking, .. } => thinking.len() as u64,
        ContentBlock::ToolCall {
            name, arguments, ..
        } => name.len() as u64 + arguments.to_string().len() as u64,
        ContentBlock::Image { data, .. } => data.len() as u64,
    }
}

// ─── Threshold detection ──────────────────────────────────────────────────────

/// Return `true` when the estimated token count exceeds the compaction threshold.
///
/// Budget = `context_window - max_tokens`; trigger at `budget * threshold`.
pub fn should_compact(messages: &[Message], model: &Model) -> bool {
    let estimated = estimate_tokens(messages);
    // Cap effective context window at 60K to account for provider-side limits
    // (e.g., Copilot may cap at 64K even if model advertises 128K)
    let effective_window = model.context_window.min(60_000);
    let budget = effective_window.saturating_sub(model.max_tokens.min(16_000));
    let trigger = (budget as f64 * 0.75) as u64;
    estimated > trigger
}

// ─── Summarization prompt ─────────────────────────────────────────────────────

fn build_summarization_prompt(messages: &[Message]) -> String {
    let mut conversation = String::new();
    for msg in messages {
        match msg {
            Message::User { content } => {
                conversation.push_str("User: ");
                for block in content {
                    if let ContentBlock::Text { text } = block {
                        conversation.push_str(text);
                    }
                }
                conversation.push('\n');
            }
            Message::Assistant { content, .. } => {
                conversation.push_str("Assistant: ");
                for block in content {
                    match block {
                        ContentBlock::Text { text } => conversation.push_str(text),
                        ContentBlock::ToolCall { name, .. } => {
                            conversation.push_str(&format!("[called tool: {name}]"));
                        }
                        _ => {}
                    }
                }
                conversation.push('\n');
            }
            Message::ToolResult {
                tool_name,
                content,
                is_error,
                ..
            } => {
                let status = if *is_error { "error" } else { "ok" };
                conversation.push_str(&format!("Tool {tool_name} ({status}): "));
                for block in content {
                    if let ContentBlock::Text { text } = block {
                        // Truncate long tool output so we don't bloat the summary prompt.
                        let truncated = if text.len() > 200 {
                            format!("{}... [truncated]", &text[..200])
                        } else {
                            text.clone()
                        };
                        conversation.push_str(&truncated);
                    }
                }
                conversation.push('\n');
            }
        }
    }

    format!(
        "Summarize this conversation concisely. Preserve:\n\
         - Key decisions made\n\
         - File paths mentioned and modified\n\
         - Current task state and progress\n\
         - Tool names used and their outcomes\n\
         - Any errors or blockers encountered\n\n\
         Keep it under 500 words.\n\n\
         Conversation:\n{conversation}"
    )
}

// ─── Fallback summary (no LLM) ────────────────────────────────────────────────

fn fallback_summary(messages: &[Message]) -> String {
    let mut summary = format!("Previous conversation had {} messages. ", messages.len());

    let mut tools: Vec<String> = Vec::new();
    for msg in messages {
        if let Message::ToolResult { tool_name, .. } = msg {
            if !tools.contains(tool_name) {
                tools.push(tool_name.clone());
            }
        }
    }
    if !tools.is_empty() {
        summary.push_str(&format!("Tools used: {}. ", tools.join(", ")));
    }

    summary
}

// ─── Stream collector ─────────────────────────────────────────────────────────

async fn collect_text_from_stream(stream: ChatStream) -> String {
    let mut stream = std::pin::pin!(stream);
    let mut text = String::new();
    while let Some(Ok(event)) = stream.next().await {
        if let ChatEvent::TextDelta { text: t } = event {
            text.push_str(&t);
        }
    }
    if text.is_empty() {
        "[Summary unavailable]".to_string()
    } else {
        text
    }
}

// ─── Core compaction logic ────────────────────────────────────────────────────

/// Compact `messages` by summarising the oldest ones via the LLM.
///
/// The last `config.preserve_count` messages are kept verbatim; everything
/// before that window is replaced with a single User-role summary message.
pub async fn compact_messages(
    messages: Vec<Message>,
    provider: &dyn LlmProvider,
    model: &Model,
    config: &CompactionConfig,
) -> Vec<Message> {
    if messages.len() <= config.preserve_count {
        return messages;
    }

    let split = messages.len() - config.preserve_count;
    let to_compact = &messages[..split];
    let to_keep = &messages[split..];

    // If the conversation is severely over budget (>2x), use fallback summary
    // without calling the LLM (the summarization call itself would fail).
    let budget = model.context_window.saturating_sub(model.max_tokens);
    let estimated = estimate_tokens(to_compact);
    let summary_text = if estimated > budget * 2 {
        tracing::warn!(
            "Context severely over budget ({estimated} >> {budget}), using fallback summary"
        );
        fallback_summary(to_compact)
    } else {
        let summary_prompt = build_summarization_prompt(to_compact);

        let request = ChatRequest {
            model: model.clone(),
            messages: vec![Message::User {
                content: vec![ContentBlock::Text {
                    text: summary_prompt,
                }],
            }],
            system_prompt: Some(
                "You are a conversation summarizer. Be concise and factual.".to_string(),
            ),
            tools: vec![],
            thinking_level: None,
            max_tokens: Some(2048),
            temperature: Some(0.3),
        };

        match provider.chat(request).await {
            Ok(stream) => collect_text_from_stream(stream).await,
            Err(e) => {
                tracing::warn!("Compaction failed (provider error): {e}. Using fallback.");
                fallback_summary(to_compact)
            }
        }
    };

    let before_count = messages.len();
    let token_before = estimate_tokens(&messages);

    // Build the compacted message list.
    let summary_message = Message::User {
        content: vec![ContentBlock::Text {
            text: format!("[Previous conversation summary]:\n{summary_text}"),
        }],
    };

    let mut result = vec![summary_message];
    result.extend_from_slice(to_keep);

    let after_count = result.len();
    let token_after = estimate_tokens(&result);

    tracing::info!(
        "Compaction: {before_count} → {after_count} messages, \
         ~{token_before} → ~{token_after} tokens"
    );

    // Recursive guard: if still over budget, truncate the summary text.
    if should_compact(&result, model) {
        tracing::warn!("Context still too large after compaction, truncating summary");
        if let Some(Message::User { content }) = result.first_mut() {
            if let Some(ContentBlock::Text { text }) = content.first_mut() {
                let budget_chars =
                    (model.context_window.saturating_sub(model.max_tokens) / 2 * 4) as usize;
                if text.len() > budget_chars {
                    text.truncate(budget_chars);
                    text.push_str("\n[summary truncated]");
                }
            }
        }
    }

    result
}

// ─── Hook factory ─────────────────────────────────────────────────────────────

/// Create a [`TransformContextHook`] that performs automatic compaction.
pub fn make_compaction_hook(
    provider: Arc<dyn LlmProvider>,
    model: Model,
    config: CompactionConfig,
) -> TransformContextHook {
    Box::new(move |messages: Vec<Message>| -> BoxFuture<Vec<Message>> {
        let provider = Arc::clone(&provider);
        let model = model.clone();
        let config = config.clone();
        Box::pin(async move {
            if should_compact(&messages, &model) {
                compact_messages(messages, provider.as_ref(), &model, &config).await
            } else {
                messages
            }
        })
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures::stream;

    use pi_ai::{
        ApiType, ChatEvent, ChatRequest, ChatStream, ContentBlock, Message, Model, ModelCost,
        ProviderError, StopReason, Usage,
    };

    use super::*;

    // ─── Helpers ──────────────────────────────────────────────────────────

    fn test_model_with_windows(context_window: u64, max_tokens: u64) -> Model {
        Model {
            id: "test".to_string(),
            name: "Test".to_string(),
            provider: "mock".to_string(),
            api: ApiType::AnthropicMessages,
            base_url: String::new(),
            reasoning: false,
            input_types: vec![],
            cost: ModelCost::default(),
            context_window,
            max_tokens,
            headers: Default::default(),
        }
    }

    fn text_user(text: &str) -> Message {
        Message::User {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    fn text_tool_result(tool_name: &str, text: &str) -> Message {
        Message::ToolResult {
            tool_call_id: "tc1".to_string(),
            tool_name: tool_name.to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            is_error: false,
        }
    }

    // ─── Mock provider ────────────────────────────────────────────────────

    struct MockSummaryProvider {
        summary: String,
    }

    #[async_trait]
    impl LlmProvider for MockSummaryProvider {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatStream, ProviderError> {
            let events: Vec<Result<ChatEvent, ProviderError>> = vec![
                Ok(ChatEvent::Start),
                Ok(ChatEvent::TextStart),
                Ok(ChatEvent::TextDelta {
                    text: self.summary.clone(),
                }),
                Ok(ChatEvent::TextEnd),
                Ok(ChatEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: Usage::default(),
                    cost: pi_ai::Cost::default(),
                }),
            ];
            Ok(Box::pin(stream::iter(events)))
        }

        fn name(&self) -> &str {
            "mock-summary"
        }
    }

    // ─── Test 1: estimate_tokens_empty ────────────────────────────────────

    #[test]
    fn estimate_tokens_empty() {
        assert_eq!(estimate_tokens(&[]), 0);
    }

    // ─── Test 2: estimate_tokens_single_message ───────────────────────────

    #[test]
    fn estimate_tokens_single_message() {
        // "hello world" = 11 chars → 11/4 = 2 tokens
        let msg = text_user("hello world");
        assert_eq!(estimate_tokens(&[msg]), 2);
    }

    // ─── Test 3: estimate_tokens_minimum ─────────────────────────────────

    #[test]
    fn estimate_tokens_minimum() {
        // Empty text → 0 chars → 0/4 = 0, but minimum is 1
        let msg = text_user("");
        assert_eq!(estimate_tokens(&[msg]), 1);
    }

    // ─── Test 4: should_compact_under_threshold ───────────────────────────

    #[test]
    fn should_compact_under_threshold() {
        // 10 tiny messages, big context window → well below threshold
        let model = test_model_with_windows(128_000, 16_384);
        let messages: Vec<Message> = (0..10).map(|_| text_user("hi")).collect();
        assert!(!should_compact(&messages, &model));
    }

    // ─── Test 5: should_compact_over_threshold ────────────────────────────

    #[test]
    fn should_compact_over_threshold() {
        // Budget = 1000 - 100 = 900; threshold = 675 tokens.
        // Each message = 400 chars of text = 100 tokens.
        // 7 messages = 700 tokens > 675.
        let model = test_model_with_windows(1000, 100);
        let big_text = "a".repeat(400); // 400 chars → 100 tokens
        let messages: Vec<Message> = (0..7).map(|_| text_user(&big_text)).collect();
        assert!(should_compact(&messages, &model));
    }

    // ─── Test 6: build_summarization_prompt_includes_tools ───────────────

    #[test]
    fn build_summarization_prompt_includes_tools() {
        let messages = vec![
            text_user("run a tool"),
            text_tool_result("read_file", "file contents here"),
        ];
        let prompt = build_summarization_prompt(&messages);
        assert!(
            prompt.contains("read_file"),
            "Prompt should mention the tool name"
        );
    }

    // ─── Test 7: build_summarization_prompt_truncates_long_output ────────

    #[test]
    fn build_summarization_prompt_truncates_long_output() {
        let long_output = "x".repeat(500);
        let messages = vec![text_tool_result("big_tool", &long_output)];
        let prompt = build_summarization_prompt(&messages);
        assert!(
            prompt.contains("[truncated]"),
            "Long tool output should be truncated in the prompt"
        );
    }

    // ─── Test 8: fallback_summary_includes_tool_names ────────────────────

    #[test]
    fn fallback_summary_includes_tool_names() {
        let messages = vec![
            text_user("do something"),
            text_tool_result("bash", "output"),
            text_tool_result("read_file", "contents"),
        ];
        let summary = fallback_summary(&messages);
        assert!(summary.contains("bash"), "Fallback should mention bash");
        assert!(
            summary.contains("read_file"),
            "Fallback should mention read_file"
        );
    }

    // ─── Test 9: compact_preserves_recent ────────────────────────────────

    #[tokio::test]
    async fn compact_preserves_recent() {
        let provider = Arc::new(MockSummaryProvider {
            summary: "This is a mock summary of the old messages.".to_string(),
        });
        let model = test_model_with_windows(1_000_000, 1024);
        let config = CompactionConfig {
            preserve_count: 3,
            threshold: 0.75,
        };

        // Build 10 messages with distinguishable text.
        let messages: Vec<Message> = (0..10)
            .map(|i| text_user(&format!("message number {i}")))
            .collect();

        let result = compact_messages(messages.clone(), provider.as_ref(), &model, &config).await;

        // Should have: 1 summary + 3 preserved = 4 messages.
        assert_eq!(result.len(), 4, "Expected summary + 3 preserved messages");

        // The last 3 original messages must appear verbatim at the end.
        for (expected, actual) in messages[7..].iter().zip(result[1..].iter()) {
            assert_eq!(
                format!("{expected:?}"),
                format!("{actual:?}"),
                "Preserved messages should be identical to the originals"
            );
        }

        // The first message must be the summary.
        if let Message::User { content } = &result[0] {
            if let Some(ContentBlock::Text { text }) = content.first() {
                assert!(
                    text.starts_with("[Previous conversation summary]:"),
                    "Summary message must start with the required prefix"
                );
            } else {
                panic!("Summary message has no text content");
            }
        } else {
            panic!("Summary message should be a User message");
        }
    }
}
