//! OpenSpec tool — allows the agent to manage specs via the openspec CLI.

use async_trait::async_trait;
use pi_tools::{error_result, text_result, Tool, ToolResult};
use tokio_util::sync::CancellationToken;

pub struct OpenSpecTool;

#[async_trait]
impl Tool for OpenSpecTool {
    fn name(&self) -> &str {
        "openspec"
    }

    fn description(&self) -> &str {
        "Manage specifications with OpenSpec. Commands: \
         'new change <name>' to create a change, \
         'status --change <name>' to check progress, \
         'instructions <artifact> --change <name> --json' to get artifact instructions. \
         Use this for spec-driven development: create specs before implementing."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The openspec command to run, e.g. 'new change my-feature', 'status --change my-feature', 'list'"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _cancel: CancellationToken) -> ToolResult {
        let command = match params.get("command").and_then(|c| c.as_str()) {
            Some(c) => c.to_string(),
            None => return error_result("Missing required parameter: command"),
        };

        // Find openspec binary
        let openspec = which_openspec();
        let Some(bin) = openspec else {
            return error_result(
                "openspec not found in PATH. Install with: npm install -g @fission-ai/openspec@latest",
            );
        };

        // Run openspec command
        let args: Vec<&str> = command.split_whitespace().collect();
        match tokio::process::Command::new(&bin)
            .args(&args)
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined = if stderr.is_empty() {
                    stdout
                } else if stdout.is_empty() {
                    stderr
                } else {
                    format!("{stdout}\n{stderr}")
                };

                if output.status.success() {
                    text_result(if combined.is_empty() {
                        "Command completed successfully.".to_string()
                    } else {
                        combined
                    })
                } else {
                    error_result(format!(
                        "openspec exited with code {}: {}",
                        output.status.code().unwrap_or(-1),
                        combined
                    ))
                }
            }
            Err(e) => error_result(format!("Failed to run openspec: {e}")),
        }
    }
}

/// Find the openspec binary in PATH.
fn which_openspec() -> Option<String> {
    // Check common locations
    for path in &[
        "openspec",
        "/usr/local/bin/openspec",
        "/opt/homebrew/bin/openspec",
    ] {
        if std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Some(path.to_string());
        }
    }

    // Check NVM paths
    if let Ok(home) = std::env::var("HOME") {
        let nvm_path = format!("{home}/.nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(&nvm_path) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin/openspec");
                if candidate.exists() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
    }

    None
}
