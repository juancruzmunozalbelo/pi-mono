//! Core agent loop — orchestrates provider calls and tool execution.

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use pi_ai::{ChatEvent, ChatRequest, ContentBlock, Message, StopReason, ToolDefinition, Usage};
use pi_tools::{ToolContent, ToolResult};

use crate::event::AgentEvent;
use crate::hooks::{AfterToolCallContext, BeforeToolCallContext};
use crate::state::Agent;
use crate::AgentError;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn emit(tx: &mpsc::UnboundedSender<AgentEvent>, event: AgentEvent) {
    // Best-effort: if the receiver has been dropped we just skip.
    let _ = tx.send(event);
}

/// Convert a `pi_tools::Tool` into a `pi_ai::ToolDefinition`.
fn tool_to_definition(tool: &Arc<dyn pi_tools::Tool>) -> ToolDefinition {
    ToolDefinition {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        parameters: tool.schema(),
    }
}

/// Consume the chat stream and build the completed assistant `Message`.
/// Emits `AgentEvent::MessageUpdate` for every raw `ChatEvent`.
async fn stream_to_message(
    stream: pi_ai::ChatStream,
    tx: &mpsc::UnboundedSender<AgentEvent>,
) -> Result<Message, AgentError> {
    let mut stream = Box::pin(stream);

    // Accumulators
    let mut text_buf = String::new();
    let mut thinking_buf = String::new();
    let mut thinking_signature: Option<String> = None;
    // tool calls: id → (name, args_delta)
    let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
    let mut stop_reason = StopReason::Stop;
    let mut usage = Usage::default();
    let mut error_msg: Option<String> = None;

    while let Some(item) = stream.next().await {
        match item {
            Err(e) => {
                error_msg = Some(e.to_string());
                break;
            }
            Ok(event) => {
                emit(
                    tx,
                    AgentEvent::MessageUpdate {
                        event: event.clone(),
                    },
                );

                match event {
                    ChatEvent::TextDelta { text } => text_buf.push_str(&text),
                    ChatEvent::ThinkingDelta { text } => thinking_buf.push_str(&text),
                    ChatEvent::ToolCallStart { id, name } => {
                        tool_calls.push((id, name, String::new()));
                    }
                    ChatEvent::ToolCallDelta {
                        id,
                        arguments_delta,
                        ..
                    } => {
                        if let Some(tc) = tool_calls.iter_mut().find(|tc| tc.0 == id) {
                            tc.2.push_str(&arguments_delta);
                        }
                    }
                    ChatEvent::ToolCallDone {
                        id,
                        name,
                        arguments,
                    } => {
                        // Replace delta accumulator with final parsed value
                        if let Some(tc) = tool_calls.iter_mut().find(|tc| tc.0 == id) {
                            tc.1 = name;
                            tc.2 = arguments.to_string();
                        }
                    }
                    ChatEvent::Done {
                        stop_reason: sr,
                        usage: u,
                        ..
                    } => {
                        stop_reason = sr;
                        usage = u;
                    }
                    ChatEvent::Error { message } => {
                        error_msg = Some(message);
                        stop_reason = StopReason::Error;
                    }
                    _ => {}
                }
            }
        }
    }

    // Build content blocks
    let mut content: Vec<ContentBlock> = Vec::new();

    if !thinking_buf.is_empty() {
        content.push(ContentBlock::Thinking {
            thinking: thinking_buf,
            signature: thinking_signature.take(),
            redacted: false,
        });
    }
    if !text_buf.is_empty() {
        content.push(ContentBlock::Text { text: text_buf });
    }
    for (id, name, args_str) in tool_calls {
        let arguments = serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
        content.push(ContentBlock::ToolCall {
            id,
            name,
            arguments,
        });
    }

    Ok(Message::Assistant {
        content,
        stop_reason: Some(stop_reason),
        usage: Some(usage),
        error_message: error_msg,
    })
}

/// Extract tool calls from an assistant message.
fn extract_tool_calls(msg: &Message) -> Vec<(String, String, serde_json::Value)> {
    let mut out = Vec::new();
    if let Message::Assistant { content, .. } = msg {
        for block in content {
            if let ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } = block
            {
                out.push((id.clone(), name.clone(), arguments.clone()));
            }
        }
    }
    out
}

/// Execute tools sequentially (task 6.2).
async fn execute_tools_sequential(
    tool_calls: Vec<(String, String, serde_json::Value)>,
    agent: &Agent,
) -> Vec<(String, String, ToolResult)> {
    let mut results = Vec::new();

    for (id, name, args) in tool_calls {
        emit(
            &agent.event_tx,
            AgentEvent::ToolExecutionStart {
                tool_call_id: id.clone(),
                tool_name: name.clone(),
            },
        );

        // before_tool_call hook
        if let Some(hook) = &agent.config.hooks.before_tool_call {
            let ctx = BeforeToolCallContext {
                tool_call_id: id.clone(),
                tool_name: name.clone(),
                arguments: args.clone(),
            };
            if let Some(hook_result) = hook(ctx).await {
                if hook_result.block {
                    let reason = hook_result
                        .reason
                        .unwrap_or_else(|| "Blocked by hook".to_string());
                    let result = ToolResult {
                        content: vec![ToolContent::Text { text: reason }],
                        is_error: true,
                    };
                    emit(
                        &agent.event_tx,
                        AgentEvent::ToolExecutionEnd {
                            tool_call_id: id.clone(),
                            tool_name: name.clone(),
                            result: result.clone(),
                        },
                    );
                    results.push((id, name, result));
                    continue;
                }
            }
        }

        // Find and execute the tool
        let result =
            execute_single_tool(&name, args, &agent.config.tools, agent.cancel.clone()).await;

        // after_tool_call hook
        let result = apply_after_hook(&agent.config.hooks, id.clone(), name.clone(), result).await;

        emit(
            &agent.event_tx,
            AgentEvent::ToolExecutionEnd {
                tool_call_id: id.clone(),
                tool_name: name.clone(),
                result: result.clone(),
            },
        );
        results.push((id, name, result));
    }

    results
}

/// Execute tools in parallel (task 6.3).
async fn execute_tools_parallel(
    tool_calls: Vec<(String, String, serde_json::Value)>,
    agent: &Agent,
) -> Vec<(String, String, ToolResult)> {
    // Emit all starts first (in source order)
    for (id, name, _) in &tool_calls {
        emit(
            &agent.event_tx,
            AgentEvent::ToolExecutionStart {
                tool_call_id: id.clone(),
                tool_name: name.clone(),
            },
        );
    }

    // Launch all concurrently
    let mut handles = Vec::new();
    for (id, name, args) in &tool_calls {
        let tools = agent.config.tools.clone();
        let cancel = agent.cancel.clone();
        let name_c = name.clone();
        let args_c = args.clone();
        let handle =
            tokio::spawn(async move { execute_single_tool(&name_c, args_c, &tools, cancel).await });
        handles.push((id.clone(), name.clone(), handle));
    }

    // Collect in source order and apply after hooks
    let mut results = Vec::new();
    for (id, name, handle) in handles {
        let raw = handle.await.unwrap_or_else(|e| ToolResult {
            content: vec![ToolContent::Text {
                text: format!("Task panicked: {e}"),
            }],
            is_error: true,
        });

        let result = apply_after_hook(&agent.config.hooks, id.clone(), name.clone(), raw).await;

        emit(
            &agent.event_tx,
            AgentEvent::ToolExecutionEnd {
                tool_call_id: id.clone(),
                tool_name: name.clone(),
                result: result.clone(),
            },
        );
        results.push((id, name, result));
    }

    results
}

/// Find a tool by name and run it (or return an error result).
async fn execute_single_tool(
    name: &str,
    args: serde_json::Value,
    tools: &[Arc<dyn pi_tools::Tool>],
    cancel: CancellationToken,
) -> ToolResult {
    if let Some(tool) = tools.iter().find(|t| t.name() == name) {
        tool.execute(args, cancel).await
    } else {
        ToolResult {
            content: vec![ToolContent::Text {
                text: format!("Unknown tool: {name}"),
            }],
            is_error: true,
        }
    }
}

/// Apply the `after_tool_call` hook to potentially override a result.
async fn apply_after_hook(
    hooks: &crate::hooks::Hooks,
    id: String,
    name: String,
    mut result: ToolResult,
) -> ToolResult {
    if let Some(hook) = &hooks.after_tool_call {
        let ctx = AfterToolCallContext {
            tool_call_id: id,
            tool_name: name,
            result: result.clone(),
        };
        if let Some(override_result) = hook(ctx).await {
            if let Some(content) = override_result.content {
                result.content = content;
            }
            if let Some(is_error) = override_result.is_error {
                result.is_error = is_error;
            }
        }
    }
    result
}

/// Push tool results into state and emit MessageStart/MessageEnd per result.
async fn push_tool_results(results: Vec<(String, String, ToolResult)>, agent: &Agent) {
    let mut state = agent.state.write().await;
    for (id, name, result) in results {
        let content_blocks: Vec<ContentBlock> = result
            .content
            .iter()
            .map(|c| match c {
                ToolContent::Text { text } => ContentBlock::Text { text: text.clone() },
            })
            .collect();
        let msg = Message::ToolResult {
            tool_call_id: id,
            tool_name: name,
            content: content_blocks,
            is_error: result.is_error,
        };
        state.messages.push(msg.clone());
        emit(
            &agent.event_tx,
            AgentEvent::MessageStart {
                message: msg.clone(),
            },
        );
        emit(&agent.event_tx, AgentEvent::MessageEnd { message: msg });
    }
}

// ─── Main loop ────────────────────────────────────────────────────────────────

/// Run the agent loop until the assistant produces a stop without tool calls
/// or the agent is aborted.
#[allow(unused_assignments)]
pub async fn run_loop(agent: &mut Agent) -> Result<(), AgentError> {
    emit(&agent.event_tx, AgentEvent::AgentStart);

    let mut pending_messages: Vec<Message> = agent.steering_queue.drain();
    let mut final_stop_reason = StopReason::Stop;

    'outer: loop {
        // ── Inner loop: one provider call per iteration ───────────────────
        'inner: loop {
            // Check for cancellation before starting a new turn.
            if agent.cancel.is_cancelled() {
                emit(
                    &agent.event_tx,
                    AgentEvent::TurnEnd {
                        error: Some("Aborted".to_string()),
                    },
                );
                emit(
                    &agent.event_tx,
                    AgentEvent::AgentEnd {
                        stop_reason: StopReason::Aborted,
                    },
                );
                return Err(AgentError::Aborted);
            }

            emit(&agent.event_tx, AgentEvent::TurnStart);

            // Inject pending steering messages
            {
                let mut state = agent.state.write().await;
                for msg in std::mem::take(&mut pending_messages) {
                    state.messages.push(msg.clone());
                    emit(
                        &agent.event_tx,
                        AgentEvent::MessageStart {
                            message: msg.clone(),
                        },
                    );
                    emit(&agent.event_tx, AgentEvent::MessageEnd { message: msg });
                }
            }

            // Build ChatRequest
            let request = {
                let state = agent.state.read().await;

                let messages = if let Some(transform) = &agent.config.hooks.transform_context {
                    transform(state.messages.clone()).await
                } else {
                    state.messages.clone()
                };

                let tools: Vec<ToolDefinition> =
                    agent.config.tools.iter().map(tool_to_definition).collect();

                ChatRequest {
                    model: state.model.clone(),
                    messages,
                    system_prompt: state.system_prompt.clone(),
                    tools,
                    thinking_level: state.thinking_level,
                    max_tokens: None,
                    temperature: None,
                }
            };

            // Stream the response
            {
                let mut state = agent.state.write().await;
                state.is_streaming = true;
            }

            let stream = agent
                .config
                .provider
                .chat(request)
                .await
                .map_err(AgentError::Provider)?;

            let assistant_msg = stream_to_message(stream, &agent.event_tx).await?;

            {
                let mut state = agent.state.write().await;
                state.is_streaming = false;
            }

            // Record stop reason / error
            let (msg_stop_reason, msg_error) = if let Message::Assistant {
                stop_reason,
                error_message,
                ..
            } = &assistant_msg
            {
                (*stop_reason, error_message.clone())
            } else {
                (None, None)
            };

            // Push assistant message to state
            {
                let mut state = agent.state.write().await;
                state.messages.push(assistant_msg.clone());
            }

            emit(
                &agent.event_tx,
                AgentEvent::MessageStart {
                    message: assistant_msg.clone(),
                },
            );
            emit(
                &agent.event_tx,
                AgentEvent::MessageEnd {
                    message: assistant_msg.clone(),
                },
            );

            // Handle error / abort
            if msg_error.is_some()
                || msg_stop_reason == Some(StopReason::Error)
                || msg_stop_reason == Some(StopReason::Aborted)
                || agent.cancel.is_cancelled()
            {
                let err = msg_error.or_else(|| {
                    if agent.cancel.is_cancelled() {
                        Some("Aborted".to_string())
                    } else {
                        msg_stop_reason.map(|r| format!("{r:?}"))
                    }
                });
                let sr = if agent.cancel.is_cancelled() {
                    StopReason::Aborted
                } else {
                    msg_stop_reason.unwrap_or(StopReason::Error)
                };
                emit(&agent.event_tx, AgentEvent::TurnEnd { error: err.clone() });
                emit(&agent.event_tx, AgentEvent::AgentEnd { stop_reason: sr });
                return if agent.cancel.is_cancelled() {
                    Err(AgentError::Aborted)
                } else {
                    Ok(())
                };
            }

            // Extract tool calls
            let tool_calls = extract_tool_calls(&assistant_msg);

            if tool_calls.is_empty() {
                // No tools — this turn is done
                final_stop_reason = msg_stop_reason.unwrap_or(StopReason::Stop);
                emit(&agent.event_tx, AgentEvent::TurnEnd { error: None });
                break 'inner;
            }

            // Record ToolUse stop reason for later
            final_stop_reason = msg_stop_reason.unwrap_or(StopReason::ToolUse);

            // Execute tools
            let results = match agent.config.tool_execution_mode {
                crate::state::ToolExecutionMode::Sequential => {
                    execute_tools_sequential(tool_calls, agent).await
                }
                crate::state::ToolExecutionMode::Parallel => {
                    execute_tools_parallel(tool_calls, agent).await
                }
            };

            push_tool_results(results, agent).await;

            emit(&agent.event_tx, AgentEvent::TurnEnd { error: None });

            // Drain any steering messages for the next inner turn
            pending_messages = agent.steering_queue.drain();

            // Check cancellation after tool execution
            if agent.cancel.is_cancelled() {
                emit(
                    &agent.event_tx,
                    AgentEvent::AgentEnd {
                        stop_reason: StopReason::Aborted,
                    },
                );
                return Err(AgentError::Aborted);
            }
        } // end 'inner

        // ── Check follow-up queue ─────────────────────────────────────────
        let follow_ups = agent.follow_up_queue.drain();
        if follow_ups.is_empty() {
            break 'outer;
        }
        pending_messages = follow_ups;
    } // end 'outer

    emit(
        &agent.event_tx,
        AgentEvent::AgentEnd {
            stop_reason: final_stop_reason,
        },
    );
    Ok(())
}
