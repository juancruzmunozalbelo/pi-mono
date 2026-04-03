//! AgentEvent — events emitted by the agent loop.

use pi_ai::{ChatEvent, Message, StopReason};
use pi_tools::ToolResult;

/// Events emitted by the agent during execution.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// The agent loop started.
    AgentStart,
    /// A new turn (provider call) started.
    TurnStart,
    /// An assistant message was started (before streaming completes).
    MessageStart { message: Message },
    /// A streaming event from the provider.
    MessageUpdate { event: ChatEvent },
    /// An assistant message was completed.
    MessageEnd { message: Message },
    /// A tool execution started.
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
    },
    /// A partial / intermediate update from a tool (optional, for long-running tools).
    ToolExecutionUpdate {
        tool_call_id: String,
        partial_result: ToolResult,
    },
    /// A tool execution completed.
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: ToolResult,
    },
    /// A turn completed.
    TurnEnd { error: Option<String> },
    /// The agent loop ended.
    AgentEnd { stop_reason: StopReason },
}
