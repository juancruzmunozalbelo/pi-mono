//! Hook types for intercepting tool execution and context transformation.

use std::future::Future;
use std::pin::Pin;

use pi_ai::Message;
use pi_tools::{ToolContent, ToolResult};

/// A boxed, pinned future that is `Send`.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// ─── Before-tool-call hook ────────────────────────────────────────────────────

/// Context passed to the `before_tool_call` hook.
pub struct BeforeToolCallContext {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Return value from the `before_tool_call` hook.
pub struct BeforeToolCallResult {
    /// If `true` the tool call is blocked and never executed.
    pub block: bool,
    /// Optional human-readable reason returned as a tool error.
    pub reason: Option<String>,
}

// ─── After-tool-call hook ─────────────────────────────────────────────────────

/// Context passed to the `after_tool_call` hook.
pub struct AfterToolCallContext {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: ToolResult,
}

/// Return value from the `after_tool_call` hook.
pub struct AfterToolCallResult {
    /// Override the result content (if `Some`).
    pub content: Option<Vec<ToolContent>>,
    /// Override the error flag (if `Some`).
    pub is_error: Option<bool>,
}

// ─── Hooks bundle ─────────────────────────────────────────────────────────────

/// Hook function that runs before a tool call.
pub type BeforeToolCallHook = Box<
    dyn Fn(BeforeToolCallContext) -> BoxFuture<Option<BeforeToolCallResult>> + Send + Sync,
>;

/// Hook function that runs after a tool call.
pub type AfterToolCallHook = Box<
    dyn Fn(AfterToolCallContext) -> BoxFuture<Option<AfterToolCallResult>> + Send + Sync,
>;

/// Hook function that transforms the context before sending to the provider.
pub type TransformContextHook =
    Box<dyn Fn(Vec<Message>) -> BoxFuture<Vec<Message>> + Send + Sync>;

/// Collection of optional lifecycle hooks for an agent.
#[derive(Default)]
pub struct Hooks {
    pub before_tool_call: Option<BeforeToolCallHook>,
    pub after_tool_call: Option<AfterToolCallHook>,
    pub transform_context: Option<TransformContextHook>,
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks")
            .field(
                "before_tool_call",
                &self.before_tool_call.as_ref().map(|_| "<fn>"),
            )
            .field(
                "after_tool_call",
                &self.after_tool_call.as_ref().map(|_| "<fn>"),
            )
            .field(
                "transform_context",
                &self.transform_context.as_ref().map(|_| "<fn>"),
            )
            .finish()
    }
}
