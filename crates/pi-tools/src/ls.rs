use async_trait::async_trait;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{error_result, text_result, Tool, ToolResult};

pub struct LsTool;

#[derive(Deserialize)]
struct LsParams {
    path: Option<String>,
    limit: Option<u64>,
}

const DEFAULT_LIMIT: usize = 500;

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List directory entries with name, type, and size"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path to list (default: current directory)" },
                "limit": { "type": "integer", "description": "Max entries to return (default 500)" }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let p: LsParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let dir_path = p.path.as_deref().unwrap_or(".");
        let limit = p.limit.unwrap_or(DEFAULT_LIMIT as u64) as usize;

        let meta = match std::fs::metadata(dir_path) {
            Ok(m) => m,
            Err(e) => return error_result(format!("Cannot access '{}': {e}", dir_path)),
        };

        if meta.is_file() {
            return error_result(format!("'{}' is a file, not a directory", dir_path));
        }

        let read_dir = match std::fs::read_dir(dir_path) {
            Ok(r) => r,
            Err(e) => return error_result(format!("Cannot list '{}': {e}", dir_path)),
        };

        struct Entry {
            name: String,
            kind: &'static str,
            size: u64,
        }

        let mut entries: Vec<Entry> = Vec::new();

        for item in read_dir.filter_map(|e| e.ok()) {
            let name = item.file_name().to_string_lossy().to_string();
            let meta = item.metadata().ok();
            let (kind, size) = match &meta {
                Some(m) if m.is_dir() => ("dir", 0u64),
                Some(m) => ("file", m.len()),
                None => ("unknown", 0u64),
            };
            entries.push(Entry { name, kind, size });
        }

        // Sort alphabetically, case-insensitive
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let mut output = String::new();
        for entry in entries.iter().take(limit) {
            let size_str = if entry.kind == "dir" {
                String::from("-")
            } else {
                entry.size.to_string()
            };
            output.push_str(&format!("{}\t{}\t{}\n", entry.name, entry.kind, size_str));
        }

        if output.is_empty() {
            return text_result("(empty directory)");
        }

        text_result(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> LsTool {
        LsTool
    }

    #[tokio::test]
    async fn test_ls_directory() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("alpha.txt");
        let f2 = dir.path().join("beta.txt");
        std::fs::File::create(&f1)
            .unwrap()
            .write_all(b"hello")
            .unwrap();
        std::fs::File::create(&f2)
            .unwrap()
            .write_all(b"world")
            .unwrap();

        let tool = make_tool();
        let params = serde_json::json!({ "path": dir.path().to_str().unwrap() });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("alpha.txt"));
        assert!(text.contains("beta.txt"));
        assert!(text.contains("file"));
    }

    #[tokio::test]
    async fn test_ls_file_is_error() {
        let tmp = tempfile::NamedTempFile::new().unwrap();

        let tool = make_tool();
        let params = serde_json::json!({ "path": tmp.path().to_str().unwrap() });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_ls_nonexistent() {
        let tool = make_tool();
        let params = serde_json::json!({ "path": "/nonexistent/path/xyz" });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(result.is_error);
    }
}
