# Rust Rewrite: Gaps, Risks, and Improvements

**Scope:** GitHub Copilot (OpenAI Completions API) + MiniMax (Anthropic Messages API)
**Reference codebase:** `/Users/juancruz/pi-mono` — analyzed April 2026

---

## 1. Gaps

### 1.1 Async Streaming: `AsyncIterable` → Rust streams

The TS codebase has a hand-rolled `EventStream<T, R>` class
(`packages/ai/src/utils/event-stream.ts`) that drives all provider streaming.
It implements `Symbol.asyncIterator` with a waiter queue:

```ts
// event-stream.ts:8-66
private queue: T[] = [];
private waiting: ((value: IteratorResult<T>) => void)[] = [];
// push() either delivers to waiting consumer or enqueues; [Symbol.asyncIterator] does the inverse
```

The agent loop then does `for await (const event of response)` at the call
site (`agent-loop.ts:276`).

**Rust equivalent needed:**
- `tokio::sync::mpsc` channel + `tokio_stream::wrappers::ReceiverStream` is
  the closest idiomatic match.
- The `result()` method (`event-stream.ts:63`) — a separate `Promise` that
  resolves to the final `AssistantMessage` — needs a parallel channel or
  `tokio::sync::oneshot`.
- The "push to waiting waiter or queue" pattern maps naturally to an mpsc
  sender; no explicit waiter tracking needed.

**What is implicit:** The TS version silently swallows events pushed after
`done = true` (`event-stream.ts:22`). Rust's closed-channel error must be
handled explicitly (e.g., `let _ = tx.send(event)` after shutdown).

---

### 1.2 Partial-JSON Parsing During Tool-Call Streaming

`parseStreamingJson` (`packages/ai/src/utils/json-parse.ts`) uses the
`partial-json` npm package to parse incomplete JSON arriving over SSE chunks:

```ts
// json-parse.ts:1
import { parse as partialParse } from "partial-json";
// Tries JSON.parse first, falls back to partial-json, falls back to {}
```

Tool-call arguments (`block.partialArgs` in `openai-completions.ts:249` and
`block.partialJson` in `anthropic.ts:349`) are accumulated as strings and
re-parsed on every delta. The final parse on `content_block_stop` / the
`toolcall_end` event is expected to be valid JSON.

**Rust gap:** No standard crate handles partial JSON the same way. Options:
- `serde_json::from_str` for complete JSON.
- For partial: `sonic-rs` has streaming support; `gjson` is read-only.
- The simplest approach: accumulate raw bytes, only parse at `_end` events,
  expose the raw string as intermediate state — matching what the TS code
  actually needs at the UI layer (the delta string, not the parsed object).

---

### 1.3 Unicode Surrogate Sanitization

`sanitizeSurrogates` (`packages/ai/src/utils/sanitize-unicode.ts`) strips
unpaired surrogates before every API call. This is called for every user
message, assistant text block, system prompt, and tool result.

```ts
// sanitize-unicode.ts:22-24
return text.replace(
  /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g, ""
);
```

**Rust:** Rust `String` is always valid UTF-8; lone surrogates cannot exist.
However, when bytes arrive from external sources (tool stdout, user input
from stdin) as raw `Vec<u8>`, `String::from_utf8_lossy` replaces invalid
sequences with U+FFFD rather than removing them. Providers may reject U+FFFD
in some contexts. The Rust port must decide: strip replacement characters or
pass them through.

---

### 1.4 Tool Call ID Normalization

Two separate normalization paths exist:

- **OpenAI Completions** (`openai-completions.ts:497-509`):
  - Pipe-separated IDs from GitHub Copilot/Responses API: `{call_id}|{id}`
    — strip everything after `|`, sanitize to `[a-zA-Z0-9_-]`, truncate to 40 chars.
  - Standard OpenAI IDs: truncate to 40 chars.

- **Anthropic** (`anthropic.ts:698-700`):
  - Replace `[^a-zA-Z0-9_-]` with `_`, truncate to 64 chars.

**Gap:** These constraints are not documented anywhere outside the code.
The Rust implementation must bake in both limits (40-char and 64-char) per
API type with the correct character sets.

**Additional:** `transformMessages` (`transform-messages.ts:14`) builds a
`toolCallIdMap` in a two-pass traversal to keep `toolResult.toolCallId` in
sync with normalized IDs. This bi-directional ID tracking must be replicated
in Rust.

---

### 1.5 GitHub Copilot: Dynamic Request Headers

Every request to GitHub Copilot requires per-request computed headers
(`providers/github-copilot-headers.ts`):

```ts
// github-copilot-headers.ts:24-36
"X-Initiator": inferCopilotInitiator(messages),  // "user" | "agent"
"Openai-Intent": "conversation-edits",
"Copilot-Vision-Request": "true",  // only when images present
```

`inferCopilotInitiator` inspects the last message role — not a static
header. The Rust HTTP client must recompute these before every request, not
cache them at client construction time.

---

### 1.6 GitHub Copilot: OAuth Token Refresh Under Concurrency

`auth-storage.ts:369-413` implements lock-based token refresh:

```ts
// auth-storage.ts:370-413
private async refreshOAuthTokenWithLock(providerId: OAuthProviderId): Promise<...> {
  return await this.storage.withLockAsync(async (current) => {
    // Re-read file inside lock to detect if another instance already refreshed
    if (Date.now() < cred.expires) { return { result: apiKey } }
    // ... refresh and write
  });
}
```

The lock is a file lock (`proper-lockfile`) with retry/backoff. On failure,
it re-reads the file to check if another process succeeded (`auth-storage.ts:458-464`).

**Rust gaps:**
- `fd-lock` or `fs2` crate for cross-process file locking (POSIX `flock`).
- The "re-read and check if another instance already refreshed" pattern
  (`auth-storage.ts:458-464`) must be reproduced.
- Token expiry is stored as `expires: number` (Unix ms timestamp in
  `OAuthCredentials`). The Rust type must match this wire format exactly for
  the `auth.json` file to be compatible across TS and Rust instances.

---

### 1.7 Anthropic Stealth Mode: Claude Code Identity Headers

When using an OAuth token (detected by `apiKey.includes("sk-ant-oat")`,
`anthropic.ts:519`), the client adds identity headers to pass as Claude Code:

```ts
// anthropic.ts:574-582
"anthropic-beta": `claude-code-20250219,oauth-2025-04-20,${betaFeatures.join(",")}`,
"user-agent": `claude-cli/${claudeCodeVersion}`,  // "2.1.75"
"x-app": "cli",
```

Tool names are also renamed using a CC-canonical lookup table (`anthropic.ts:66-101`):

```ts
const claudeCodeTools = ["Read", "Write", "Edit", "Bash", "Grep", "Glob", ...];
const ccToolLookup = new Map(claudeCodeTools.map((t) => [t.toLowerCase(), t]));
```

On response, tool names are mapped back (`fromClaudeCodeName`,
`anthropic.ts:94-101`). The Rust port must replicate both the header
injection and the bidirectional tool-name mapping, and must update
`claudeCodeVersion` when the stealth version bumps.

---

### 1.8 Adaptive Thinking / Budget Thinking Split

For MiniMax (Anthropic Messages API), two thinking modes coexist:

- **Adaptive** (Opus 4.6, Sonnet 4.6 — `supportsAdaptiveThinking`,
  `anthropic.ts:447-454`): `thinking: { type: "adaptive" }` +
  `output_config: { effort: "low"|"medium"|"high"|"max" }`.
- **Budget-based** (older models): `thinking: { type: "enabled", budget_tokens: N }`.
  Budget is dynamically computed by `adjustMaxTokensForThinking` in
  `simple-options.ts` (not shown but referenced at `anthropic.ts:503`).

**Gap:** The model-ID string matching is fragile:

```ts
// anthropic.ts:448-453
modelId.includes("opus-4-6") ||
modelId.includes("opus-4.6") ||
modelId.includes("sonnet-4-6") ||
modelId.includes("sonnet-4.6")
```

The Rust port should use a structured model capability flag (already present
as `Model.reasoning: bool`) rather than string matching to determine thinking
mode. The mapping from `ThinkingLevel` to Anthropic `effort` string
(`mapThinkingLevelToEffort`, `anthropic.ts:460-475`) must be preserved
exactly.

---

### 1.9 Message Transformation: Orphaned Tool Calls

`transformMessages` (`transform-messages.ts:98-169`) performs a two-pass
transformation including synthetic insertion of `toolResult` messages for
orphaned tool calls (calls with no matching result, e.g. from aborted turns):

```ts
// transform-messages.ts:108-123
if (pendingToolCalls.length > 0) {
  for (const tc of pendingToolCalls) {
    if (!existingToolResultIds.has(tc.id)) {
      result.push({ role: "toolResult", toolCallId: tc.id, ..., isError: true })
    }
  }
}
```

Errored/aborted assistant messages are also dropped entirely
(`transform-messages.ts:132-135`). These behaviors are invisible from the
API contract — they are pure defensive logic to prevent provider rejections.
The Rust port must replicate them or providers will return 400 errors.

---

### 1.10 Session File Format (JSONL v3)

`session-manager.ts:27-80` defines the on-disk format:

- Header: `{ type: "session", version: 3, id, timestamp, cwd, parentSession? }`
- Entries: `SessionMessageEntry`, `ModelChangeEntry`, `CompactionEntry`,
  `BranchSummaryEntry`, `ThinkingLevelChangeEntry` — all tagged unions
  discriminated by `type`.

The file is append-only JSONL (one JSON object per line). The Rust port must
read and write this exact format for sessions to be portable between old and
new binaries.

---

### 1.11 Bash Tool: Process Tree Management

`createLocalBashOperations` (`tools/bash.ts:69-128`) uses `detached: true`
when spawning the shell to enable process-group killing:

```ts
// bash.ts:79-84
const child = spawn(shell, [...args, command], {
  cwd, detached: true,
  env: env ?? getShellEnv(),
  stdio: ["ignore", "pipe", "pipe"],
});
// Kill: killProcessTree(child.pid)
```

Shell config is platform-detected (`getShellConfig()`). The rolling buffer
pattern (`DEFAULT_MAX_BYTES * 2` in-memory, spill to tmpfile past
`DEFAULT_MAX_BYTES`) is used for streaming output with truncation
(`bash.ts:290-326`).

**Rust gaps:**
- `tokio::process::Command` with `process_group(0)` on Unix for process-group
  management; Windows requires a Job Object.
- `nix::unistd::killpg` / `libc::kill(-pgid, SIGKILL)` for tree kill.
- The "rolling buffer + tmpfile spill" pattern has no stdlib equivalent;
  needs explicit ring-buffer logic.
- `signal` abort (`bash.ts:97-99`): the Rust equivalent is a
  `CancellationToken` from `tokio-util`, not `AbortSignal`.

---

### 1.12 TUI: Differential Rendering

The TUI (`tui.ts`) is a custom terminal renderer using direct ANSI writes.
It tracks Kitty keyboard protocol state globally
(`keys.ts:25-40`) and parses multi-byte escape sequences. The differential
rendering compares old/new rendered lines to minimize writes.

**Gap:** The entire TUI is ~15 files with no documented rendering contract.
Options for Rust:
- `ratatui` (most mature) — requires mapping the `Component` interface to
  ratatui `Widget`.
- `crossterm` raw mode only — matches the "direct ANSI write" approach most
  closely.
- The custom Kitty protocol negotiation (`CURSOR_MARKER = "\x1b_pi:c\x07"`,
  `tui.ts:67`) and Kitty image protocol (`terminal-image.ts`) are
  non-standard extensions that ratatui does not handle natively.

---

### 1.13 Compaction and Context Token Estimation

`agent-session.ts` references `calculateContextTokens`,
`estimateContextTokens`, `compact`, `shouldCompact` from
`compaction/index.ts`. Compaction triggers when the context window is
estimated to be near capacity. The Rust port needs equivalent token
estimation logic (likely via a character-based heuristic or a tokenizer
crate like `tiktoken-rs`).

---

## 2. Risks

### 2.1 SSE Streaming Parsing Edge Cases

Both providers return `data: [DONE]` or event-typed chunks. The `openai` npm
SDK and `@anthropic-ai/sdk` abstract the raw SSE parsing. In Rust, using
`reqwest` + manual SSE parsing introduces risks:

- **Empty data lines:** SSE spec allows empty `data:` fields; some providers
  (e.g., the Anthropic SDK) emit `data: ` (space) vs `data:` (no space).
- **Multi-line data:** SSE spec allows `data:` spread across multiple lines;
  most AI providers don't use this but parsers must handle it.
- **Comment lines:** Lines starting with `:` are SSE comments (heartbeats).
  The Copilot endpoint sends these. Ignoring them is correct but must be
  explicit.
- **Chunk boundary:** A single TCP packet may contain partial SSE events, or
  multiple complete events. The Rust parser must buffer incomplete events.
- **`choices: []` chunks:** `openai-completions.ts:139` guards `Array.isArray
  (chunk.choices) ? chunk.choices[0] : undefined`. The Copilot endpoint
  periodically sends usage-only chunks with `choices: []` before the final
  `DONE`. Forgetting this check causes a panic in Rust if indexing directly.

**Recommended approach:** Use `eventsource-client` or `reqwest-eventsource`
crate; avoid hand-rolling SSE line splitting.

---

### 2.2 GitHub Copilot Auth Token Refresh Race Conditions

The TS code uses `proper-lockfile` (POSIX `flock`-based) to serialize token
refreshes across multiple processes. The critical window is:

1. Process A reads token, sees it expired.
2. Process B reads token, sees it expired.
3. Both attempt refresh — one must win, the other must re-read and use the
   winner's token.

The TS code handles this at `auth-storage.ts:451-464` (reload and re-check
after failed refresh). In Rust:

- On macOS/Linux: `flock(2)` on the auth.json file via `fd-lock`.
- On Windows: `LockFileEx` — `fd-lock` crate supports this.
- **Risk:** If the Rust binary and the TS binary run simultaneously (mixed
  deployments), they must use the same locking mechanism. Both use file
  locks on the same path, so interop is possible but untested.
- The `stale: 30000` (30-second stale lock timeout in `auth-storage.ts:132`)
  prevents deadlock if a process dies holding the lock. Rust must replicate
  this staleness check.

---

### 2.3 TUI Rendering Performance Differences

The TS TUI does differential string comparison per line to minimize terminal
writes. In Rust with `ratatui`, the backend also does differential rendering,
but the rendering model differs:

- TS renders to `string[]` per component, then diffs arrays.
- ratatui renders to a `Buffer` of `Cell` objects (char + style), then diffs
  cells.

Potential issues:
- **ANSI escape passthrough:** The TS TUI passes pre-formatted ANSI strings
  (from `chalk`, syntax highlighting, `renderDiff`) as opaque lines. ratatui
  `Paragraph` strips ANSI; you need `tui-markup` or `ansitok` + `ansi_to_tui`
  to convert ANSI to ratatui `Spans`.
- **Kitty image protocol:** `terminal-image.ts` sends raw binary Kitty
  protocol frames. ratatui has no built-in image support; `ratatui-image`
  crate is the current option but is not stable.
- **Resizing:** The TS TUI handles `SIGWINCH` via `process.stdout.on
  ("resize", ...)`. In Rust, `crossterm::event::Event::Resize` is the
  equivalent — works but requires the event loop to process terminal events.

---

### 2.4 Tool Execution: Bash Process Management, Signals, Encoding

**Signal propagation:** `killProcessTree(child.pid)` in TS wraps a
platform-specific tree kill. On macOS/Linux this is `kill(-pgid, SIGKILL)`.
On Rust, `nix::sys::signal::kill(Pid::from_raw(-pgid), Signal::SIGKILL)` is
the equivalent, but only on Unix. Windows needs a different approach
(`TerminateJobObject`).

**Output encoding:** `child.stdout?.on("data", onData)` receives raw
`Buffer`. The TS code decodes as UTF-8 (`buffer.toString("utf-8")`), but
on Windows, cmd.exe outputs Windows-1252 or UTF-16LE. The Rust port must
handle this explicitly (likely: try UTF-8, fallback to `encoding_rs` for
Windows).

**Timeout precision:** The TS timeout uses `setTimeout` (millisecond
precision) but the input is in whole seconds (`timeout: Type.Number`, units
"seconds", `bash.ts:31`). Rust `tokio::time::timeout` is more precise but
the unit conversion from user-facing seconds must be explicit.

**Abort race:** In `bash.ts:96-99`, the abort handler calls `killProcessTree`
synchronously when `signal.aborted` at spawn time. In async Rust with
`tokio`, there is a gap between spawning the child and registering the
cancellation; `select!` with the cancel token is the idiomatic fix.

---

### 2.5 Session File Compatibility

The JSONL session format uses TypeScript discriminated unions. Migration
logic (`migrations.ts`, referenced in `main.ts:688`) handles v1→v3 upgrades.
The Rust deserializer must:

- Handle missing `version` field (v1 sessions, `session-manager.ts:31`).
- Handle `type` discriminants added in later versions.
- Avoid silently dropping unknown fields — use `#[serde(deny_unknown_fields)]`
  only for new entries; old entries need `#[serde(default)]` for optional fields.

A corrupt JSONL line (partial write from crash) must not prevent loading the
rest of the session — the TS code reads line-by-line and skips parse errors.

---

### 2.6 Platform-Specific Issues

| Area | macOS | Linux | Windows |
|---|---|---|---|
| Shell detection | `getShellConfig()` reads `$SHELL`, defaults to `zsh` | Same, defaults to `bash` | `cmd.exe` or `PowerShell` — different arg passing |
| Process kill | `kill(-pgid, SIGKILL)` | Same | `TerminateJobObject` |
| File locking | `flock(2)` | `flock(2)` | `LockFileEx` |
| Auth dir | `~/.pi/` (or `XDG_DATA_HOME`) | Same | `%APPDATA%\pi\` |
| Image detection | `file` magic bytes | Same | Same |
| Kitty protocol | iTerm2/Kitty/WezTerm | Same | Windows Terminal (partial) |

The TS code uses `process.platform` checks implicitly via shell detection.
The Rust port should use `cfg!(target_os = ...)` at compile time for shell
defaults and runtime detection for terminal capabilities.

---

### 2.7 Reasoning Details: Encrypted Thought Signatures

`openai-completions.ts:261-273` handles `reasoning_details` in chunks —
encrypted reasoning blobs that must be attached to the matching `toolCall`
by ID:

```ts
const reasoningDetails = (choice.delta as any).reasoning_details;
if (reasoningDetails && Array.isArray(reasoningDetails)) {
  for (const detail of reasoningDetails) {
    if (detail.type === "reasoning.encrypted" && detail.id && detail.data) {
      const matchingToolCall = output.content.find(
        (b) => b.type === "toolCall" && b.id === detail.id,
      );
      if (matchingToolCall) { matchingToolCall.thoughtSignature = JSON.stringify(detail); }
    }
  }
}
```

This is a non-standard extension for GitHub Copilot's internal reasoning
passthrough. It is not in the official OpenAI SSE schema. The Rust deserializer
must use a permissive/dynamic field approach (`serde_json::Value`) here.

---

## 3. Improvements

### 3.1 Memory Safety: No `(as any)` Escape Hatches

The TS code has ~40 `as any` casts, mostly to paper over API
type incompatibilities:
- `openai-completions.ts:146`: `(choice as any).usage` — Moonshot
  puts usage in a non-standard field.
- `openai-completions.ts:184`: `(choice.delta as any)[field]` — reasoning
  field name probing at runtime.
- `anthropic.ts:797,800,860`: `(lastBlock as any).cache_control` — adding
  fields not in the SDK type.

In Rust, `serde_json::Value` and `#[serde(flatten)]` with
`HashMap<String, Value>` force explicit handling of dynamic fields. There
are no silent `as any` escapes. Every unknown field must be consciously
handled or ignored.

---

### 3.2 Error Handling: Typed Errors Instead of String Messages

The TS code propagates errors almost entirely as `Error.message` strings:

```ts
// agent-loop.ts:194
if (message.stopReason === "error" || message.stopReason === "aborted") { ... }
// openai-completions.ts:294
output.errorMessage = error instanceof Error ? error.message : JSON.stringify(error);
```

The `stopReason` + `errorMessage: string` pattern cannot be pattern-matched
programmatically — callers must parse the string.

In Rust, a typed error enum (`ProviderError`, `ToolError`, `SessionError`)
allows exhaustive `match` on failure paths and compiler-enforced handling.
Retry logic (e.g., the `maxRetryDelayMs` pattern in `types.ts:97-100`) can
be structured errors rather than magic strings.

---

### 3.3 Concurrency: True Parallel Tool Execution

`executeToolCallsParallel` (`agent-loop.ts:390-437`) in TS starts all tool
calls with `runnableCalls.map((prepared) => ({ prepared, execution:
executePreparedToolCall(...) }))` — this is JS concurrency via the event
loop, not true parallelism (V8 is single-threaded). Long-running tools like
`bash` block the event loop during CPU-bound portions.

In Rust with `tokio`, `futures::future::join_all` or `tokio::task::spawn`
gives true OS-thread parallelism for I/O-bound tools and genuine CPU
parallelism via `rayon` for CPU-bound work.

---

### 3.4 Distribution: Single Static Binary

The TS stack requires:
- Node.js runtime (typically 200+ MB)
- `npm install` with native addons (`canvas` for image resize, `sharp`,
  `proper-lockfile`)
- Platform-specific `.node` binaries

Rust compiles to a single static binary (~15-30 MB for a CLI of this
complexity). No runtime installation, no npm, no native addon compatibility
issues. `cargo install` or a pre-built binary is sufficient.

---

### 3.5 Startup Performance

The TS `main.ts` has explicit `time()` instrumentation showing the startup
path: `parseArgs.firstPass` → `runMigrations` → `resourceLoader.reload`
→ `createAgentSession` — each step adds hundreds of milliseconds due to
CommonJS/ESM module loading, `require()` chains, and TypeBox schema
compilation.

Rust binaries have negligible startup time (< 50 ms for a CLI of this size).
This matters for non-interactive use cases (`runPrintMode`,
`runRpcMode` in `main.ts:948-960`) where the agent is invoked
programmatically in a loop.

---

### 3.6 Avoiding TS Technical Debt

**Debt item 1 — Provider compat auto-detection via URL substring matching**
(`openai-completions.ts:793-843`): `detectCompat` checks 15+ URL substrings
to infer capabilities. The Rust port scopes down to exactly 2 providers
(GitHub Copilot, MiniMax), eliminating this entire heuristic system.

**Debt item 2 — `hasToolHistory` workaround** (`openai-completions.ts:41-53`):
Exists because Anthropic via LiteLLM/proxy requires `tools` param when
messages include tool history, even when the current turn has no tools.
With only 2 providers, provider-specific quirks can be hardcoded per-provider
rather than in shared logic.

**Debt item 3 — `requiresAssistantAfterToolResult` bridge messages**
(`openai-completions.ts:526-531`): Synthetic `"I have processed the tool
results."` messages injected for providers that don't allow user messages
directly after tool results. GitHub Copilot does not require this; MiniMax
(Anthropic) does not either. Can be removed entirely for the 2-provider scope.

**Debt item 4 — `normalizeToolCallId` pipe-separator handling**
(`openai-completions.ts:497-509`): The pipe-separated ID format
`{call_id}|{id}` comes from the OpenAI Responses API (a different API path
not used by GitHub Copilot's completions endpoint). This normalization may
be unnecessary for the completions-only path but is currently shared code.

**Debt item 5 — Reasoning format branching** (`openai-completions.ts:411-429`):
`thinkingFormat` can be `"zai"`, `"qwen"`, `"qwen-chat-template"`,
`"openrouter"`, or `"openai"`. The 2-provider scope uses only the
`openai` format for GitHub Copilot. The entire branch can be removed.

---

### 3.7 Session Storage: SQLite Instead of Append-Only JSONL

The current JSONL format (`session-manager.ts`) is append-only, requiring a
full file scan to load any session. As sessions grow (thousands of turns,
compaction entries), load time grows linearly.

The Rust port could use SQLite (via `rusqlite` or `sqlx`) with an indexed
schema, enabling:
- O(1) session lookup by ID.
- Range queries for compaction entries.
- Atomic writes without file locking.
- Cross-process safety via SQLite WAL mode.

**Caveat:** This breaks compatibility with the TS session format. A migration
path or dual-format support would be required for existing sessions.

---

### 3.8 Type-Safe Tool Parameter Schemas

The TS code uses TypeBox (`@sinclair/typebox`) for JSON Schema generation.
The parameters are `TSchema` objects passed opaquely to providers:

```ts
// types.ts:217-221
export interface Tool<TParameters extends TSchema = TSchema> {
  name: string;
  description: string;
  parameters: TParameters;
}
```

The schema is serialized to JSON for providers but TypeBox validation
(`validateToolArguments` called in `agent-loop.ts:491`) is runtime-only.

In Rust, tool parameter schemas can be:
- `schemars::JsonSchema` derive macros for compile-time schema generation.
- `serde` for deserialization with compile-time type checking.

This gives a compile-time guarantee that tool implementations match their
declared schemas — currently impossible in TS without extra type gymnastics.

---

### 3.9 Cancellation: Structured via `CancellationToken`

TS uses `AbortController`/`AbortSignal` threaded through every function
signature (`agent-loop.ts:155`, `bash.ts:53`, `read.ts:137`, etc.). The
signal must be passed explicitly at every call site; forgetting it silently
loses cancellation.

`tokio_util::sync::CancellationToken` with `token.clone()` at spawn sites
and `select! { _ = token.cancelled() => ... }` at await points is idiomatic
Rust and harder to accidentally omit (the token is part of the task's
context, not a hidden optional parameter).

---

### 3.10 Performance: Zero-Copy SSE Parsing

The Anthropic and OpenAI SDKs in TS parse SSE by decoding the full response
body as UTF-8 strings, splitting on `\n`, and then JSON-parsing each event.
Every chunk allocation is a new heap string.

In Rust, `bytes::Bytes` allows zero-copy slicing of the response body.
The SSE frame can be parsed in-place with `memchr` for newline detection,
and JSON deserialization can use `simd-json` or `sonic-rs` for SIMD-
accelerated parsing on the hot path. For a high-throughput agent running
many parallel tool calls, this can meaningfully reduce GC pressure
(which doesn't exist in Rust) and latency.
