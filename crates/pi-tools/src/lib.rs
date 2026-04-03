//! Pi tools — Tool execution and filesystem utilities

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub mod bash;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod write;

pub use bash::BashTool;
pub use edit::EditFileTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use ls::LsTool;
pub use read::ReadFileTool;
pub use write::WriteFileTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute(
        &self,
        params: serde_json::Value,
        cancel: CancellationToken,
    ) -> ToolResult;
}

/// Helper to create a text ToolResult
pub fn text_result(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: vec![ToolContent::Text { text: text.into() }],
        is_error: false,
    }
}

pub fn error_result(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: vec![ToolContent::Text { text: text.into() }],
        is_error: true,
    }
}
