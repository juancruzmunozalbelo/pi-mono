#![allow(dead_code)]
use pi_agent::AgentEvent;
use std::io::Write;
use std::time::{Duration, Instant};

/// Inactivity timeout before marking run as timed out (used in future timer integration).
#[allow(dead_code)]
const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(180);

pub struct TabStatus {
    dir_name: String,
    pub(crate) running: bool,
    pub(crate) saw_commit: bool,
    pub(crate) last_activity: Instant,
}

impl TabStatus {
    pub fn new() -> Self {
        let dir_name = std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "pi".to_string());

        Self {
            dir_name,
            running: false,
            saw_commit: false,
            last_activity: Instant::now(),
        }
    }

    /// Update tab title using OSC escape sequence.
    /// No-op if stdout is not a terminal.
    fn set_title(&self, status: &str) {
        use std::io::IsTerminal;
        if !std::io::stdout().is_terminal() {
            return;
        }
        let title = format!("pi - {}{}", self.dir_name, status);
        let _ = write!(std::io::stdout(), "\x1b]0;{title}\x07");
        let _ = std::io::stdout().flush();
    }

    /// Process an agent event and update tab title accordingly.
    pub fn handle_event(&mut self, event: &AgentEvent) {
        self.last_activity = Instant::now();

        match event {
            AgentEvent::AgentStart => {
                self.running = true;
                self.saw_commit = false;
                self.set_title(":running...");
            }
            AgentEvent::ToolExecutionStart { tool_name, .. } => {
                // Detect git commit in bash tool calls
                if tool_name == "bash" {
                    // We'll check the actual command in ToolExecutionEnd
                }
            }
            AgentEvent::ToolExecutionEnd {
                tool_name, result, ..
            } => {
                if tool_name == "bash" {
                    // Check result content for git commit pattern
                    for content in &result.content {
                        let pi_tools::ToolContent::Text { text } = content;
                        if regex_lite_git_commit(text) {
                            self.saw_commit = true;
                        }
                    }
                }
            }
            AgentEvent::AgentEnd { stop_reason } => {
                self.running = false;
                let status = match stop_reason {
                    pi_ai::StopReason::Error | pi_ai::StopReason::Aborted => ":🛑",
                    _ if self.saw_commit => ":✅",
                    _ => ":🚧",
                };
                self.set_title(status);
            }
            _ => {}
        }
    }

    /// Check if inactivity timeout has been exceeded.
    /// Call this periodically from the event loop.
    #[allow(dead_code)]
    pub fn check_timeout(&mut self) {
        if self.running && self.last_activity.elapsed() > INACTIVITY_TIMEOUT {
            self.running = false;
            self.set_title(":🛑");
        }
    }

    /// Reset title on exit.
    pub fn reset(&self) {
        self.set_title("");
    }
}

/// Simple check for git commit pattern without pulling in regex crate.
fn regex_lite_git_commit(text: &str) -> bool {
    // Check if text contains "git" followed by "commit" (possibly with flags between)
    if let Some(git_pos) = text.find("git") {
        let after_git = &text[git_pos..];
        after_git.contains("commit")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_agent::AgentEvent;
    use pi_ai::StopReason;
    use pi_tools::{ToolContent, ToolResult};

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_tool_result(text: &str) -> ToolResult {
        ToolResult {
            content: vec![ToolContent::Text {
                text: text.to_string(),
            }],
            is_error: false,
        }
    }

    // ── new() ─────────────────────────────────────────────────────────────────

    #[test]
    fn new_sets_dir_name() {
        let ts = TabStatus::new();
        // dir_name should be non-empty (actual value depends on cwd which other tests may change)
        assert!(!ts.dir_name.is_empty(), "dir_name should not be empty");
        assert!(!ts.running);
        assert!(!ts.saw_commit);
    }

    // ── AgentStart ────────────────────────────────────────────────────────────

    #[test]
    fn agent_start_sets_running() {
        let mut ts = TabStatus::new();
        ts.saw_commit = true; // should be reset
        ts.handle_event(&AgentEvent::AgentStart);
        assert!(ts.running, "running should be true after AgentStart");
        assert!(
            !ts.saw_commit,
            "saw_commit should be reset to false on AgentStart"
        );
    }

    // ── AgentEnd ──────────────────────────────────────────────────────────────

    #[test]
    fn agent_end_with_commit_shows_checkmark_state() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        ts.saw_commit = true;
        ts.handle_event(&AgentEvent::AgentEnd {
            stop_reason: StopReason::Stop,
        });
        assert!(!ts.running, "running should be false after AgentEnd");
        assert!(
            ts.saw_commit,
            "saw_commit should remain true (commit happened)"
        );
    }

    #[test]
    fn agent_end_without_commit_shows_construction_state() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        // saw_commit stays false
        ts.handle_event(&AgentEvent::AgentEnd {
            stop_reason: StopReason::Stop,
        });
        assert!(!ts.running);
        assert!(
            !ts.saw_commit,
            "saw_commit should be false when no commit was detected"
        );
    }

    #[test]
    fn agent_end_with_error_sets_not_running() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        ts.handle_event(&AgentEvent::AgentEnd {
            stop_reason: StopReason::Error,
        });
        assert!(!ts.running, "running should be false after Error stop");
    }

    #[test]
    fn agent_end_with_abort_sets_not_running() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        ts.handle_event(&AgentEvent::AgentEnd {
            stop_reason: StopReason::Aborted,
        });
        assert!(!ts.running, "running should be false after Aborted stop");
    }

    // ── git commit detection ──────────────────────────────────────────────────

    #[test]
    fn git_commit_detection_positive() {
        assert!(
            regex_lite_git_commit("git commit -m 'test'"),
            "plain 'git commit' should be detected"
        );
    }

    #[test]
    fn git_commit_detection_with_flags_between_git_and_commit() {
        assert!(
            regex_lite_git_commit("git -c user.name=x commit -m 'test'"),
            "flags between git and commit should still be detected"
        );
    }

    #[test]
    fn git_commit_detection_negative_no_git_keyword() {
        assert!(
            !regex_lite_git_commit("commit changes"),
            "text without 'git' should NOT be detected"
        );
    }

    #[test]
    fn git_commit_detection_negative_committed_not_commit() {
        // "committed" contains "commit" as substring — this IS detected by the current
        // implementation because we only check substrings.  The test documents the
        // actual (known) behavior so a future refactor doesn't silently change it.
        // If the implementation becomes more strict, flip the assertion.
        let result = regex_lite_git_commit("echo committed");
        // Current implementation: "git" not in "echo committed" → false
        assert!(
            !result,
            "'echo committed' has no 'git' prefix → not detected"
        );
    }

    #[test]
    fn git_commit_detection_no_git_word() {
        assert!(
            !regex_lite_git_commit("commit all the things"),
            "no 'git' keyword → should NOT be detected"
        );
    }

    /// Tests that ToolExecutionEnd with tool_name="bash" sets saw_commit when text
    /// contains a git commit pattern.
    #[test]
    fn tool_execution_end_bash_with_git_commit_sets_saw_commit() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        ts.handle_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "call-1".to_string(),
            tool_name: "bash".to_string(),
            result: make_tool_result("$ git commit -m 'fix bug'\n[main abc1234] fix bug"),
        });
        assert!(
            ts.saw_commit,
            "bash result containing git commit should set saw_commit"
        );
    }

    /// ToolExecutionEnd from a non-bash tool should NOT set saw_commit.
    #[test]
    fn tool_execution_end_non_bash_does_not_set_saw_commit() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        ts.handle_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "call-2".to_string(),
            tool_name: "read".to_string(),
            result: make_tool_result("git commit -m 'test'"),
        });
        assert!(
            !ts.saw_commit,
            "non-bash tool result should NOT set saw_commit"
        );
    }

    // ── check_timeout ─────────────────────────────────────────────────────────

    #[test]
    fn check_timeout_marks_stop_when_elapsed() {
        let mut ts = TabStatus::new();
        ts.handle_event(&AgentEvent::AgentStart);
        assert!(ts.running);

        // Backdate last_activity beyond the inactivity threshold (180s).
        ts.last_activity = Instant::now()
            .checked_sub(Duration::from_secs(200))
            .expect("time subtraction should succeed on a modern system");

        ts.check_timeout();
        assert!(
            !ts.running,
            "check_timeout should clear running after 200s inactivity"
        );
    }

    #[test]
    fn check_timeout_no_op_when_not_running() {
        let mut ts = TabStatus::new();
        // running is false by default; backdating should have no effect
        ts.last_activity = Instant::now()
            .checked_sub(Duration::from_secs(200))
            .expect("time subtraction ok");
        ts.check_timeout();
        assert!(
            !ts.running,
            "check_timeout on idle tab should leave running=false"
        );
    }
}
