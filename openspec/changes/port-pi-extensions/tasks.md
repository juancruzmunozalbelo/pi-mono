## 1. Ralph-Wiggum Loop Manager

- [ ] 1.1 Create `crates/pi-cli/src/ralph.rs` with `RalphState` struct (name, status, iteration, max_iterations, reflection_interval, task_file, created_at, updated_at) and serde derives
- [ ] 1.2 Implement `.ralph/<name>/state.json` persistence: `save_state()`, `load_state()`, `ralph_dir()`
- [ ] 1.3 Implement `/ralph start <name>` command: create loop directory, write initial state, read task from `.ralph/<name>/task.md`
- [ ] 1.4 Implement iteration loop: call `spawn_agent` with task content + iteration context, parse result, increment iteration
- [ ] 1.5 Implement completion detection: scan sub-agent output for `<complete>DONE</complete>` marker, set status to "completed"
- [ ] 1.6 Implement reflection checkpoints: every N iterations (default 5), prepend reflection prompt to the next spawn_agent call
- [ ] 1.7 Implement max iterations guard: stop loop and set status "max_iterations_reached" after 50 iterations (configurable)
- [ ] 1.8 Implement `/ralph stop`: pause active loop, save state with status "paused"
- [ ] 1.9 Implement `/ralph start <name>` resume: detect existing paused state, resume from saved iteration
- [ ] 1.10 Implement `/ralph status`: list all loops in `.ralph/` with name, status, iteration count, last updated
- [ ] 1.11 Implement `/ralph archive <name>`: move `.ralph/<name>/` to `.ralph/archive/<name>/`
- [ ] 1.12 Wire ralph commands into the REPL input handler in `main.rs`

## 2. Usage Dashboard

- [ ] 2.1 Create `crates/pi-cli/src/usage.rs` with `UsageStats`, `ModelStats`, `ProviderStats` structs
- [ ] 2.2 Implement session file reader: stream-parse `~/.pi/sessions/*.json` with `serde_json::from_reader`
- [ ] 2.3 Implement usage extraction: parse assistant messages for usage (input/output/cache tokens) and cost fields
- [ ] 2.4 Implement deduplication by hash of (timestamp, input_tokens, output_tokens)
- [ ] 2.5 Implement period filtering: today, this week, all time (using chrono date comparison)
- [ ] 2.6 Implement table output: Provider/Model, Sessions, Messages, Cost, Input Tokens, Output Tokens columns
- [ ] 2.7 Add `pi usage [--period today|week|all]` subcommand to `cli.rs` and wire in `main.rs`

## 3. Tab Status

- [ ] 3.1 Create `crates/pi-cli/src/tab_status.rs` with `TabStatus` struct (state, running, saw_commit, last_activity)
- [ ] 3.2 Implement `set_title()`: write `\x1b]0;{title}\x07` to stdout, guard with `std::io::IsTerminal`
- [ ] 3.3 Implement event handlers: on AgentStart set running, on tool_call detect git commit regex, on AgentEnd set final status
- [ ] 3.4 Implement 180-second inactivity timeout via `tokio::select!` with timer alongside event recv
- [ ] 3.5 Integrate TabStatus into the event drain loop in `main.rs` (both print mode and REPL)

## 4. Agent Guidance

- [ ] 4.1 Create `crates/pi-cli/src/guidance.rs` with `load_guidance(provider_name, cwd)` function
- [ ] 4.2 Implement file lookup: check `.pi/guidance/{PROVIDER}.md` (project-local) then `~/.pi/guidance/{PROVIDER}.md` (global)
- [ ] 4.3 Implement system_prompt prepend: if guidance found, prepend to existing system_prompt with `\n---\n` separator
- [ ] 4.4 Wire guidance loading into `build_agent()` and `setup_sub_agent_config()` in `main.rs`
- [ ] 4.5 Add `tracing::debug` logs for guidance loaded / not found

## 5. Verification

- [ ] 5.1 `cargo check --workspace` — 0 errors
- [ ] 5.2 `cargo test --workspace` — all tests pass
- [ ] 5.3 `cargo clippy --workspace -- -D warnings` — 0 warnings
- [ ] 5.4 Manual test: `/ralph start test-loop` with a simple task file
- [ ] 5.5 Manual test: `pi usage` with existing sessions
- [ ] 5.6 Manual test: tab title updates during agent run
- [ ] 5.7 Manual test: guidance file loaded when present
