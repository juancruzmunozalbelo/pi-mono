## 1. Project Scaffold

- [ ] 1.1 Create root `Cargo.toml` workspace manifest listing all 5 member crates (`pi-ai`, `pi-agent`, `pi-tui`, `pi-tools`, `pi-cli`)
- [ ] 1.2 Pin Rust stable toolchain via `rust-toolchain.toml` and configure `.cargo/config.toml` (target defaults, linker flags for static linking)
- [ ] 1.3 Create skeleton `Cargo.toml` for each of the 5 crates with correct dependency declarations and `[lib]` / `[[bin]]` targets
- [ ] 1.4 Add `crates/pi-types` shared module (or inline in `pi-ai`) defining `Message`, `Model`, `ChatEvent`, `Usage`, `ToolCall`, `ToolResult` types with `serde` derives
- [ ] 1.5 Add GitHub Actions CI workflow: `cargo check`, `cargo clippy -- -D warnings`, `cargo test` on `ubuntu-latest` and `macos-latest`

## 2. Core Types & SSE Parser

- [ ] 2.1 Define `AgentMessage` enum variants: `User`, `Assistant`, `ToolResult`, `ToolCall` with full `serde` serialization
- [ ] 2.2 Define `ChatEvent` enum: `TextDelta`, `ThinkingDelta`, `ToolCallDelta`, `ToolCallDone`, `StreamDone`, `Error` with associated data fields
- [ ] 2.3 Define `Model`, `ChatRequest`, and `Usage` structs (input/output/cache_read/cache_write token counts)
- [ ] 2.4 Implement SSE line parser: buffers partial lines, skips comment lines (`:` prefix) and empty lines, extracts `data:` payload; handles both OpenAI and Anthropic envelope formats
- [ ] 2.5 Write unit tests for the SSE parser using recorded golden SSE response fixtures for both providers (including partial chunks, `[DONE]` sentinel, and malformed lines)

## 3. LLM Providers (pi-ai)

- [ ] 3.1 Define `LlmProvider` async trait: `async fn chat(&self, request: ChatRequest) -> Result<Pin<Box<dyn Stream<Item = Result<ChatEvent>> + Send>>>`
- [ ] 3.2 Implement `OpenAICompletionsProvider`: serialize `ChatRequest` to OpenAI `messages` + `tools` wire format
- [ ] 3.3 Implement OpenAI SSE response parsing: accumulate `tool_calls` argument deltas, emit `ToolCallDone` when the call closes, emit `TextDelta` per content delta
- [ ] 3.4 Implement GitHub Copilot header injection in `OpenAICompletionsProvider`: `Authorization: Bearer <token>`, `editor-version`, `Copilot-Integration-Id`
- [ ] 3.5 Implement `AnthropicMessagesProvider`: serialize `ChatRequest` to Anthropic `messages` + `tools` wire format including `thinking` support
- [ ] 3.6 Implement Anthropic SSE response parsing: handle `content_block_delta` (text and thinking), `tool_use` blocks, emit typed `ChatEvent` variants
- [ ] 3.7 Implement retry logic (exponential back-off with jitter) for HTTP 429 and 5xx; respect `Retry-After` header and fail immediately if it exceeds `max_retry_delay_ms`
- [ ] 3.8 Implement provider registry: select `Arc<dyn LlmProvider>` by name string at startup
- [ ] 3.9 Write integration tests for both providers using a local mock HTTP server (recorded request/response pairs)

## 4. GitHub Copilot Auth

- [ ] 4.1 Implement `start_device_flow()`: POST to `{github_base}/login/device/code`, return `DeviceFlowState` with `user_code`, `verification_uri`, `device_code`, `interval`
- [ ] 4.2 Implement `poll_for_token()`: poll `{github_base}/login/oauth/access_token` at the specified interval until success, `expires_in` deadline, or terminal error
- [ ] 4.3 Implement Copilot token exchange: POST GitHub OAuth token to the Copilot token endpoint, cache the short-lived Copilot API token with its expiry timestamp
- [ ] 4.4 Implement `get_token()`: return cached Copilot token if still valid; otherwise re-exchange the stored OAuth token transparently (use `tokio::sync::RwLock` to prevent refresh races)
- [ ] 4.5 Implement credential persistence: write/read `~/.pi/credentials.toml` with `0600` permissions; skip device flow on startup when valid credentials exist
- [ ] 4.6 Support `github_api_base_url` config override for enterprise GitHub instances

## 5. Built-in Tools (pi-tools)

- [ ] 5.1 Define `Tool` trait: `fn name(&self) -> &str`, `fn schema(&self) -> serde_json::Value`, `async fn execute(&self, params: serde_json::Value, cancel: CancellationToken) -> ToolResult`
- [ ] 5.2 Implement `read_file`: return file contents with 1-based `<n>\t<content>` line prefixes; support `offset` and `limit` parameters; return `is_error: true` on missing file
- [ ] 5.3 Implement `write_file`: create or overwrite file, auto-create parent directories via `tokio::fs::create_dir_all`, return byte count on success
- [ ] 5.4 Implement `edit_file`: locate first occurrence of `old_string`, fail with occurrence count if ambiguous (>1) or absent (0), replace in-place atomically
- [ ] 5.5 Implement `bash`: spawn `/bin/sh -c <cmd>` via `tokio::process`, capture combined stdout/stderr, enforce `timeout_ms`, kill process and return `is_error: true` on timeout or cancellation
- [ ] 5.6 Implement `grep`: regex search across files under a path, support `context` lines (before/after) and `glob` filter, return `<file>:<line>:<content>` format with `--` separators between groups
- [ ] 5.7 Implement `find`: glob-pattern search rooted at a directory, sort results by modification time descending, respect `head_limit`
- [ ] 5.8 Implement `ls`: list immediate directory entries with name, type, size in bytes, and ISO-8601 modified timestamp; return `is_error: true` if path is a file
- [ ] 5.9 Write unit tests for each tool covering the happy path, error cases, and cancellation where applicable

## 6. Agent Runtime (pi-agent)

- [ ] 6.1 Implement conversation transcript management: ordered `Vec<AgentMessage>`, atomic append before event emission, `reset()` clears messages and streaming state
- [ ] 6.2 Implement `Sequential` tool execution mode: run tool calls one at a time in order, appending each result before starting the next
- [ ] 6.3 Implement `Parallel` tool execution mode: run all tool calls in the same assistant turn concurrently via `tokio::join`, append results in source order
- [ ] 6.4 Define and emit `AgentEvent` stream: `AgentStart`, `TurnStart`, `MessageStart`, `MessageUpdate`, `MessageEnd`, `ToolExecutionStart`, `ToolExecutionEnd`, `TurnEnd`, `AgentEnd`
- [ ] 6.5 Implement `before_tool_call` hook: invoke async callback before each tool; block execution and emit error result when hook returns `block: true`
- [ ] 6.6 Implement `after_tool_call` hook: invoke async callback after each tool; allow hook to override `content`, `details`, or `is_error` fields before the result is emitted
- [ ] 6.7 Implement `transform_context` hook: invoke async callback before each LLM request; use the returned (possibly pruned) message slice for the wire request while keeping the full transcript in state
- [ ] 6.8 Implement `steer(message)` and `follow_up(message)` queues: steer injects after the current turn's tools complete; follow-up injects only when the agent would otherwise stop
- [ ] 6.9 Implement `abort()` via `CancellationToken`: cancel in-progress tool executions and provider stream, end the run with `stop_reason: "aborted"`
- [ ] 6.10 Write integration tests using a mock `LlmProvider`: cover sequential/parallel tools, hook overrides, steer/follow-up, and abort mid-run

## 7. Terminal UI (pi-tui)

- [ ] 7.1 Define app state struct and main event loop: multiplex `crossterm` keyboard events and `AgentEvent` channel using `tokio::select!`, drive `ratatui` render on each update
- [ ] 7.2 Implement scrollable message list widget: render user and assistant messages, support `PgUp`/`PgDn` scrolling
- [ ] 7.3 Implement Markdown rendering for assistant messages: bold, italic, inline code, fenced code blocks with syntax highlighting and a visible block border
- [ ] 7.4 Implement streaming token display: append `TextDelta` content in the current render frame without flickering preceding content
- [ ] 7.5 Implement thinking/reasoning block widget: render `ThinkingDelta` content in a collapsed "Reasoning" panel showing label and character count; expandable by user
- [ ] 7.6 Implement collapsible tool output panels: show tool name and first output line when collapsed; expand to full output on key press; auto-collapse on `ToolExecutionEnd`
- [ ] 7.7 Implement multiline input editor: `Enter` inserts newline and grows editor height; configurable submit chord (default `Ctrl+Enter`) dispatches message and resets to single line
- [ ] 7.8 Implement status bar: display active model name, cumulative session token count, and session ID; show streaming indicator during agent run; update within the same frame as `TurnEnd`
- [ ] 7.9 Implement theming: load `[theme]` section from `~/.pi/config.toml` (foreground, background, accent, code_bg); fall back to built-in default theme when absent
- [ ] 7.10 Implement keyboard shortcuts: `Ctrl+C` aborts active run or quits when idle; `Ctrl+L` clears conversation view; `Tab` cycles focus between input editor and message list

## 8. CLI & Session Management (pi-cli)

- [ ] 8.1 Implement `clap` argument parsing: `--model <id>`, `--session <id>`, `--mode <interactive|print>`, `--prompt <text>` flags with proper help text and validation
- [ ] 8.2 Implement config loading from `~/.pi/config.toml` at startup: treat missing file as empty config with defaults; emit warning (not error) for unknown keys
- [ ] 8.3 Implement session create and resume: load `~/.pi/sessions/<id>.json` when `--session` is provided and file exists; create a new empty transcript otherwise; persist on first message
- [ ] 8.4 Implement `pi sessions` subcommand: list all `~/.pi/sessions/*.json` sorted by last-modified, print `<id>  <model>  <count> messages  <last-modified>`; print "No sessions found." when empty
- [ ] 8.5 Implement interactive mode: wire `pi-tui` app loop with `pi-agent` runtime and configured provider; block until user quits
- [ ] 8.6 Implement print mode: read prompt from `--prompt` or stdin, run agent to completion, write final assistant message to stdout; exit 1 with stderr message on `stop_reason: "error"`
- [ ] 8.7 Implement `/model <id>` slash command in interactive mode: update `agent.state.model` and refresh status bar without ending the session

## 9. Integration & Polish

- [ ] 9.1 Write end-to-end test: `pi --mode print --prompt "list files"` against a mock provider and tool registry, assert stdout and exit code
- [ ] 9.2 Verify CI matrix covers `ubuntu-latest` and `macos-latest`; fix any platform-specific failures in the `bash` tool or path handling
- [ ] 9.3 Audit and polish user-facing error messages: provider auth failures, missing config, session load errors, tool permission errors
- [ ] 9.4 Configure `cargo build --release` with `strip = true` and `opt-level = "z"` in `[profile.release]`; measure and document binary size
- [ ] 9.5 Add GitHub Actions release workflow: build matrix for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`; upload binaries as release artifacts
