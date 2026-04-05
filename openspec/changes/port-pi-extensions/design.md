## Context

Pi Rust has a working agent loop, provider abstraction, tool system, and `spawn_agent` infrastructure. The REPL (`run_repl` in `main.rs`) handles slash commands via prefix matching. Sessions are stored as JSON files in `~/.pi/sessions/` and loaded by `session.rs`. Events flow through an `mpsc::UnboundedReceiver<AgentEvent>` drained in `drain_events_until_end`. The `AgentEvent` enum exposes `AgentStart`, `ToolExecutionStart`/`End`, and `AgentEnd { stop_reason }` — exactly the hooks the new features need.

All four extensions are added as native Rust modules inside `crates/pi-cli/`. No new crates are introduced, and no new external dependencies are required.

## Goals / Non-Goals

**Goals:**
- Port Ralph-Wiggum as interactive REPL slash commands (`/ralph start|status|stop|archive`) with persistent state in `.ralph/<name>/`
- Port the usage dashboard as a `pi usage [--period today|week|all]` subcommand that reads and aggregates existing session files
- Port tab status as a side-effect hook wired into the existing `drain_events_until_end` event loop
- Port agent guidance as a file-based system prompt loader checked at agent construction time, for both orchestrator and sub-agents

**Non-Goals:**
- A dynamic plugin/extension loading system
- A TUI — all output stays `println!`-based
- Persistent session recording for new sessions (sessions already exist; usage just reads them)
- Any new crates or external crate dependencies
- Guidance per model granularity (per-provider is sufficient for the first iteration)

## Decisions

### 1. Ralph-Wiggum: REPL slash commands + `.ralph/` state directory

Ralph is implemented as REPL slash commands, not a CLI subcommand, because it needs the live agent context. Three new arms are added to the slash-command `if`/`else` chain in `run_repl`:

- `/ralph start <name>` — creates `.ralph/<name>/state.json` and enters the iteration loop
- `/ralph status` — reads and pretty-prints `.ralph/<name>/state.json` for any active loop
- `/ralph stop` — sets `status: "paused"` in state.json and exits the loop cleanly
- `/ralph archive <name>` — moves `.ralph/<name>/` to `.ralph/archive/<name>/`

State file schema (`.ralph/<name>/state.json`):
```json
{ "name": "...", "status": "active|paused|completed", "iteration": 0, "max_iterations": 50, "reflection_every": 5, "task_file": ".ralph/<name>/task.md" }
```

The iteration loop calls `spawn_agent` (via the existing `SpawnAgentTool` path, but invoked programmatically from `ralph.rs`) with the task content. Every `reflection_every` iterations it appends a reflection prompt. The loop checks the sub-agent's text output for a `<complete>DONE</complete>` marker to detect completion. Ctrl+P is not interceptable in raw stdin mode — `/ralph stop` is the pause mechanism.

Rationale: keeping Ralph as REPL-only avoids duplicating agent setup code and keeps it consistent with `/model` switching. State in `.ralph/` (project-local) rather than `~/.pi/` makes it scoped to the current task directory.

### 2. Usage dashboard: `pi usage` subcommand, stream-parse sessions

A new `Commands::Usage { period: Option<String> }` variant is added to `cli.rs`. The handler in `main.rs` calls `usage::run_usage(period)`.

`usage.rs` iterates session files from `session::sessions_dir()` using `std::fs::read_dir`. Each file is parsed with `serde_json::from_reader` (streaming, not fully loaded into a `String` first) to avoid high memory usage on large session archives. Usage data is extracted from message metadata fields. Deduplication is done via a `HashSet<(timestamp, input_tokens, output_tokens)>` hash key before aggregation.

Output is a formatted table grouped by provider → model, with columns: model, input tokens, output tokens, cache tokens, estimated cost. Period filtering uses `session.updated_at` string prefix comparison (`"2026-04-03"` for today, ISO week prefix for week).

Rationale: read-only, no agent runtime needed — it runs before `run_interactive` is reached. Stream parsing avoids OOM on repos with hundreds of large sessions.

### 3. Tab status: hook into `drain_events_until_end`

`tab_status.rs` exports a `TabStatus` struct with a method `update(title: &str)` that writes `\x1b]0;{title}\x07` to stdout only when `std::io::stdout().is_terminal()` (using `std::io::IsTerminal`, stable since Rust 1.70). A boolean `saw_commit` flag is tracked by inspecting `ToolExecutionEnd { tool_name, result }` events where `tool_name == "bash"` and the result content contains `"git commit"`.

State machine:
- `AgentStart` → emit `pi - [dirname]:running...`
- `ToolExecutionEnd` with bash containing `git commit` → set `saw_commit = true`
- `AgentEnd { stop_reason: Error }` → emit `pi - [dirname]:🛑`
- `AgentEnd { stop_reason: _ }` → emit `:✅` if `saw_commit`, else `:🚧`
- 180s inactivity: a `tokio::time::interval` ticker runs alongside event drain; if no event arrives within 180s, emit `:🛑`

`drain_events_until_end` is updated to accept a mutable `TabStatus` reference and call it on relevant events. Dirname is resolved once at startup via `std::env::current_dir()`.

Rationale: zero overhead when not in a terminal (IS_TERMINAL check). No separate thread needed — `tokio::select!` with a sleep handles the inactivity timeout.

### 4. Agent guidance: file-based system prompt injection

`guidance.rs` exports `load_guidance(provider: &str) -> Option<String>` which checks two locations in order:
1. `.pi/guidance/{PROVIDER_UPPERCASE}.md` in the current working directory
2. `~/.pi/guidance/{PROVIDER_UPPERCASE}.md`

Project-local overrides the global. If neither exists, returns `None`. The function does a case-insensitive filename match (e.g., provider `"github-copilot"` maps to `GITHUB-COPILOT.md`).

In `build_agent` and `setup_sub_agent_config`, after resolving `provider_name`, guidance is loaded and prepended to `system_prompt` with a `\n\n---\n\n` separator before the existing prompt content. Sub-agents use their own provider's guidance file (currently always `"minimax"`), not the orchestrator's.

A `eprintln!` warning is emitted if a guidance directory exists but no file matches the active provider — this catches typos without being noisy by default.

Rationale: file-based (not config-based) because guidance docs can be long and prose-heavy. Per-provider granularity covers the current two-provider setup (copilot orchestrator + minimax sub-agent) without over-engineering.

### 5. All modules live in `crates/pi-cli/`

New files: `ralph.rs`, `usage.rs`, `tab_status.rs`, `guidance.rs`. All declared in `main.rs` via `mod` statements alongside the existing `mod session; mod spawn_agent;`. No new crates.

`session.rs` exposes `sessions_dir()` (already public) — no other changes needed for usage dashboard access.

## Risks / Trade-offs

- **Ralph loop cost spiral** — each iteration calls a full sub-agent round-trip. Mitigated by the `max_iterations` cap (default 50) and the `<complete>DONE</complete>` early-exit. Reflection prompts every N iterations add a small overhead but help surface stuck loops sooner.
- **Usage parsing memory on large archives** — stream-parsing with `serde_json::from_reader` keeps per-file allocation bounded. The deduplication `HashSet` grows with unique events but is bounded by the total number of non-duplicate entries, not file size.
- **Tab title escapes on non-iTerm2 terminals** — the `\x1b]0;...\x07` OSC sequence is widely supported (iTerm2, Kitty, Windows Terminal, most xterm derivatives) but silently ignored by terminals that don't support it. The `is_terminal()` guard prevents escape code garbage in piped output. No further fallback is needed.
- **Guidance file naming collisions** — provider names like `"github-copilot"` produce filenames that are unambiguous. The convention (uppercase, `.md`) is documented by the empty `~/.pi/guidance/` directory created on first use. The warn-if-directory-exists-but-no-match behavior catches the most common error (wrong case or extension) without requiring a strict registry.
- **Ralph state consistency on crash** — state is written to disk after each iteration completes, so a mid-iteration crash leaves `iteration` at the last completed value. The partially-written iteration's work may need to be re-done on resume, which is acceptable for a best-effort loop manager.
