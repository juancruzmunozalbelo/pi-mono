## 1. Context Compaction

- [ ] 1.1 Create `crates/pi-agent/src/compaction.rs` with `estimate_tokens(messages)` function using chars/4 heuristic
- [ ] 1.2 Implement `should_compact(messages, model)` — returns true when estimated tokens > 75% of (context_window - max_tokens)
- [ ] 1.3 Implement `build_summarization_prompt(messages_to_compact)` — creates a prompt asking the LLM to summarize conversation preserving key decisions, file paths, and task state
- [ ] 1.4 Implement `compact_messages(messages, provider, model, preserve_count)` — calls provider to summarize old messages, returns compacted message list
- [ ] 1.5 Implement recursive guard: if compacted result still exceeds budget, truncate summary text to fit
- [ ] 1.6 Implement `make_compaction_hook(provider, model, config) -> TransformContextHook` factory function
- [ ] 1.7 Add `CompactionConfig { preserve_count: usize, threshold: f64 }` with defaults (10, 0.75)
- [ ] 1.8 Wire compaction hook into `build_agent()` in `crates/pi-cli/src/main.rs`
- [ ] 1.9 Add tracing::info log when compaction occurs (message count before/after, token estimate savings)
- [ ] 1.10 Write unit tests: token estimation, should_compact threshold, summary message format
- [ ] 1.11 Write integration test with mock provider: 50 messages → compaction triggers → result has ~12 messages (summary + preserved)

## 2. Markdown TUI Rendering

- [ ] 2.1 Add `pulldown-cmark` and `syntect` to workspace Cargo.toml and pi-tui Cargo.toml
- [ ] 2.2 Create `crates/pi-tui/src/widgets/markdown.rs` with `render_markdown(text, width, theme) -> Vec<Line>` function
- [ ] 2.3 Implement heading rendering: H1 bold+underline, H2 bold, H3+ bold prefix
- [ ] 2.4 Implement inline styles: bold, italic, inline code (different foreground), strikethrough
- [ ] 2.5 Implement fenced code blocks: bordered block with language label, syntax highlighting via syntect
- [ ] 2.6 Implement list rendering: bullets for unordered, numbers for ordered, nested indentation
- [ ] 2.7 Implement blockquote rendering: vertical bar prefix + italic style
- [ ] 2.8 Implement link rendering: underlined text
- [ ] 2.9 Implement word wrapping: break at terminal width without splitting words, preserve styles across wraps
- [ ] 2.10 Implement fallback: unrecognized elements render as plain text, malformed markdown never panics
- [ ] 2.11 Replace current `render_markdown` in `crates/pi-tui/src/widgets/messages.rs` with the new pulldown-cmark based renderer
- [ ] 2.12 Write unit tests: each markdown element type renders to expected Spans, code block highlighting works, malformed input doesn't panic

## 3. Verification

- [ ] 3.1 `cargo check --workspace` — 0 errors
- [ ] 3.2 `cargo test --workspace` — all tests pass
- [ ] 3.3 `cargo clippy --workspace -- -D warnings` — 0 warnings
- [ ] 3.4 Manual test: long conversation (>20 messages) triggers compaction, session continues working
- [ ] 3.5 Manual test: assistant response with markdown renders correctly in TUI
