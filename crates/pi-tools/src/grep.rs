use async_trait::async_trait;
use regex::RegexBuilder;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

use crate::{error_result, text_result, Tool, ToolResult};

pub struct GrepTool;

#[derive(Deserialize)]
struct GrepParams {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    ignore_case: Option<bool>,
    context: Option<u64>,
    limit: Option<u64>,
}

const DEFAULT_LIMIT: usize = 100;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for a pattern in files using regex"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "path": { "type": "string", "description": "Directory or file to search in" },
                "glob": { "type": "string", "description": "Glob pattern to filter files" },
                "ignore_case": { "type": "boolean", "description": "Case-insensitive matching" },
                "context": { "type": "integer", "description": "Lines of context before/after match" },
                "limit": { "type": "integer", "description": "Max matches to return (default 100)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let p: GrepParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let re = match RegexBuilder::new(&p.pattern)
            .case_insensitive(p.ignore_case.unwrap_or(false))
            .build()
        {
            Ok(r) => r,
            Err(e) => return error_result(format!("Invalid regex pattern: {e}")),
        };

        let search_path = p.path.as_deref().unwrap_or(".");
        let limit = p.limit.unwrap_or(DEFAULT_LIMIT as u64) as usize;
        let ctx_lines = p.context.unwrap_or(0) as usize;

        // Build glob matcher if provided
        let glob_pattern = p.glob.as_deref();

        let mut output = String::new();
        let mut match_count = 0;

        'outer: for entry in WalkDir::new(search_path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let file_path = entry.path();

            // Apply glob filter if specified
            if let Some(glob_pat) = glob_pattern {
                let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Match against the full path for glob patterns with path separators
                let path_str = file_path.to_string_lossy();
                let matches = if glob_pat.contains('/') || glob_pat.contains('*') {
                    match glob::Pattern::new(glob_pat) {
                        Ok(pat) => pat.matches(&path_str) || pat.matches(file_name),
                        Err(_) => false,
                    }
                } else {
                    match glob::Pattern::new(glob_pat) {
                        Ok(pat) => pat.matches(file_name),
                        Err(_) => false,
                    }
                };
                if !matches {
                    continue;
                }
            }

            // Read file content
            let content = match std::fs::read(file_path) {
                Ok(b) => b,
                Err(_) => continue,
            };

            // Skip binary files (check first 512 bytes for null bytes)
            let check_len = content.len().min(512);
            if content[..check_len].contains(&0u8) {
                continue;
            }

            let text = match String::from_utf8(content) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let lines: Vec<&str> = text.lines().collect();
            let path_display = file_path.to_string_lossy();

            // Find matching line indices
            let mut i = 0;
            while i < lines.len() {
                if re.is_match(lines[i]) {
                    let line_num = i + 1;
                    // Context before
                    let ctx_start = i.saturating_sub(ctx_lines);
                    for (ci, _) in lines[ctx_start..i].iter().enumerate() {
                        let actual_idx = ctx_start + ci;
                        let _ = writeln_grep(
                            &mut output,
                            &path_display,
                            actual_idx + 1,
                            lines[actual_idx],
                            false,
                        );
                    }
                    // Match line
                    let _ = writeln_grep(&mut output, &path_display, line_num, lines[i], true);
                    match_count += 1;

                    // Context after
                    let ctx_end = (i + ctx_lines + 1).min(lines.len());
                    for (ci, _) in lines[(i + 1)..ctx_end].iter().enumerate() {
                        let actual_idx = i + 1 + ci;
                        let _ = writeln_grep(
                            &mut output,
                            &path_display,
                            actual_idx + 1,
                            lines[actual_idx],
                            false,
                        );
                    }

                    if match_count >= limit {
                        output
                            .push_str(&format!("\n[Limit reached: showing first {limit} matches]"));
                        break 'outer;
                    }

                    // Skip ahead past context to avoid duplicate context lines
                    i += 1;
                } else {
                    i += 1;
                }
            }
        }

        if match_count == 0 {
            return text_result("No matches found");
        }

        text_result(output)
    }
}

fn writeln_grep(
    out: &mut String,
    path: &str,
    line_num: usize,
    line: &str,
    is_match: bool,
) -> std::fmt::Result {
    use std::fmt::Write;
    if is_match {
        writeln!(out, "{path}:{line_num}:{line}")
    } else {
        writeln!(out, "{path}-{line_num}-{line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> GrepTool {
        GrepTool
    }

    #[tokio::test]
    async fn test_grep_basic() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "hello world").unwrap();
        writeln!(f, "foo bar").unwrap();
        writeln!(f, "hello again").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "pattern": "hello",
            "path": dir.path().to_str().unwrap()
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("hello world"));
        assert!(text.contains("hello again"));
        assert!(!text.contains("foo bar"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "hello world").unwrap();

        let tool = make_tool();
        let params = serde_json::json!({
            "pattern": "xyz_not_found",
            "path": dir.path().to_str().unwrap()
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("No matches found"));
    }
}
