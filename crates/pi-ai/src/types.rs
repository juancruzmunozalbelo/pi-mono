use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Content block in a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default)]
        redacted: bool,
    },
    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    #[serde(rename = "image")]
    Image {
        data: String,
        mime_type: String,
    },
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Message {
    #[serde(rename = "user")]
    User { content: Vec<ContentBlock> },
    #[serde(rename = "assistant")]
    Assistant {
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<StopReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// LLM model definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: ApiType,
    pub base_url: String,
    pub reasoning: bool,
    pub input_types: Vec<String>,
    pub cost: ModelCost,
    pub context_window: u64,
    pub max_tokens: u64,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApiType {
    OpenaiCompletions,
    AnthropicMessages,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

/// Thinking level for reasoning models
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// Chat request sent to a provider
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: Model,
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinition>,
    pub thinking_level: Option<ThinkingLevel>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Events emitted during streaming
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// Stream started
    Start,
    /// Text content block started
    TextStart,
    /// Text content delta
    TextDelta { text: String },
    /// Text content block ended
    TextEnd,
    /// Thinking/reasoning block started
    ThinkingStart,
    /// Thinking/reasoning content delta
    ThinkingDelta { text: String },
    /// Thinking/reasoning block ended
    ThinkingEnd,
    /// Tool call started
    ToolCallStart { id: String, name: String },
    /// Tool call argument delta (partial JSON)
    ToolCallDelta {
        id: String,
        name: String,
        arguments_delta: String,
    },
    /// Tool call complete with parsed arguments
    ToolCallDone {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Stream completed successfully
    Done {
        stop_reason: StopReason,
        usage: Usage,
        cost: Cost,
    },
    /// Stream error
    Error { message: String },
}

/// Agent-layer message (type alias for extensibility)
pub type AgentMessage = Message;
