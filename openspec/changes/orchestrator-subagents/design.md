## Context

Pi currently runs a single `Agent` with a single LLM provider. `AgentConfig` holds the provider, tool list, execution mode, and hooks; `AgentState` holds the conversation history, model, and system prompt. The seven built-in tools (`Read`, `Write`, `Edit`, `Bash`, `Grep`, `Find`, `Ls`) are defined in `pi-tools` and implement the `Tool` trait (`name`, `description`, `schema`, `execute`). All tool wiring happens in `pi-cli/src/main.rs` inside `create_tools()` and `build_agent()`.

The goal is to let a powerful orchestrator model delegate mechanical sub-tasks to a fast, cheap model (e.g. MiniMax M2.7-highspeed) running as an ephemeral agent — mirroring the pattern used by Claude Code with Sonnet/Haiku sub-agents.

## Goals / Non-Goals

**Goals:**
- Add a `spawn_agent` tool that the orchestrator can call to delegate a task to a sub-agent running a different provider/model.
- Sub-agent runs to completion and returns its final text output as the `ToolResult`.
- Support a `[sub_agent]` TOML config section and a `--sub-agent-key` CLI flag so the sub-agent can use a separate API key and model.
- Propagate the orchestrator's `CancellationToken` to the sub-agent so Ctrl-C aborts both.
- Apply a 5-minute hard timeout per sub-agent call.
- Keep `pi-tools` free of any dependency on `pi-agent` (no circular crate dependency).

**Non-Goals:**
- Recursive spawning: sub-agents cannot themselves call `spawn_agent` (v1).
- Streaming sub-agent output to the TUI in real time (v1).
- Budget/cost limits on sub-agent calls (v1).
- Persistent state or conversation history across multiple `spawn_agent` calls.
- Any changes to `pi-tools`, `pi-agent`, or `pi-ai` crates.

## Decisions

### 1. `SpawnAgentTool` lives in `pi-cli`

`pi-tools` already depends on `pi-ai`. `pi-agent` depends on `pi-tools`. If `SpawnAgentTool` were placed in `pi-tools`, it would need to import `pi-agent` to construct and run an `Agent`, creating a circular dependency: `pi-tools → pi-agent → pi-tools`.

The tool instead lives in a new file `crates/pi-cli/src/spawn_agent.rs`. It implements `pi_tools::Tool` (the trait is defined in `pi-tools` and is object-safe) but is constructed and registered only inside `pi-cli`. `create_tools()` in `main.rs` gains a `SpawnAgentTool` entry when sub-agent config is present; sub-agents created inside `execute()` receive only the base seven tools.

### 2. Sub-agents are ephemeral

Each call to `SpawnAgentTool::execute()` constructs a fresh `Agent` (with its own `AgentState`, empty message history, and a new `CancellationToken` child), runs `agent.prompt(task)` to completion, then drops the agent. No state is preserved between calls. The orchestrator is responsible for summarizing and forwarding any context the sub-agent needs via the `task` parameter it passes to `spawn_agent`.

### 3. Sub-agents receive the 7 base tools, not `spawn_agent`

Sub-agents call `create_tools()` directly — the same seven tools the orchestrator has — but `SpawnAgentTool` is not included. This is enforced structurally: `SpawnAgentTool` is only added to the orchestrator's tool list in `build_agent()`, after checking that sub-agent config is available.

### 4. Communication via return value

The sub-agent's output is collected by draining its `AgentEvent` stream: `TextDelta` chunks are concatenated; `AgentEnd` signals completion or error. The full concatenated text is returned as a `ToolResult` with `is_error: false` on success or `is_error: true` on timeout/cancellation/agent error. The orchestrator sees this as a normal tool response and continues its own loop. Sub-agent output is not streamed to the TUI (v1).

### 5. Config structure

A new optional `[sub_agent]` section in `~/.pi/config.toml`:

```toml
[sub_agent]
provider = "minimax"
model    = "MiniMax-Text-01"
api_key  = "..."          # optional; overridden by --sub-agent-key
system_prompt = "..."     # optional
```

`crates/pi-cli/src/config.rs` gains a `SubAgentConfig` struct derived with `serde::Deserialize`. When `[sub_agent]` is absent, `spawn_agent` is not registered and the orchestrator behaves as today. The `--sub-agent-key` CLI flag (added to `cli.rs`) overrides `sub_agent.api_key` at runtime.

### 6. Timeout — 5 minutes per call

`SpawnAgentTool::execute()` wraps the sub-agent run in `tokio::time::timeout(Duration::from_secs(300), ...)`. On expiry the child `CancellationToken` is cancelled, the sub-agent is dropped, and an error `ToolResult` is returned. The timeout value is hardcoded in v1; it will become a `[sub_agent] timeout_secs` config field in a follow-up.

### 7. Cancellation propagation

`SpawnAgentTool` captures the orchestrator's `CancellationToken` (passed into `execute()` as the `cancel` parameter from the `Tool` trait). Before constructing the sub-agent it creates a child token: `let child_cancel = cancel.child_token()`. This is stored in the sub-agent's `Agent::cancel` field. When the orchestrator is aborted (`cancel.cancel()`), the child token fires automatically and the sub-agent's loop exits at its next cancellation checkpoint.

## Risks / Trade-offs

- **Sub-agent timeout too short for complex tasks** — 5 minutes may be insufficient for tasks that require many tool calls or large file operations. Mitigation: make `timeout_secs` configurable in `[sub_agent]` in v2; document the limit clearly in error messages.

- **No visibility into sub-agent progress** — the orchestrator and user see nothing until the sub-agent finishes. Long-running sub-tasks will appear frozen in the TUI. Mitigation: v2 will forward sub-agent `AgentEvent`s to the orchestrator's event channel so the TUI can display them.

- **Context size limits** — the orchestrator must be selective about what context it passes in the `task` string. Passing full file contents or long conversation histories may exhaust the sub-agent model's context window. Mitigation: document this in the tool's schema description; no automatic truncation in v1.

- **Cost control absent** — there are no per-call or per-session budget limits. MiniMax pricing is low enough that this is acceptable for v1, but unrestricted spawning could run up costs if the orchestrator loops incorrectly. Mitigation: add optional `max_calls` and `max_tokens_per_call` to `[sub_agent]` in a follow-up.
