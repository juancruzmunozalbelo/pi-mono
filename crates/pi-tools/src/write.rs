use async_trait::async_trait;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{Tool, ToolResult, error_result, text_result};

pub struct WriteFileTool;

#[derive(Deserialize)]
struct WriteParams {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file, creating parent directories as needed"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let p: WriteParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let path = std::path::Path::new(&p.path);

        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return error_result(format!("Cannot create directories for '{}': {e}", p.path));
            }
        }

        let bytes = p.content.len();
        if let Err(e) = tokio::fs::write(&p.path, &p.content).await {
            return error_result(format!("Cannot write '{}': {e}", p.path));
        }

        text_result(format!("Wrote {bytes} bytes to '{}'", p.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> WriteFileTool {
        WriteFileTool
    }

    #[tokio::test]
    async fn test_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new_file.txt");

        let tool = make_tool();
        let params = serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "hello world\n"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, "hello world\n");
    }

    #[tokio::test]
    async fn test_write_overwrites_existing() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut tmp, b"old content").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "content": "new content"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let written = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(written, "new content");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subdir/nested/file.txt");

        let tool = make_tool();
        let params = serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "nested file"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        assert!(path.exists());
    }
}
