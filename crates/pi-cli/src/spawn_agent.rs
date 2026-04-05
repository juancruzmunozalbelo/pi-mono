//! SpawnAgentTool — lets the orchestrator delegate tasks to an ephemeral sub-agent.
//!
//! This tool lives in `pi-cli` (not `pi-tools`) to avoid the circular dependency that
//! would arise if `pi-tools` depended on `pi-agent` which already depends on `pi-tools`.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use pi_agent::{Agent, AgentConfig, AgentEvent, AgentState, ToolExecutionMode};
use pi_ai::{ApiType, ChatEvent, Model, ModelCost};
use pi_tools::{error_result, text_result, Tool, ToolResult};

// ─── SubAgentConfig ───────────────────────────────────────────────────────────

/// Configuration for the ephemeral sub-agent spawned by `SpawnAgentTool`.
pub struct SubAgentConfig {
    /// LLM provider for sub-agents (e.g., MiniMax).
    pub provider: Arc<dyn pi_ai::LlmProvider>,
    /// Model to use for sub-agents.
    pub model: Model,
    /// Tools available to the sub-agent (typically the same 7 basic tools).
    pub tools: Vec<Arc<dyn Tool>>,
    /// Optional system prompt override for the sub-agent.
    pub system_prompt: Option<String>,
}

// ─── SpawnAgentTool ───────────────────────────────────────────────────────────

/// A tool that spawns an ephemeral sub-agent to handle a task autonomously.
pub struct SpawnAgentTool {
    sub_config: Arc<SubAgentConfig>,
}

impl SpawnAgentTool {
    pub fn new(sub_config: Arc<SubAgentConfig>) -> Self {
        Self { sub_config }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a task autonomously. The sub-agent has access to \
         file tools (read, write, edit, bash, grep, find, ls) and runs on a fast model. \
         Use this for implementation tasks while you focus on planning and coordination."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task description for the sub-agent. Be specific: include \
                                    file paths, what to read, what to write, and verification steps."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context to prepend to the task. Use this to pass \
                                    relevant code snippets, types, or constraints the sub-agent needs."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: serde_json::Value, cancel: CancellationToken) -> ToolResult {
        // ── 1. Parse parameters ───────────────────────────────────────────────
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

        // ── 2. Build ephemeral sub-agent ──────────────────────────────────────
        let system_prompt = self.sub_config.system_prompt.clone().or_else(|| {
            Some(
                "You are a coding sub-agent. Execute the task precisely. \
                 Use tools to read files, write code, and verify with cargo check. \
                 Be concise in your final response."
                    .to_string(),
            )
        });

        let config = AgentConfig {
            provider: Arc::clone(&self.sub_config.provider),
            tools: self.sub_config.tools.clone(),
            tool_execution_mode: ToolExecutionMode::Sequential,
            hooks: pi_agent::Hooks::default(),
        };

        let state = AgentState {
            messages: vec![],
            model: self.sub_config.model.clone(),
            system_prompt,
            thinking_level: None,
            is_streaming: false,
            error_message: None,
        };

        let mut agent = Agent::new(config, state);
        let mut event_rx = match agent.take_event_receiver() {
            Some(rx) => rx,
            None => return error_result("Failed to acquire sub-agent event receiver"),
        };

        // ── 3. Run with a 5-minute timeout, draining events concurrently ──────
        let mut result_text = String::new();

        let run_fut = async {
            // Spawn agent.prompt so we can drain events concurrently.
            let prompt_handle = tokio::spawn(async move { agent.prompt(user_message).await });

            // Drain events, collecting text deltas.
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        prompt_handle.abort();
                        return Err("Cancelled".to_string());
                    }
                    event = event_rx.recv() => {
                        match event {
                            Some(AgentEvent::MessageUpdate {
                                event: ChatEvent::TextDelta { text },
                            }) => {
                                result_text.push_str(&text);
                            }
                            Some(AgentEvent::AgentEnd { .. }) => break,
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
            }

            match prompt_handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(format!("Sub-agent error: {e}")),
                Err(e) => Err(format!("Sub-agent join error: {e}")),
            }
        };

        match tokio::time::timeout(std::time::Duration::from_secs(300), run_fut).await {
            Ok(Ok(())) => {
                if result_text.is_empty() {
                    text_result("Sub-agent completed but produced no text output.")
                } else {
                    text_result(result_text)
                }
            }
            Ok(Err(msg)) => error_result(msg),
            Err(_) => error_result("Sub-agent timed out after 5 minutes"),
        }
    }
}

// ─── Helper: build a MiniMax Model from config values ────────────────────────

/// Construct a `pi_ai::Model` for the MiniMax sub-agent provider.
pub fn minimax_model(model_id: &str) -> Model {
    Model {
        id: model_id.to_string(),
        name: "MiniMax Sub-Agent".to_string(),
        provider: "minimax".to_string(),
        api: ApiType::AnthropicMessages,
        base_url: "https://api.minimax.io/anthropic".to_string(),
        reasoning: false,
        input_types: vec!["text".to_string()],
        cost: ModelCost::default(),
        context_window: 204800,
        max_tokens: 131072,
        headers: Default::default(),
    }
}
