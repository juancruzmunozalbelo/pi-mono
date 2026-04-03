//! Pi agent — Agentic loop and conversation orchestration.

pub mod agent_loop;
pub mod event;
pub mod hooks;
pub mod state;

pub use event::AgentEvent;
pub use hooks::{
    AfterToolCallContext, AfterToolCallResult, BeforeToolCallContext, BeforeToolCallResult, Hooks,
};
pub use state::{Agent, AgentConfig, AgentState, MessageQueue, ToolExecutionMode};

/// Errors returned by the agent.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Provider error: {0}")]
    Provider(#[from] pi_ai::ProviderError),
    #[error("Already running")]
    AlreadyRunning,
    #[error("Aborted")]
    Aborted,
    #[error("{0}")]
    Other(String),
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures::stream;
    use tokio_util::sync::CancellationToken;

    use pi_ai::{
        ApiType, ChatEvent, ChatRequest, ChatStream, ContentBlock, Message, Model, ModelCost,
        ProviderError, StopReason, ThinkingLevel, Usage,
    };
    use pi_tools::{ToolContent, ToolResult};

    use super::*;

    // ─── Mock provider ───────────────────────────────────────────────────

    struct MockProvider {
        /// Each element is one "provider response" (a sequence of ChatEvents).
        /// Responses are consumed in order across multiple `chat()` calls.
        responses: std::sync::Mutex<Vec<Vec<ChatEvent>>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Vec<ChatEvent>>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl pi_ai::LlmProvider for MockProvider {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatStream, ProviderError> {
            let events = {
                let mut guard = self.responses.lock().unwrap();
                if guard.is_empty() {
                    return Err(ProviderError::Other("No more mock responses".to_string()));
                }
                guard.remove(0)
            };
            let mapped: Vec<Result<ChatEvent, ProviderError>> =
                events.into_iter().map(Ok).collect();
            Ok(Box::pin(stream::iter(mapped)))
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    // ─── Mock tool ───────────────────────────────────────────────────────

    struct EchoTool {
        name: &'static str,
    }

    #[async_trait]
    impl pi_tools::Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "Echo tool"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _cancel: CancellationToken,
        ) -> ToolResult {
            ToolResult {
                content: vec![ToolContent::Text {
                    text: format!("echo: {params}"),
                }],
                is_error: false,
            }
        }
    }

    // ─── Helpers ─────────────────────────────────────────────────────────

    fn test_model() -> Model {
        Model {
            id: "test".to_string(),
            name: "Test".to_string(),
            provider: "mock".to_string(),
            api: ApiType::AnthropicMessages,
            base_url: String::new(),
            reasoning: false,
            input_types: vec![],
            cost: ModelCost::default(),
            context_window: 4096,
            max_tokens: 1024,
            headers: Default::default(),
        }
    }

    fn done_events(stop: StopReason) -> Vec<ChatEvent> {
        vec![
            ChatEvent::Start,
            ChatEvent::TextStart,
            ChatEvent::TextDelta {
                text: "Hello!".to_string(),
            },
            ChatEvent::TextEnd,
            ChatEvent::Done {
                stop_reason: stop,
                usage: Usage::default(),
                cost: pi_ai::Cost::default(),
            },
        ]
    }

    fn tool_call_events(id: &str, name: &str, args: &str) -> Vec<ChatEvent> {
        vec![
            ChatEvent::Start,
            ChatEvent::ToolCallStart {
                id: id.to_string(),
                name: name.to_string(),
            },
            ChatEvent::ToolCallDelta {
                id: id.to_string(),
                name: name.to_string(),
                arguments_delta: args.to_string(),
            },
            ChatEvent::ToolCallDone {
                id: id.to_string(),
                name: name.to_string(),
                arguments: serde_json::from_str(args).unwrap_or(serde_json::Value::Null),
            },
            ChatEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
                cost: pi_ai::Cost::default(),
            },
        ]
    }

    fn make_agent(
        responses: Vec<Vec<ChatEvent>>,
        tools: Vec<Arc<dyn pi_tools::Tool>>,
        mode: ToolExecutionMode,
        hooks: Hooks,
    ) -> Agent {
        let provider = Arc::new(MockProvider::new(responses));
        let config = AgentConfig {
            provider,
            tools,
            tool_execution_mode: mode,
            hooks,
        };
        let state = AgentState {
            messages: vec![],
            model: test_model(),
            system_prompt: None,
            thinking_level: None,
            is_streaming: false,
            error_message: None,
        };
        Agent::new(config, state)
    }

    // ─── Test 1: Simple text response (no tools) ─────────────────────────

    #[tokio::test]
    async fn test_simple_text_response() {
        let mut agent = make_agent(
            vec![done_events(StopReason::Stop)],
            vec![],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("Hi".to_string()).await;
        assert!(result.is_ok(), "prompt should succeed: {result:?}");

        // Collect events
        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let has_agent_end = events
            .iter()
            .any(|e| matches!(e, AgentEvent::AgentEnd { .. }));
        assert!(has_agent_end, "Should have AgentEnd event");

        let state = agent.state().await;
        assert_eq!(state.messages.len(), 2); // user + assistant
    }

    // ─── Test 2: Single tool call → result → final response ──────────────

    #[tokio::test]
    async fn test_single_tool_call() {
        let tool_events = tool_call_events("tc1", "echo", r#"{"x":1}"#);
        let follow_events = done_events(StopReason::Stop);

        let mut agent = make_agent(
            vec![tool_events, follow_events],
            vec![Arc::new(EchoTool { name: "echo" })],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("call echo".to_string()).await;
        assert!(result.is_ok(), "prompt should succeed: {result:?}");

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let tool_end = events.iter().any(
            |e| matches!(e, AgentEvent::ToolExecutionEnd { tool_name, .. } if tool_name == "echo"),
        );
        assert!(tool_end, "Should have ToolExecutionEnd for echo");

        let state = agent.state().await;
        // user + assistant(tool call) + tool result + assistant(final)
        assert!(state.messages.len() >= 3);
    }

    // ─── Test 3: Parallel tool execution ─────────────────────────────────

    #[tokio::test]
    async fn test_parallel_tool_execution() {
        // Two tool calls in one response
        let parallel_events = vec![
            ChatEvent::Start,
            ChatEvent::ToolCallStart {
                id: "tc1".to_string(),
                name: "echo".to_string(),
            },
            ChatEvent::ToolCallDone {
                id: "tc1".to_string(),
                name: "echo".to_string(),
                arguments: serde_json::json!({"n": 1}),
            },
            ChatEvent::ToolCallStart {
                id: "tc2".to_string(),
                name: "echo2".to_string(),
            },
            ChatEvent::ToolCallDone {
                id: "tc2".to_string(),
                name: "echo2".to_string(),
                arguments: serde_json::json!({"n": 2}),
            },
            ChatEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
                cost: pi_ai::Cost::default(),
            },
        ];
        let follow_events = done_events(StopReason::Stop);

        let mut agent = make_agent(
            vec![parallel_events, follow_events],
            vec![
                Arc::new(EchoTool { name: "echo" }),
                Arc::new(EchoTool { name: "echo2" }),
            ],
            ToolExecutionMode::Parallel,
            Hooks::default(),
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("parallel".to_string()).await;
        assert!(result.is_ok());

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let tool_ends: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { .. }))
            .collect();
        assert_eq!(tool_ends.len(), 2, "Should have 2 ToolExecutionEnd events");
    }

    // ─── Test 4: before_tool_call hook blocks a tool ──────────────────────

    #[tokio::test]
    async fn test_before_hook_blocks() {
        let tool_events = tool_call_events("tc1", "echo", r#"{}"#);
        let follow_events = done_events(StopReason::Stop);

        let hooks = Hooks {
            before_tool_call: Some(Box::new(|_ctx| {
                Box::pin(async {
                    Some(BeforeToolCallResult {
                        block: true,
                        reason: Some("Blocked for test".to_string()),
                    })
                })
            })),
            after_tool_call: None,
            transform_context: None,
        };

        let mut agent = make_agent(
            vec![tool_events, follow_events],
            vec![Arc::new(EchoTool { name: "echo" })],
            ToolExecutionMode::Sequential,
            hooks,
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("call echo".to_string()).await;
        assert!(result.is_ok());

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Tool end should have error flag (blocked = error result)
        let blocked_end = events.iter().any(|e| {
            if let AgentEvent::ToolExecutionEnd {
                result, tool_name, ..
            } = e
            {
                tool_name == "echo" && result.is_error
            } else {
                false
            }
        });
        assert!(
            blocked_end,
            "Blocked tool should produce error ToolExecutionEnd"
        );
    }

    // ─── Test 5: Abort mid-run ────────────────────────────────────────────

    #[tokio::test]
    async fn test_abort() {
        // Simulate a slow tool by using a provider that never completes (we abort immediately).
        // We'll use done_events but call abort before prompt.
        let mut agent = make_agent(
            vec![done_events(StopReason::Stop)],
            vec![],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );

        // Cancel before running
        agent.abort();

        let result = agent.prompt("test".to_string()).await;
        assert!(
            matches!(result, Err(AgentError::Aborted)),
            "Expected Aborted error, got {result:?}"
        );
    }

    // ─── Test 6: Steer message injected between turns ────────────────────

    /// Provider that emits an Error ChatEvent mid-stream, followed by Done.
    struct ErrorMidStreamProvider;

    #[async_trait]
    impl pi_ai::LlmProvider for ErrorMidStreamProvider {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatStream, ProviderError> {
            let events: Vec<Result<ChatEvent, ProviderError>> = vec![
                Ok(ChatEvent::Start),
                Ok(ChatEvent::TextStart),
                Ok(ChatEvent::TextDelta {
                    text: "partial".to_string(),
                }),
                Ok(ChatEvent::Error {
                    message: "provider blew up mid-stream".to_string(),
                }),
                Ok(ChatEvent::Done {
                    stop_reason: StopReason::Error,
                    usage: Usage::default(),
                    cost: pi_ai::Cost::default(),
                }),
            ];
            Ok(Box::pin(stream::iter(events)))
        }
        fn name(&self) -> &str {
            "error-mid-stream"
        }
    }

    #[tokio::test]
    async fn test_provider_error_event_mid_stream() {
        // Provider emits an Error ChatEvent — agent should handle it gracefully
        let provider = Arc::new(ErrorMidStreamProvider);
        let config = AgentConfig {
            provider,
            tools: vec![],
            tool_execution_mode: ToolExecutionMode::Sequential,
            hooks: Hooks::default(),
        };
        let state = AgentState {
            messages: vec![],
            model: test_model(),
            system_prompt: None,
            thinking_level: None,
            is_streaming: false,
            error_message: None,
        };
        let mut agent = Agent::new(config, state);
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("Hi".to_string()).await;

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // The agent should complete without panicking.
        // It may return Ok or Err — but it must emit AgentEnd.
        let has_agent_end = events
            .iter()
            .any(|e| matches!(e, AgentEvent::AgentEnd { .. }));
        assert!(
            has_agent_end,
            "Agent should emit AgentEnd even after a mid-stream Error event; result={result:?}"
        );

        // Verify the error was recorded in the assistant message
        let state = agent.state().await;
        let last_msg = state.messages.last();
        let has_error_recorded = last_msg
            .map(|m| {
                matches!(
                    m,
                    Message::Assistant {
                        error_message: Some(_),
                        ..
                    }
                )
            })
            .unwrap_or(false);
        assert!(
            has_error_recorded || result.is_err(),
            "Error mid-stream should be recorded in message or propagated as Err"
        );
    }

    #[tokio::test]
    async fn test_done_with_stop_reason_length_no_tools() {
        // Provider returns Done with StopReason::Length (context limit hit, no tools)
        // Agent should complete cleanly and emit AgentEnd with Length reason
        let mut agent = make_agent(
            vec![done_events(StopReason::Length)],
            vec![],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("Tell me everything".to_string()).await;
        assert!(
            result.is_ok(),
            "StopReason::Length with no tools should complete ok: {result:?}"
        );

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let agent_end = events.iter().find_map(|e| {
            if let AgentEvent::AgentEnd { stop_reason } = e {
                Some(*stop_reason)
            } else {
                None
            }
        });
        assert!(agent_end.is_some(), "Should have AgentEnd event");
        assert_eq!(
            agent_end.unwrap(),
            StopReason::Length,
            "AgentEnd should carry StopReason::Length"
        );
    }

    #[tokio::test]
    async fn test_tool_call_references_nonexistent_tool() {
        // Provider calls a tool that doesn't exist in the registered tools list.
        // Agent should return an error ToolResult and continue to the follow-up turn.
        let tool_events = tool_call_events("tc1", "ghost_tool", r#"{"x":1}"#);
        let follow_events = done_events(StopReason::Stop);

        let mut agent = make_agent(
            vec![tool_events, follow_events],
            // No tools registered at all
            vec![],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("use ghost_tool".to_string()).await;
        assert!(
            result.is_ok(),
            "Missing tool should not crash agent: {result:?}"
        );

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // The tool execution end event should have is_error=true
        let ghost_end = events.iter().find(|e| {
            matches!(e, AgentEvent::ToolExecutionEnd { tool_name, result, .. }
                if tool_name == "ghost_tool" && result.is_error)
        });
        assert!(
            ghost_end.is_some(),
            "Non-existent tool should produce a ToolExecutionEnd with is_error=true"
        );
    }

    #[tokio::test]
    async fn test_two_consecutive_tool_call_turns() {
        // Sequence: user → assistant(tool A) → tool result → assistant(tool B) → tool result → assistant(final)
        let tool_events_a = tool_call_events("tc1", "echo", r#"{"n":1}"#);
        let tool_events_b = tool_call_events("tc2", "echo", r#"{"n":2}"#);
        let final_events = done_events(StopReason::Stop);

        let mut agent = make_agent(
            vec![tool_events_a, tool_events_b, final_events],
            vec![Arc::new(EchoTool { name: "echo" })],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );
        let mut rx = agent.take_event_receiver().unwrap();

        let result = agent.prompt("run two tools".to_string()).await;
        assert!(
            result.is_ok(),
            "Two consecutive tool turns should succeed: {result:?}"
        );

        let mut events = vec![];
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        let tool_ends: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolExecutionEnd { tool_name, .. } if tool_name == "echo"))
            .collect();
        assert_eq!(
            tool_ends.len(),
            2,
            "Should have 2 ToolExecutionEnd events for two consecutive tool turns"
        );

        let state = agent.state().await;
        // user + assistant(tc1) + tool_result(tc1) + assistant(tc2) + tool_result(tc2) + assistant(final)
        assert!(
            state.messages.len() >= 5,
            "Conversation should have at least 5 messages after two tool turns, got {}",
            state.messages.len()
        );
    }

    #[tokio::test]
    async fn test_steer_message() {
        // First turn: tool call. After tool executes, a steer message is injected.
        // Second turn: final text response.
        let tool_events = tool_call_events("tc1", "echo", r#"{}"#);
        let follow_events = done_events(StopReason::Stop);

        let mut agent = make_agent(
            vec![tool_events, follow_events],
            vec![Arc::new(EchoTool { name: "echo" })],
            ToolExecutionMode::Sequential,
            Hooks::default(),
        );

        // Inject steering message (will be picked up on the next inner turn)
        agent.steer(Message::User {
            content: vec![ContentBlock::Text {
                text: "Steered!".to_string(),
            }],
        });

        let result = agent.prompt("call echo".to_string()).await;
        assert!(result.is_ok(), "Expected Ok, got {result:?}");

        let state = agent.state().await;
        // Should include the steered message in the conversation
        let has_steered = state.messages.iter().any(|m| {
            if let Message::User { content } = m {
                content
                    .iter()
                    .any(|c| matches!(c, ContentBlock::Text { text } if text == "Steered!"))
            } else {
                false
            }
        });
        assert!(has_steered, "Steer message should appear in conversation");
    }
}
