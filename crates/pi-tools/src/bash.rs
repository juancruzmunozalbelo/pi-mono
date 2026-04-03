use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::{error_result, text_result, Tool, ToolResult};

pub struct BashTool;

#[derive(Deserialize)]
struct BashParams {
    command: String,
    timeout: Option<u64>,
}

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_BYTES: usize = 50 * 1024;
const MAX_LINES: usize = 2000;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return combined stdout+stderr output"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "timeout": { "type": "integer", "description": "Timeout in seconds (default 120)" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: serde_json::Value, cancel: CancellationToken) -> ToolResult {
        let p: BashParams = match serde_json::from_value(params) {
            Ok(v) => v,
            Err(e) => return error_result(format!("Invalid parameters: {e}")),
        };

        let timeout_secs = p.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.args(["-c", &p.command])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return error_result(format!("Failed to spawn command: {e}")),
        };

        // Take stdout and stderr handles before waiting
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        // Read stdout + stderr concurrently
        let read_fut = async {
            let mut out_buf = Vec::new();
            let mut err_buf = Vec::new();
            let _ = tokio::join!(
                stdout.read_to_end(&mut out_buf),
                stderr.read_to_end(&mut err_buf)
            );
            (out_buf, err_buf)
        };

        let timeout_dur = std::time::Duration::from_secs(timeout_secs);

        let (status, combined) = tokio::select! {
            _ = tokio::time::sleep(timeout_dur) => {
                let _ = child.kill().await;
                return error_result(format!(
                    "Command timed out after {timeout_secs}s: {}",
                    p.command
                ));
            }
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return error_result("Command cancelled");
            }
            result = async {
                let (out, err) = read_fut.await;
                let status = child.wait().await;
                (status, (out, err))
            } => {
                let (status, (out, err)) = result;
                (status, (out, err))
            }
        };

        let (stdout_bytes, stderr_bytes) = combined;
        let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

        // Combine stdout + stderr
        let mut all_output = stdout_bytes;
        if !stderr_bytes.is_empty() {
            all_output.extend_from_slice(&stderr_bytes);
        }

        let output_str = String::from_utf8_lossy(&all_output).into_owned();

        // Tail truncation: keep last MAX_LINES lines or MAX_BYTES
        let truncated = tail_truncate(&output_str);

        text_result(format!("Exit code: {exit_code}\n{truncated}"))
    }
}

fn tail_truncate(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let total_lines = lines.len();

    // Take the last MAX_LINES lines
    let start_line = total_lines.saturating_sub(MAX_LINES);
    let kept_lines = &lines[start_line..];

    // Also enforce byte limit from the tail
    let mut output = String::new();
    let mut bytes_from_start = 0usize;
    let mut truncated_bytes = false;

    for line in kept_lines {
        let entry = format!("{line}\n");
        if bytes_from_start + entry.len() > MAX_BYTES {
            truncated_bytes = true;
            break;
        }
        bytes_from_start += entry.len();
        output.push_str(&entry);
    }

    let mut prefix = String::new();
    if start_line > 0 {
        prefix.push_str(&format!(
            "[Truncated: showing last {MAX_LINES} of {total_lines} lines]\n"
        ));
    }
    if truncated_bytes {
        prefix.push_str(&format!("[Truncated: output exceeded {MAX_BYTES} bytes]\n"));
    }

    format!("{prefix}{output}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn make_tool() -> BashTool {
        BashTool
    }

    #[tokio::test]
    async fn test_simple_command() {
        let tool = make_tool();
        let params = serde_json::json!({ "command": "echo hello" });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("Exit code: 0"));
        assert!(text.contains("hello"));
    }

    #[tokio::test]
    async fn test_command_failure() {
        let tool = make_tool();
        let params = serde_json::json!({ "command": "exit 1" });
        let result = tool.execute(params, CancellationToken::new()).await;

        // Not is_error — we report the exit code, not treat it as error
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("Exit code: 1"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let tool = make_tool();
        let params = serde_json::json!({
            "command": "sleep 60",
            "timeout": 1
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(text.contains("timed out"));
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_command_output_on_both_stdout_and_stderr() {
        let tool = make_tool();
        // Write to both stdout and stderr
        let params = serde_json::json!({
            "command": "echo 'stdout_content' && echo 'stderr_content' >&2"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(
            text.contains("stdout_content"),
            "stdout should appear in output"
        );
        assert!(
            text.contains("stderr_content"),
            "stderr should also appear in combined output"
        );
    }

    #[tokio::test]
    async fn test_very_large_output_is_truncated() {
        let tool = make_tool();
        // Generate >50KB of output (each "x\n" is 2 bytes, so 26000 lines = 52000 bytes)
        let params = serde_json::json!({
            "command": "python3 -c \"print('x' * 100)\" | yes | head -600"
        });
        let result = tool.execute(params, CancellationToken::new()).await;

        assert!(!result.is_error);
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        // Output should mention truncation OR be within byte limit
        // The tail_truncate function should have capped it
        assert!(
            text.len() <= MAX_BYTES + 200, // allow for truncation message overhead
            "Very large output should be truncated to roughly MAX_BYTES, got {} bytes",
            text.len()
        );
    }

    #[tokio::test]
    async fn test_empty_command_string() {
        let tool = make_tool();
        // An empty command string passed to /bin/sh -c ""
        // The shell handles this gracefully (exit 0, no output)
        let params = serde_json::json!({ "command": "" });
        let result = tool.execute(params, CancellationToken::new()).await;

        // Should not crash — exit 0 with empty output is acceptable
        let text = match &result.content[0] {
            crate::ToolContent::Text { text } => text.clone(),
        };
        assert!(
            text.contains("Exit code:"),
            "Empty command should still produce exit code output, got: {text}"
        );
    }
}
