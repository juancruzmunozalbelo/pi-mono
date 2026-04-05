## 1. SpawnAgentTool Implementation

- [x] 1.1 Create `crates/pi-cli/src/spawn_agent.rs` with `SubAgentConfig` struct and `SpawnAgentTool` implementing `pi_tools::Tool`
- [x] 1.2 Implement `execute()`: parse task/context params, build ephemeral Agent, run to completion, collect TextDelta events, return as ToolResult
- [x] 1.3 Implement 5-minute timeout via `tokio::time::timeout` wrapping the sub-agent run
- [x] 1.4 Implement cancellation propagation: use parent `CancellationToken` in `tokio::select!` to abort sub-agent when orchestrator is cancelled
- [x] 1.5 Implement error handling: catch provider errors, join errors, and timeouts as error ToolResults
- [x] 1.6 Add `minimax_model()` helper to construct a MiniMax Model with correct base_url and defaults

## 2. Multi-Provider Configuration

- [x] 2.1 Add `SubAgentConfigToml` struct to `crates/pi-cli/src/config.rs` with provider, model, api_key, system_prompt fields
- [x] 2.2 Add `sub_agent: Option<SubAgentConfigToml>` to `Config` struct
- [x] 2.3 Add `--sub-agent-key` CLI flag to `crates/pi-cli/src/cli.rs`
- [x] 2.4 Implement `setup_sub_agent_config()` in `main.rs`: resolve API key from CLI → config → MINIMAX_API_KEY env var
- [x] 2.5 Update `create_tools()` to conditionally include `SpawnAgentTool` when sub-agent config is available

## 3. CLI Wiring

- [x] 3.1 Update `build_agent()` to pass sub-agent config through to tool creation
- [x] 3.2 Update `run_interactive()` / `run_repl()` to rebuild tools with sub-agent config on `/model` switch
- [x] 3.3 Register `spawn_agent` module in `main.rs`

## 4. Verification

- [ ] 4.1 `cargo check --workspace` passes
- [ ] 4.2 `cargo test --workspace` — all 90 tests pass
- [ ] 4.3 `cargo clippy --workspace -- -D warnings` — 0 warnings
- [ ] 4.4 Manual test: `pi -p "use spawn_agent to list files"` with MINIMAX_API_KEY set
- [ ] 4.5 Manual test: run without MINIMAX_API_KEY — spawn_agent tool should not appear
