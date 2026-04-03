## Why

Pi is currently a TypeScript monorepo requiring Node.js at runtime, which introduces startup latency (~300–500ms), significant memory overhead, and a complex multi-package installation story. As a CLI tool that runs on every developer interaction, these costs are felt constantly.

Rewriting in Rust eliminates the Node.js runtime dependency entirely, producing a single statically-linked binary with sub-10ms startup, low memory footprint, and straightforward distribution via GitHub Releases or `cargo install`. Rust's ownership model also enforces memory safety and concurrency correctness at compile time — desirable properties for an agent runtime that manages I/O-heavy tool execution and streaming LLM responses.

The scope is intentionally narrow for v1: two LLM providers, built-in tools only, no extension system. This lets the team validate the architecture before adding complexity.

## What Changes

The entire codebase is replaced. The seven existing TypeScript packages (`pi-ai`, `pi-agent-core`, `pi-tui`, `pi-coding-agent`, `pi-mom`, `pi-pods`, `pi-web-ui`) are retired. The Rust rewrite ships as a single binary with equivalent core capabilities.

**Provider support narrows to two:**
- GitHub Copilot — accessed via the OpenAI Completions API format, authenticated through the OAuth device code flow.
- MiniMax — accessed via the Anthropic Messages API format with SSE streaming.

**What is not included in v1:**
- Extension/plugin system (was in `pi-pods`)
- Message broker / orchestration layer (`pi-mom`)
- Web UI (`pi-web-ui`)
- Any provider beyond the two listed above

**What is included:**
- Stateful agent runtime with tool execution loop and event streaming
- Built-in tools: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`
- Terminal UI built with `ratatui` (replaces `pi-tui`)
- CLI entry point with `clap` for argument parsing and session management

## Capabilities

### New Capabilities

- `rust-ai-providers` — Unified async LLM client abstracting GitHub Copilot (OpenAI Completions format) and MiniMax (Anthropic Messages format). Handles SSE streaming, retry logic, and provider-specific auth headers.

- `rust-agent-core` — Stateful agent runtime: maintains conversation context, dispatches tool calls, streams events (token delta, tool start/end, error) to consumers. Replaces `pi-agent-core`.

- `rust-tui` — Terminal UI with `ratatui`: differential rendering, input handling, theming, conversation and tool-output views. Replaces `pi-tui`.

- `rust-cli` — Binary entry point: `clap`-based argument parsing, session creation/resume, config loading from `~/.pi/config.toml`. Replaces `pi-coding-agent`.

- `rust-tools` — Built-in tool implementations: `read_file`, `write_file`, `edit_file`, `bash`, `grep`, `find`, `ls`. Each tool is sandboxed and emits structured output consumed by `rust-agent-core`.

- `github-copilot-auth` — OAuth device code flow for GitHub Copilot: initiates device authorization, polls for token, persists credentials to `~/.pi/credentials.toml`.

### Modified Capabilities

<!-- none — this is a full replacement, not a modification of existing specs -->

## Impact

**Codebase:** All TypeScript source under `packages/` is retired. New Rust source lives under `crates/`. The monorepo tooling (npm workspaces, Jest, Vitest, tsconfig) is removed or replaced with a Cargo workspace.

**Toolchain:** Rust stable (MSRV to be pinned in `rust-toolchain.toml`) becomes a required build dependency. Node.js is no longer required at runtime.

**Dependencies (key crates):** `tokio` (async runtime), `reqwest` (HTTP + SSE), `ratatui` + `crossterm` (TUI), `clap` (CLI), `serde` / `serde_json` (serialization), `anyhow` (error handling).

**Distribution:** npm packages are unpublished or deprecated. Binaries are published to GitHub Releases for `x86_64` and `aarch64` on Linux and macOS. `cargo install pi` will be the developer install path.

**APIs:** No public API surface is preserved. Existing integrations against the TypeScript packages must be rewritten.
