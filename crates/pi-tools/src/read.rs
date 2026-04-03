use async_trait::async_trait;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{Tool, ToolResult, error_result, text_result};

pub struct ReadFileTool;

#[derive(Deserialize)]
struct ReadParams {
    path: String,
    offset: Option<u64>,
    limit: Option<u64>,
}

const MAX_BYTES: usize = 50 * 1024; // 50KB
const DEFAULT_LIMIT: u64 = 2000;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read file contents with line numbers"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" },
                "offset": { "type": "integer", "description": "1-based line to start from" },
                "limit": { "type": "integer", "description": "Number of lines to read" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let p: ReadParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let content = match tokio::fs::read_to_string(&p.path).await {
            Ok(c) => c,
            Err(e) => return error_result(format!("Cannot read '{}': {e}", p.path)),
        };

        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        // offset is 1-indexed; default start at line 1
        let start = p.offset.map(|o| o.saturating_sub(1) as usize).unwrap_or(0);
        let limit = p.limit.unwrap_or(DEFAULT_LIMIT) as usize;

        let start = start.min(total);
        let end = (start + limit).min(total);
        let lines = &all_lines[start..end];

        let mut output = String::new();
        let mut bytes = 0usize;

        for (i, line) in lines.iter().enumerate() {
            let line_num = start + i + 1; // 1-indexed
            let entry = format!("{line_num}\t{line}\n");
            if bytes + entry.len() > MAX_BYTES {
                output.push_str(&format!(
                    "\n[Truncated: output exceeded {MAX_BYTES} bytes]"
                ));
                break;
            }
            bytes += entry.len();
            output.push_str(&entry);
        }

        text_result(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> ReadFileTool {
        ReadFileTool
    }

    #[tokio::test]
    async fn test_read_existing_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "line one").unwrap();
        writeln!(tmp, "line two").unwrap();
        writeln!(tmp, "line three").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("1\tline one"));
        assert!(text.contains("2\tline two"));
        assert!(text.contains("3\tline three"));
    }

    #[tokio::test]
    async fn test_read_missing_file() {
        let tool = make_tool();
        let params = serde_json::json!({ "path": "/nonexistent/file/that/does/not/exist.txt" });
        let result = tool.execute(params, CancellationToken::new()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_read_with_offset_and_limit() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for i in 1..=10 {
            writeln!(tmp, "line {i}").unwrap();
        }

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "offset": 3,
            "limit": 2
        });
        let result = tool.execute(params, CancellationToken::new()).await;
        assert!(!result.is_error);

        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("3\tline 3"));
        assert!(text.contains("4\tline 4"));
        assert!(!text.contains("5\tline 5"));
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_read_empty_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Write nothing — file is empty

        let tool = make_tool();
        let params = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error, "Reading an empty file should not be an error");
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert_eq!(
            text.trim(),
            "",
            "Reading empty file should return empty content, got: {text:?}"
        );
    }

    #[tokio::test]
    async fn test_read_offset_beyond_file_length() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "line one").unwrap();
        writeln!(tmp, "line two").unwrap();

        let tool = make_tool();
        // offset 999 is way beyond the 2 lines in the file
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "offset": 999
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(
            !result.is_error,
            "Reading with offset beyond file length should not error"
        );
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert_eq!(
            text.trim(),
            "",
            "Offset beyond file end should return empty content, not panic"
        );
    }

    #[tokio::test]
    async fn test_read_binary_file_does_not_crash() {
        use std::io::Write as IoWrite;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // Write arbitrary binary bytes including null bytes and high bytes
        let binary_data: Vec<u8> = (0u8..=255u8).cycle().take(1024).collect();
        tmp.write_all(&binary_data).unwrap();

        let tool = make_tool();
        let params = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
        // Should not panic — error is acceptable (invalid UTF-8) but panic is not
        let _ = tool.execute(params, CancellationToken::new()).await;
        // If we got here, no panic occurred
    }
}
