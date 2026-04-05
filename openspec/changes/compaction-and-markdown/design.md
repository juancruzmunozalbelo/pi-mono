# Design: compaction-and-markdown

## Overview

Two independent features shipped together: automatic context compaction to prevent context-window overflow, and full markdown rendering in the TUI to replace the current hand-rolled inline parser.

---

## Feature 1: Context Compaction

### Problem

`agent_loop.rs` accumulates every `Message` in `state.messages` without bound. When the accumulated token estimate exceeds the model's `context_window`, the provider returns an error and the session dies. The `transform_context` hook already exists and is called on every turn before the `ChatRequest` is built — it is the right place to inject compaction.

### Token Estimation

**Heuristic:** `estimated_tokens = total_chars / 4`

Applied per-message by summing the character length of all `ContentBlock` text fields. This avoids a tokenizer dependency and is ~80% accurate for English. The model's `context_window` field (on `pi_ai::types::Model`, available in `state.model`) provides the budget.

**Safety threshold:** Compact when `estimated_tokens > context_window * 0.75`. Using 75% (not 80%) as a safety margin to account for heuristic inaccuracy and for the tokens consumed by system prompt and tool definitions, which are not included in the message list passed to the hook.

### Compaction Algorithm

1. Estimate tokens for the full `Vec<Message>` passed to the hook.
2. If below threshold, return the messages unchanged.
3. Otherwise, identify the **tail**: the last `N` messages (default `N = 10`, configurable). These are always kept verbatim to preserve immediate conversation context.
4. The **head** (everything before the tail) is summarized with a single LLM call to the same provider, using this prompt:

   > "Summarize this conversation concisely, preserving key decisions, file paths mentioned, code changes made, and the current task state. Be brief."

5. The summary response is wrapped in a `Message::User` with a single `ContentBlock::Text` prefixed with `"[Summary of previous conversation]\n"`.
6. The compacted message list is: `[summary_message] + tail`.
7. **Recursive safety:** After compaction, re-estimate tokens. If still above threshold (summary was too long), truncate the summary text at `context_window * 0.5 * 4` characters and re-wrap.

### Compaction is turn-boundary-only

Compaction must never fire mid-stream. The hook runs synchronously at the start of a turn (before `provider.chat(request)` is called), never during `stream_to_message`. This invariant is already guaranteed by the current call site in `agent_loop.rs` — no change required there.

### Where It Lives

**New file:** `crates/pi-agent/src/compaction.rs`

Public API:

```rust
/// Configuration for the compaction hook.
pub struct CompactionConfig {
    /// Number of recent messages to keep verbatim (default: 10).
    pub keep_recent: usize,
    /// Fraction of context_window at which compaction triggers (default: 0.75).
    pub threshold: f64,
}

impl Default for CompactionConfig { ... }

/// Returns a TransformContextHook that compacts the context when needed.
/// Requires the provider and model to call the summarizer.
pub fn make_compaction_hook(
    provider: Arc<dyn pi_ai::Provider>,
    model: pi_ai::types::Model,
    config: CompactionConfig,
) -> TransformContextHook
```

The function returns a `TransformContextHook` (a `Box<dyn Fn(Vec<Message>) -> BoxFuture<Vec<Message>> + Send + Sync>`), matching the type alias in `hooks.rs`.

### Registration

**Modified file:** `crates/pi-cli/src/main.rs`

After building the provider and selecting the model, call `make_compaction_hook` and set it on `AgentConfig::hooks.transform_context`. The `keep_recent` and `threshold` values may be wired to CLI flags in a follow-up; for now they use defaults.

### Error Handling

If the summarization LLM call fails (network error, provider error), the hook returns the original unmodified message list. Compaction failure is non-fatal — better to risk a context-length error from the provider than to silently corrupt the conversation.

---

## Feature 2: Markdown Rendering in TUI

### Problem

`messages.rs` already has a `render_markdown` function and a `parse_inline` helper, but they are hand-rolled. They handle `**bold**`, `` `inline code` ``, and fenced code blocks with ASCII borders — nothing else. No `pulldown-cmark`, no syntax highlighting, no headings, no lists, no blockquotes, no italics, no links.

### Parser

**Library:** `pulldown-cmark` (the standard Rust markdown parser; used by rustdoc and mdBook). It produces a stream of `Event` values — `Start(tag)`, `Text(...)`, `Code(...)`, `End(tag)`, etc. — which are easy to map to ratatui `Span`s.

### Renderer

**New file:** `crates/pi-tui/src/widgets/markdown.rs`

Public API:

```rust
/// Render a markdown string into ratatui Lines, using the given theme for
/// base styles and terminal width for word wrapping.
pub fn render_markdown(text: &str, theme: &Theme, width: u16) -> Vec<Line<'static>>
```

This replaces (and takes the same name as) the existing `render_markdown` function in `messages.rs`. The call site in `MessagesWidget::build_lines` passes `area.width` as the `width` argument. The `area` is available inside `render` but not `build_lines` today — `build_lines` will be updated to accept `width: u16`.

#### Element mapping

| Markdown | Rendering |
|---|---|
| `# H1` | Bold + Underline, full width |
| `## H2`, `### H3` | Bold, prefixed with `##` / `###` |
| `**bold**` | `Modifier::BOLD` |
| `*italic*` / `_italic_` | `Modifier::ITALIC` |
| `` `inline code` `` | `theme.code_style` (different background) |
| Fenced code block | Bordered block (`┌─ lang ─┐` / `│ line` / `└───┘`) + syntax highlighting |
| `- item` / `* item` | `  • ` prefix, indented |
| `1. item` | `  1. ` prefix, indented |
| `> blockquote` | `  ▌ ` prefix, italic |
| `[text](url)` | Underline for text + dim `(url)` appended |
| Paragraph break | Empty `Line::default()` |
| Horizontal rule | `─` repeated to terminal width |

#### Syntax highlighting

**Library:** `syntect` with `SyntaxSet::load_defaults_newlines()` and `ThemeSet::load_defaults()`. Theme: `"base16-ocean.dark"` (or `"base16-eighties.dark"` as fallback). Language detected from the fenced code block info string (`rust`, `python`, `javascript`, `bash`, `json`, `toml`, `yaml`, `go`, `typescript`). Unknown languages fall back to plain `theme.code_style`.

`syntect` highlights a line to a `Vec<(Style, &str)>`. Each `(style, text)` pair maps to a ratatui `Span` with `Color::Rgb(r, g, b)` from the syntect `Color`.

**Binary size concern:** `syntect`'s default syntax and theme sets add ~8–10 MB to the binary. This is acceptable for a TUI agent. If it becomes an issue, gate it behind a `syntax-highlighting` Cargo feature flag (enabled by default) — the code block renderer falls back to plain `theme.code_style` when the feature is off.

#### Word wrapping

Ratatui's `Paragraph::wrap(Wrap { trim: false })` handles reflowing `Line`s at render time. For code blocks, wrapping is disabled — lines that exceed terminal width are truncated with a `…` suffix to avoid visual corruption of indented code.

### Dependencies

Add to workspace `Cargo.toml` and `crates/pi-tui/Cargo.toml`:

```toml
pulldown-cmark = "0.11"
syntect = { version = "5", default-features = false, features = ["default-syntaxes", "default-themes", "html-renderer", "regex-onig"] }
```

`default-features = false` with explicit features avoids pulling in unused syntect backends.

### Modified files

- `crates/pi-tui/src/widgets/messages.rs` — remove `render_markdown` and `parse_inline`, import `crate::widgets::markdown::render_markdown`, update `build_lines` signature to accept `width: u16`, pass `area.width` from `render`.
- `crates/pi-tui/src/widgets/mod.rs` — add `pub mod markdown`.

---

## Files Changed

| File | Status | Notes |
|---|---|---|
| `crates/pi-agent/src/compaction.rs` | New | Token estimation + compaction hook factory |
| `crates/pi-tui/src/widgets/markdown.rs` | New | pulldown-cmark → ratatui renderer |
| `crates/pi-cli/src/main.rs` | Modified | Register compaction hook in AgentConfig |
| `crates/pi-tui/src/widgets/messages.rs` | Modified | Delegate to markdown.rs, pass width |
| `crates/pi-tui/src/widgets/mod.rs` | Modified | Expose markdown module |
| `Cargo.toml` (workspace) | Modified | Add pulldown-cmark, syntect |
| `crates/pi-tui/Cargo.toml` | Modified | Add pulldown-cmark, syntect |

---

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| `syntect` adds ~10 MB to binary | Use `default-features = false`; gate behind optional `syntax-highlighting` feature flag |
| Compaction summary quality varies by model | MiniMax may summarize poorly; log a warning when compaction fires so users can observe; test with GPT-4o and MiniMax before shipping |
| Token estimation inaccuracy | Use 75% threshold instead of 80% as safety margin |
| Compaction mid-stream causes confusion | Hook only runs at turn start, before `provider.chat()`; the streaming invariant is already guaranteed by the call site |
| Summarizer call adds latency | Surface a `[Compacting context…]` status event so the TUI can show feedback; the hook can emit this via a channel if needed (defer to follow-up) |
