//! EscalateReviewTool — escalates to a powerful model (Sonnet 4.6) for
//! ultra-complex tasks or final code review.
//!
//! Uses the SAME provider as the orchestrator (GitHub Copilot) but with a
//! different model ID (claude-sonnet-4-6). This costs 1x premium request
//! vs 0.33x for the orchestrator's GPT-5.4-mini.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use pi_agent::{Agent, AgentConfig, AgentState, ToolExecutionMode};
use pi_ai::{ApiType, ChatEvent, Model, ModelCost};
use pi_tools::{error_result, text_result, Tool, ToolResult};

/// Configuration for the escalation reviewer.
pub struct EscalationConfig {
    /// The LLM provider (same as orchestrator — GitHub Copilot).
    pub provider: Arc<dyn pi_ai::LlmProvider>,
    /// Tools available to the reviewer (basic 7, no spawn_agent, no escalate).
    pub tools: Vec<Arc<dyn Tool>>,
}

pub struct EscalateReviewTool {
    config: Arc<EscalationConfig>,
}

impl EscalateReviewTool {
    pub fn new(config: Arc<EscalationConfig>) -> Self {
        Self { config }
    }
}

/// Build a Sonnet 4.6 model via GitHub Copilot.
fn sonnet_model() -> Model {
    Model {
        id: "claude-sonnet-4-6".to_string(),
        name: "Sonnet 4.6 (Escalation)".to_string(),
        provider: "github-copilot".to_string(),
        api: ApiType::OpenaiCompletions,
        base_url: "https://api.githubcopilot.com".to_string(),
        reasoning: false,
        input_types: vec!["text".to_string()],
        cost: ModelCost::default(),
        context_window: 200000,
        max_tokens: 16384,
        headers: Default::default(),
    }
}

#[async_trait]
impl Tool for EscalateReviewTool {
    fn name(&self) -> &str {
        "escalate_review"
    }

    fn description(&self) -> &str {
        "Escalate to a powerful reviewer model (Claude Sonnet 4.6) for ultra-complex tasks, \
         architectural decisions, subtle bug analysis, or final code review. Costs 3x more \
         than spawn_agent — only use when MiniMax sub-agents have failed or the task requires \
         deep reasoning."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The complex task or review request. Be specific about what \
                                    needs deep analysis and what outcome you expect."
                },
                "context": {
                    "type": "string",
                    "description": "Code, error messages, or prior sub-agent output that the \
                                    reviewer needs to analyze."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: serde_json::Value, cancel: CancellationToken) -> ToolResult {
        let task = match params.get("task").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => return error_result("Missing required parameter: task"),
        };

        let context = params.get("context").and_then(|c| c.as_str()).unwrap_or("");

        let user_message = if context.is_empty() {
            task
        } else {
            format!("## Context\n{context}\n\n## Task\n{task}")
        };

        let system_prompt = Some(
            "You are a senior code reviewer and architect. You are called only for \
             ultra-complex tasks that simpler models couldn't handle. Be thorough, \
             precise, and opinionated. Point out subtle bugs, architectural issues, \
             and suggest concrete improvements with code examples."
                .to_string(),
        );

        let config = AgentConfig {
            provider: Arc::clone(&self.config.provider),
            tools: self.config.tools.clone(),
            tool_execution_mode: ToolExecutionMode::Sequential,
            hooks: pi_agent::Hooks::default(),
        };

        let state = AgentState {
            messages: vec![],
            model: sonnet_model(),
            system_prompt,
            thinking_level: None,
            is_streaming: false,
            error_message: None,
        };

        let mut agent = Agent::new(config, state);
        let mut event_rx = match agent.take_event_receiver() {
            Some(rx) => rx,
            None => return error_result("Failed to acquire escalation agent event receiver"),
        };

        let mut result_text = String::new();

        let run_fut = async {
            let prompt_handle = tokio::spawn(async move { agent.prompt(user_message).await });

            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        prompt_handle.abort();
                        return Err("Cancelled".to_string());
                    }
                    event = event_rx.recv() => {
                        match event {
                            Some(pi_agent::AgentEvent::MessageUpdate {
                                event: ChatEvent::TextDelta { text },
                            }) => {
                                result_text.push_str(&text);
                            }
                            Some(pi_agent::AgentEvent::AgentEnd { .. }) => break,
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
            }

            match prompt_handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(format!("Escalation error: {e}")),
                Err(e) => Err(format!("Escalation join error: {e}")),
            }
        };

        // 10 minute timeout for complex reviews
        match tokio::time::timeout(std::time::Duration::from_secs(600), run_fut).await {
            Ok(Ok(())) => {
                if result_text.is_empty() {
                    text_result("Reviewer completed but produced no text output.")
                } else {
                    text_result(result_text)
                }
            }
            Ok(Err(msg)) => error_result(msg),
            Err(_) => error_result("Escalation review timed out after 10 minutes"),
        }
    }
}
