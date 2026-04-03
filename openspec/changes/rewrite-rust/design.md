## Context

Pi is a TypeScript monorepo with 7 packages (`pi-ai`, `pi-agent-core`, `pi-tui`, `pi-coding-agent`, `pi-mom`, `pi-pods`, `pi-web-ui`) that requires Node.js at runtime. Startup latency is 300–500ms and distribution requires npm. The current implementation uses undici for HTTP, jiti for loading extensions, and a custom TUI renderer.

Only two LLM providers are actively used: GitHub Copilot (accessed via the OpenAI Completions API format with OAuth device code auth) and MiniMax (accessed via the Anthropic Messages API format with SSE streaming).

The rewrite produces a single statically-linked Rust binary distributed via GitHub Releases or `cargo install`, with sub-10ms startup and no runtime dependency on Node.js.

## Goals / Non-Goals

**Goals:**
- Single binary, sub-10ms startup, low memory footprint
- Full feature parity for the core agent loop: tool execution, SSE streaming, TUI, session management
- Support exactly two providers: GitHub Copilot (OpenAI Completions format) and MiniMax (Anthropic Messages format)
- Clean Cargo workspace architecture with independently testable crates
- Provider trait abstraction that makes adding providers in v2 straightforward

**Non-Goals:**
- Extension/plugin system (was `pi-pods` — deferred to v2)
- Web UI (`pi-web-ui` — retired)
- Message broker / orchestration layer (`pi-mom` — retired)
- MCP (Model Context Protocol) support
- Backward compatibility with existing TS config files or session files
- Supporting any provider beyond the two listed above
- Slack bot or any non-CLI interface

## Decisions

### 1. Crate architecture

Cargo workspace with 5 crates:

| Crate | Responsibility |
|---|---|
| `pi-ai` | `LlmProvider` trait + implementations for OpenAI Completions and Anthropic Messages |
| `pi-agent` | Agent runtime, `Tool` trait, event stream, conversation context loop |
| `pi-tui` | ratatui-based terminal UI — widgets, input handling, rendering loop |
| `pi-tools` | Built-in tool implementations: `read_file`, `write_file`, `edit_file`, `bash`, `grep`, `find`, `ls` |
| `pi-cli` | Binary entry point: clap arg parsing, session creation/resume, config loading |

This mirrors the layered architecture of the TS monorepo. Each crate has a single clear responsibility and can be tested in isolation. `pi-cli` is the only crate that depends on all others; the rest form a directed acyclic dependency graph (`pi-ai` and `pi-tools` have no internal crate dependencies, `pi-agent` depends on `pi-ai` and `pi-tools`, `pi-tui` depends on `pi-agent`).

### 2. Async runtime

`tokio` with the multi-threaded scheduler.

`reqwest` requires tokio, and `crossterm` supports async event polling under tokio. Multi-threaded is chosen over `current_thread` because tool execution (especially `bash`) can block and benefits from worker thread offloading via `spawn_blocking`.

### 3. SSE streaming

`reqwest::Response::bytes_stream()` fed into a custom line-based SSE parser, not the `eventsource` crate.

GitHub Copilot and MiniMax use slightly different SSE envelope formats (field ordering, `[DONE]` sentinel, error event shapes). A thin custom parser (~80 lines) gives full control over both formats and avoids pulling in a crate that only handles the RFC 8895 happy path. Typed events are deserialized via `serde_json` after the `data:` line is extracted.

### 4. Provider abstraction

Trait-based, one implementation per provider:

```rust
#[async_trait]
trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatEvent>> + Send>>>;
}
```

`ChatRequest` is a provider-agnostic struct (messages, tools, sampling params). Each provider implementation translates it to the wire format, handles auth headers, and maps the SSE stream back to `ChatEvent` variants (`TokenDelta`, `ToolCall`, `Done`, `Error`). Provider selection at startup is done once; the agent runtime holds a `Arc<dyn LlmProvider>`.

### 5. Tool system

Trait-based with dynamic dispatch via a registry:

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute(
        &self,
        params: serde_json::Value,
        cancel: CancellationToken,
    ) -> ToolResult;
}
```

Tools are registered at startup into a `HashMap<String, Arc<dyn Tool>>`. The agent runtime looks up tools by name when the provider emits a `ToolCall` event. `CancellationToken` (from `tokio-util`) allows the TUI to cancel in-flight tool execution on user interrupt. `ToolResult` carries structured output (stdout, stderr, exit code, or file content) that both the agent and TUI can render.

### 6. TUI framework

`ratatui` + `crossterm`.

ratatui is the de-facto standard for terminal UIs in Rust: mature, actively maintained, handles differential rendering without a full redraw per frame. crossterm provides cross-platform raw mode and async event polling under tokio. Custom widgets are written for: scrollable message list, tool output panel, single-line / multi-line editor input, and a status bar showing provider, model, and token count.

### 7. Error handling

`anyhow` in `pi-cli` and `pi-tui` (application crates); `thiserror` in `pi-ai`, `pi-agent`, and `pi-tools` (library crates).

Library crates expose typed error enums via `thiserror` so callers can match on specific variants (e.g., `ProviderError::RateLimit`, `ToolError::PermissionDenied`). Application crates use `anyhow` for ergonomic error propagation with `?` and context chains in display output.

### 8. Config format

TOML at `~/.pi/config.toml`.

TOML is the standard config format in the Rust ecosystem, has first-class `serde` support via `toml`, and is human-readable/editable. The config schema covers: active provider, model name, system prompt path, and per-provider overrides. Credentials (OAuth tokens) are stored separately in `~/.pi/credentials.toml` with `0600` permissions.

### 9. Session storage

One JSON file per session under `~/.pi/sessions/<id>.json`.

Simple, inspectable, and grep-friendly for debugging. The session file stores the full message tree (including tool call / tool result pairs) in the same logical structure as the TS version, but with a new schema — no backward compatibility is preserved. Session IDs are UUIDs v4.

### 10. GitHub Copilot auth

OAuth device code flow implemented directly with `reqwest`, no SDK dependency.

The flow is straightforward (two HTTP calls + polling loop) and was already implemented in TS without a library. Implementing it in ~150 lines of Rust avoids a heavy OAuth crate dependency. Access token and refresh token are persisted to `~/.pi/credentials.toml` and refreshed automatically when a 401 is received.

## Risks / Trade-offs

- [SSE parsing edge cases across providers] → Comprehensive test suite with recorded real responses (golden files) for both providers; fuzz the parser against malformed input.

- [ratatui rendering differences from the custom TS TUI] → Ship a simpler initial UI (linear message list, no split-pane) and iterate toward feature parity in follow-up PRs; the TS TUI is not a hard dependency for v1 launch.

- [Copilot token refresh races under concurrent requests] → Store credentials behind a `tokio::sync::RwLock`; only one task acquires the write lock to refresh, others wait and re-read.

- [Cross-platform bash tool behavior (macOS vs Linux)] → Abstract process spawning behind a thin `ProcessRunner` trait; CI matrix covers `ubuntu-latest` and `macos-latest` for every PR.

- [No extension system limits adoption for power users] → Explicit v1 scope decision, documented in the proposal. Crate boundaries (`pi-tools` as a separate crate with a public `Tool` trait) are designed so a future extension loader can register external tools without modifying core crates.

- [Full codebase replacement increases review and rollback risk] → Keep the TS monorepo on a maintenance branch until the Rust binary reaches feature parity on the two providers; gate the cutover on a checklist rather than a date.
