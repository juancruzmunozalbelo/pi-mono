use async_trait::async_trait;
use serde::Deserialize;
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;

use crate::{error_result, text_result, Tool, ToolResult};

pub struct FindTool;

#[derive(Deserialize)]
struct FindParams {
    pattern: String,
    path: Option<String>,
    limit: Option<u64>,
}

const DEFAULT_LIMIT: usize = 1000;

#[async_trait]
impl Tool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern, sorted by modification time"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern to match files" },
                "path": { "type": "string", "description": "Base directory to search in" },
                "limit": { "type": "integer", "description": "Max results (default 1000)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let p: FindParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let limit = p.limit.unwrap_or(DEFAULT_LIMIT as u64) as usize;
        let base_path = p.path.as_deref().unwrap_or(".");

        // Build the full glob pattern
        let full_pattern = if p.pattern.contains('/') || p.pattern.starts_with("**") {
            // Pattern already has path components
            if p.path.is_some() {
                format!("{}/{}", base_path.trim_end_matches('/'), p.pattern)
            } else {
                p.pattern.clone()
            }
        } else {
            format!("{}/**/{}", base_path.trim_end_matches('/'), p.pattern)
        };

        let entries = match glob::glob(&full_pattern) {
            Ok(paths) => paths,
            Err(e) => return error_result(format!("Invalid glob pattern: {e}")),
        };

        struct Entry {
            path: std::path::PathBuf,
            mtime: SystemTime,
            is_dir: bool,
        }

        let mut results: Vec<Entry> = Vec::new();

        for entry in entries.filter_map(|e| e.ok()) {
            let meta = std::fs::metadata(&entry);
            let mtime = meta
                .as_ref()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            results.push(Entry {
                path: entry,
                mtime,
                is_dir,
            });
        }

        // Sort by modification time descending (newest first)
        results.sort_by(|a, b| b.mtime.cmp(&a.mtime));

        let mut output = String::new();
        for entry in results.iter().take(limit) {
            let mut path_str = entry.path.to_string_lossy().to_string();
            if entry.is_dir {
                path_str.push('/');
            }
            output.push_str(&path_str);
            output.push('\n');
        }

        if output.is_empty() {
            return text_result("No files found");
        }

        text_result(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> FindTool {
        FindTool
    }

    #[tokio::test]
    async fn test_find_files() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("foo.rs");
        let f2 = dir.path().join("bar.txt");
        std::fs::File::create(&f1).unwrap().write_all(b"").unwrap();
        std::fs::File::create(&f2).unwrap().write_all(b"").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "pattern": "*.rs",
            "path": dir.path().to_str().unwrap()
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("foo.rs"));
        assert!(!text.contains("bar.txt"));
    }

    #[tokio::test]
    async fn test_find_no_matches() {
        let dir = tempfile::tempdir().unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "pattern": "*.xyz",
            "path": dir.path().to_str().unwrap()
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("No files found"));
    }
}
