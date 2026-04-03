use async_trait::async_trait;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{Tool, ToolResult, error_result, text_result};

pub struct EditFileTool;

#[derive(Deserialize)]
struct EditParams {
    path: String,
    old_string: String,
    new_string: String,
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replace a unique string in a file with a new string"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to edit" },
                "old_string": { "type": "string", "description": "Exact string to find (must be unique)" },
                "new_string": { "type": "string", "description": "Replacement string" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let p: EditParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let content = match tokio::fs::read_to_string(&p.path).await {
            Ok(c) => c,
            Err(e) => return error_result(format!("Cannot read '{}': {e}", p.path)),
        };

        let count = content.matches(&p.old_string as &str).count();

        match count {
            0 => return error_result("old_string not found in file"),
            n if n > 1 => {
                return error_result(format!(
                    "old_string found {n} times, must be unique"
                ))
            }
            _ => {}
        }

        let new_content = content.replacen(&p.old_string as &str, &p.new_string, 1);

        if let Err(e) = tokio::fs::write(&p.path, &new_content).await {
            return error_result(format!("Cannot write '{}': {e}", p.path));
        }

        // Build a simple diff showing changed lines
        let diff = build_diff(&content, &new_content, &p.path);
        text_result(diff)
    }
}

fn build_diff(old: &str, new: &str, path: &str) -> String {
    use std::fmt::Write;

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut out = String::new();
    let _ = writeln!(out, "--- {path}");
    let _ = writeln!(out, "+++ {path}");

    let max = old_lines.len().max(new_lines.len());
    for i in 0..max {
        let old_line = old_lines.get(i);
        let new_line = new_lines.get(i);
        match (old_line, new_line) {
            (Some(o), Some(n)) if o != n => {
                let _ = writeln!(out, "-{o}");
                let _ = writeln!(out, "+{n}");
            }
            (Some(o), None) => {
                let _ = writeln!(out, "-{o}");
            }
            (None, Some(n)) => {
                let _ = writeln!(out, "+{n}");
            }
            _ => {}
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> EditFileTool {
        EditFileTool
    }

    #[tokio::test]
    async fn test_successful_edit() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "foo bar baz").unwrap();
        writeln!(tmp, "hello world").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": "hello world",
            "new_string": "goodbye world"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("goodbye world"));
        assert!(!content.contains("hello world"));
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "some content here").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": "nonexistent string",
            "new_string": "replacement"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_ambiguous() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "duplicate line").unwrap();
        writeln!(tmp, "duplicate line").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": "duplicate line",
            "new_string": "unique line"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("must be unique"));
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_edit_old_string_contains_regex_special_chars() {
        // old_string contains regex metacharacters — should be treated as literal
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "price: $10.00 (discounted)").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": "$10.00 (discounted)",
            "new_string": "$9.99 (sale)"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error, "Edit with regex-special chars should succeed as literal match");
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            content.contains("$9.99 (sale)"),
            "Replacement with regex-special chars should work literally"
        );
        assert!(
            !content.contains("$10.00"),
            "Old string with regex-special chars should be replaced"
        );
    }

    #[tokio::test]
    async fn test_edit_old_and_new_string_identical() {
        // old_string == new_string — should succeed (no-op replacement is still valid)
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "unchanged content").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": "unchanged content",
            "new_string": "unchanged content"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error, "Edit where old==new should succeed");
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            content.contains("unchanged content"),
            "File content should remain unchanged when old==new"
        );
    }

    #[tokio::test]
    async fn test_edit_on_empty_file() {
        // Empty file — old_string should not be found
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // do not write anything

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": "anything",
            "new_string": "something"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(
            result.is_error,
            "Edit on empty file should return error (old_string not found)"
        );
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(
            text.contains("not found"),
            "Error message should say old_string was not found"
        );
    }

    #[tokio::test]
    async fn test_edit_old_string_is_entire_file_content() {
        // old_string spans the entire file — should replace the whole file
        let content = "entire file content here\nwith multiple lines\n";
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), content).unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "old_string": content,
            "new_string": "brand new content\n"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error, "Edit replacing entire file content should succeed");
        let new_content = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(
            new_content, "brand new content\n",
            "Entire file should be replaced"
        );
    }
}
