#![allow(dead_code)]
//! Ralph-Wiggum — long-running iterative task loop.
//!
//! Each loop delegates individual iterations to an ephemeral sub-agent via
//! `SpawnAgentTool`. State is persisted to `.ralph/<name>/state.json` after
//! every iteration so that loops can be paused and resumed.

use std::path::PathBuf;

use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use pi_tools::Tool; // needed to call .execute()

// ─── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RalphState {
    pub name: String,
    pub status: RalphStatus,
    pub iteration: u32,
    pub max_iterations: u32,
    pub reflection_interval: u32,
    pub task_file: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RalphStatus {
    Active,
    Paused,
    Completed,
    MaxIterationsReached,
}

// ─── Directory helpers ────────────────────────────────────────────────────────

/// Root ralph directory relative to the project.
pub fn ralph_dir() -> PathBuf {
    PathBuf::from(".ralph")
}

fn loop_dir(name: &str) -> PathBuf {
    ralph_dir().join(name)
}

fn state_path(name: &str) -> PathBuf {
    loop_dir(name).join("state.json")
}

fn archive_dir() -> PathBuf {
    ralph_dir().join("archive")
}

// ─── State I/O ────────────────────────────────────────────────────────────────

/// Load state for a named loop. Returns `None` if the state file does not exist.
pub fn load_state(name: &str) -> anyhow::Result<Option<RalphState>> {
    let path = state_path(name);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read state file: {}", path.display()))?;
    let state: RalphState = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse state file: {}", path.display()))?;
    Ok(Some(state))
}

/// Persist state for a named loop. Creates the loop directory if needed.
pub fn save_state(state: &RalphState) -> anyhow::Result<()> {
    let dir = loop_dir(&state.name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create loop directory: {}", dir.display()))?;
    let path = state_path(&state.name);
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, json)
        .with_context(|| format!("Failed to write state file: {}", path.display()))?;
    Ok(())
}

// ─── List / archive ───────────────────────────────────────────────────────────

/// List all loops (active and paused) found directly under `.ralph/`.
pub fn list_loops() -> anyhow::Result<Vec<RalphState>> {
    let dir = ralph_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut loops = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("Failed to read ralph directory: {}", dir.display()))?
    {
        let entry = entry?;
        // Skip the `archive` subdirectory.
        if entry.file_name() == "archive" {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(state) = load_state(&name)? {
            loops.push(state);
        }
    }
    Ok(loops)
}

/// Move a loop directory from `.ralph/<name>/` to `.ralph/archive/<name>/`.
pub fn archive_loop(name: &str) -> anyhow::Result<()> {
    let src = loop_dir(name);
    if !src.exists() {
        anyhow::bail!("Loop '{}' not found at {}", name, src.display());
    }
    let archive = archive_dir();
    std::fs::create_dir_all(&archive)
        .with_context(|| format!("Failed to create archive directory: {}", archive.display()))?;
    let dst = archive.join(name);
    std::fs::rename(&src, &dst).with_context(|| {
        format!(
            "Failed to move loop '{}' from {} to {}",
            name,
            src.display(),
            dst.display()
        )
    })?;
    println!("Loop '{name}' archived to {}", dst.display());
    Ok(())
}

// ─── Reflection prompt ────────────────────────────────────────────────────────

fn reflection_prompt(iteration: u32, max: u32) -> String {
    format!(
        "REFLECTION CHECKPOINT (iteration {iteration}/{max}):\n\
         Before continuing, assess your progress:\n\
         1. What have you accomplished so far?\n\
         2. Are you on the right track or do you need to adjust your approach?\n\
         3. What are the remaining steps?\n\
         4. Are there any blockers?\n\
         Reflect briefly, then continue with the next step of the task.\n\n"
    )
}

// ─── Main loop ────────────────────────────────────────────────────────────────

/// Run the ralph loop — the main function called from the REPL.
///
/// Loads or creates state, then iteratively spawns sub-agents until:
/// - cancellation is requested,
/// - the sub-agent signals `<complete>DONE</complete>`, or
/// - `max_iterations` is reached.
pub async fn run_ralph_loop(
    name: &str,
    spawn_tool: &dyn Tool,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    // ── 1. Load existing state or error if no task file exists ───────────────
    let mut state = match load_state(name)? {
        Some(s) if s.status == RalphStatus::Paused => {
            println!("Resuming loop '{}' from iteration {}.", name, s.iteration);
            s
        }
        Some(s) if s.status == RalphStatus::Active => {
            println!(
                "Loop '{}' is already active at iteration {}.",
                name, s.iteration
            );
            s
        }
        Some(s) => {
            anyhow::bail!(
                "Loop '{}' has status '{:?}' and cannot be started again.",
                name,
                s.status
            );
        }
        None => {
            // Brand-new loop — task.md must already exist.
            let task_file = loop_dir(name).join("task.md");
            if !task_file.exists() {
                anyhow::bail!(
                    "Task file not found: {}\n\
                     Create it before starting the loop.",
                    task_file.display()
                );
            }
            let task_file_str = task_file.to_string_lossy().to_string();
            let now = Utc::now().to_rfc3339();
            let new_state = RalphState {
                name: name.to_string(),
                status: RalphStatus::Active,
                iteration: 0,
                max_iterations: 50,
                reflection_interval: 5,
                task_file: task_file_str,
                created_at: now.clone(),
                updated_at: now,
            };
            save_state(&new_state)?;
            println!("Created new loop '{name}'.");
            new_state
        }
    };

    // Mark active and persist.
    state.status = RalphStatus::Active;
    state.updated_at = Utc::now().to_rfc3339();
    save_state(&state)?;

    // ── 2. Read task content ─────────────────────────────────────────────────
    let task_content = std::fs::read_to_string(&state.task_file)
        .with_context(|| format!("Failed to read task file: {}", state.task_file))?;

    // ── 3. Iterate ───────────────────────────────────────────────────────────
    loop {
        // Check cancellation before each iteration.
        if cancel.is_cancelled() {
            println!(
                "Loop '{name}' cancelled — pausing at iteration {}.",
                state.iteration
            );
            state.status = RalphStatus::Paused;
            state.updated_at = Utc::now().to_rfc3339();
            save_state(&state)?;
            break;
        }

        // Check max iterations.
        if state.iteration >= state.max_iterations {
            println!(
                "Loop '{name}' reached max iterations ({}).",
                state.max_iterations
            );
            state.status = RalphStatus::MaxIterationsReached;
            state.updated_at = Utc::now().to_rfc3339();
            save_state(&state)?;
            break;
        }

        let current_iter = state.iteration + 1;

        // Build iteration prompt.
        let mut prompt = String::new();

        // Inject reflection checkpoint every N iterations (non-zero multiples).
        if current_iter % state.reflection_interval == 0 {
            prompt.push_str(&reflection_prompt(current_iter, state.max_iterations));
        }

        prompt.push_str(&format!(
            "## Task\n{}\n\n## Iteration {}/{}\n\
             Continue making progress on the task above. \
             When the task is fully complete, output `<complete>DONE</complete>`.",
            task_content, current_iter, state.max_iterations
        ));

        println!(
            "[ralph:{name}] Iteration {}/{}...",
            current_iter, state.max_iterations
        );

        // Call spawn_tool.
        let params = serde_json::json!({ "task": prompt });
        let result = spawn_tool.execute(params, cancel.clone()).await;

        // Extract result text.
        let result_text = result
            .content
            .iter()
            .map(|c| match c {
                pi_tools::ToolContent::Text { text } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("");

        if result.is_error {
            eprintln!("[ralph:{name}] Sub-agent error: {result_text}");
        }

        // Increment iteration and persist.
        state.iteration = current_iter;
        state.updated_at = Utc::now().to_rfc3339();
        save_state(&state)?;

        // Check completion marker.
        if result_text.contains("<complete>DONE</complete>") {
            println!("[ralph:{name}] Sub-agent signaled completion.");
            state.status = RalphStatus::Completed;
            state.updated_at = Utc::now().to_rfc3339();
            save_state(&state)?;
            break;
        }
    }

    println!(
        "[ralph:{name}] Loop ended with status: {:?} (iteration {}/{}).",
        state.status, state.iteration, state.max_iterations
    );
    Ok(())
}

// ─── Argument parsing ─────────────────────────────────────────────────────────

/// Parsed arguments from `/ralph start <name> [--max N] [--reflect N]`.
#[derive(Debug)]
pub struct RalphStartArgs {
    pub name: String,
    pub max_iterations: Option<u32>,
    pub reflection_interval: Option<u32>,
}

/// Parse the argument string from `/ralph start`.
///
/// Returns an error if `args` is empty (name is required) or if a numeric
/// flag value cannot be parsed as `u32`.  Unknown flags are silently ignored.
pub fn parse_ralph_start_args(args: &str) -> anyhow::Result<RalphStartArgs> {
    let mut parts = args.split_whitespace();
    let name = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Usage: /ralph start <name> [--max N] [--reflect N]"))?
        .to_string();

    let mut max_iterations: Option<u32> = None;
    let mut reflection_interval: Option<u32> = None;

    let rest: Vec<&str> = parts.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--max" => {
                i += 1;
                if let Some(val) = rest.get(i) {
                    max_iterations = Some(
                        val.parse::<u32>()
                            .with_context(|| format!("Invalid --max value: {val}"))?,
                    );
                }
            }
            "--reflect" => {
                i += 1;
                if let Some(val) = rest.get(i) {
                    reflection_interval = Some(
                        val.parse::<u32>()
                            .with_context(|| format!("Invalid --reflect value: {val}"))?,
                    );
                }
            }
            other => {
                eprintln!("Warning: unknown argument '{other}' ignored.");
            }
        }
        i += 1;
    }

    Ok(RalphStartArgs {
        name,
        max_iterations,
        reflection_interval,
    })
}

// ─── Slash-command handlers ───────────────────────────────────────────────────

/// Handle `/ralph start <name> [--max N] [--reflect N]`
pub async fn handle_ralph_start(
    args: &str,
    spawn_tool: &dyn Tool,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let parsed = parse_ralph_start_args(args)?;
    let name = parsed.name;
    let max_iterations = parsed.max_iterations;
    let reflection_interval = parsed.reflection_interval;

    // If a new loop and flags were given, create/update state before running.
    if let Some(existing) = load_state(&name)? {
        // Resuming — only update fields if flags explicitly provided.
        let mut s = existing;
        if let Some(m) = max_iterations {
            s.max_iterations = m;
        }
        if let Some(r) = reflection_interval {
            s.reflection_interval = r;
        }
        save_state(&s)?;
    } else {
        // New loop — create the directory and a default state so we can
        // apply the flags before run_ralph_loop initialises it.
        let task_file = loop_dir(&name).join("task.md");
        if !task_file.exists() {
            anyhow::bail!(
                "Task file not found: {}\nCreate it before starting the loop.",
                task_file.display()
            );
        }
        let now = Utc::now().to_rfc3339();
        let new_state = RalphState {
            name: name.clone(),
            status: RalphStatus::Paused, // run_ralph_loop will flip to Active
            iteration: 0,
            max_iterations: max_iterations.unwrap_or(50),
            reflection_interval: reflection_interval.unwrap_or(5),
            task_file: task_file.to_string_lossy().to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        save_state(&new_state)?;
    }

    run_ralph_loop(&name, spawn_tool, cancel).await
}

/// Handle `/ralph status`
pub fn handle_ralph_status() -> anyhow::Result<()> {
    let loops = list_loops()?;
    if loops.is_empty() {
        println!("No ralph loops found under {}.", ralph_dir().display());
        return Ok(());
    }

    println!(
        "{:<20}  {:<24}  {:>9}  LAST UPDATED",
        "NAME", "STATUS", "ITERATION"
    );
    println!("{}", "-".repeat(70));
    for s in loops {
        let status_str = match s.status {
            RalphStatus::Active => "active",
            RalphStatus::Paused => "paused",
            RalphStatus::Completed => "completed",
            RalphStatus::MaxIterationsReached => "max_iterations_reached",
        };
        println!(
            "{:<20}  {:<24}  {:>4}/{:<4}  {}",
            s.name, status_str, s.iteration, s.max_iterations, s.updated_at
        );
    }
    Ok(())
}

/// Handle `/ralph stop <name>`
pub fn handle_ralph_stop(name: &str) -> anyhow::Result<()> {
    let mut state =
        load_state(name)?.ok_or_else(|| anyhow::anyhow!("Loop '{}' not found.", name))?;

    if state.status != RalphStatus::Active {
        println!(
            "Loop '{}' is not active (status: {:?}). Nothing to stop.",
            name, state.status
        );
        return Ok(());
    }

    state.status = RalphStatus::Paused;
    state.updated_at = Utc::now().to_rfc3339();
    save_state(&state)?;
    println!(
        "Loop '{}' paused at iteration {}/{}.",
        name, state.iteration, state.max_iterations
    );
    Ok(())
}

/// Handle `/ralph archive <name>`
pub fn handle_ralph_archive(name: &str) -> anyhow::Result<()> {
    archive_loop(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialise all cwd-mutating tests: ralph uses relative paths (".ralph/"),
    // so each test must chdir into its own tempdir.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Change cwd to `dir`, run `f`, then restore the original cwd.
    fn with_tmpdir<F: FnOnce() -> anyhow::Result<()>>(
        dir: &std::path::Path,
        f: F,
    ) -> anyhow::Result<()> {
        let original = std::env::current_dir()?;
        std::env::set_current_dir(dir)?;
        let result = f();
        // Always restore, even on failure.
        let _ = std::env::set_current_dir(&original);
        result
    }

    fn make_state(name: &str) -> RalphState {
        let now = chrono::Utc::now().to_rfc3339();
        RalphState {
            name: name.to_string(),
            status: RalphStatus::Active,
            iteration: 3,
            max_iterations: 50,
            reflection_interval: 5,
            task_file: format!(".ralph/{name}/task.md"),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    // ── State persistence ─────────────────────────────────────────────────────

    #[test]
    fn save_and_load_state() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            let state = make_state("myloop");
            save_state(&state)?;
            let loaded = load_state("myloop")?.expect("state should be present");
            assert_eq!(loaded.name, "myloop");
            assert_eq!(loaded.status, RalphStatus::Active);
            assert_eq!(loaded.iteration, 3);
            assert_eq!(loaded.max_iterations, 50);
            assert_eq!(loaded.reflection_interval, 5);
            assert_eq!(loaded.task_file, state.task_file);
            assert_eq!(loaded.created_at, state.created_at);
            assert_eq!(loaded.updated_at, state.updated_at);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            let result = load_state("no-such-loop")?;
            assert!(
                result.is_none(),
                "loading non-existent loop should return None"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn save_creates_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            let state = make_state("brand-new");
            // Directory does not yet exist.
            assert!(!std::path::Path::new(".ralph/brand-new").exists());
            save_state(&state)?;
            assert!(
                std::path::Path::new(".ralph/brand-new/state.json").exists(),
                "save_state should auto-create the loop directory"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn state_json_format() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            let mut state = make_state("jsoncheck");
            state.status = RalphStatus::MaxIterationsReached;
            save_state(&state)?;
            let raw = std::fs::read_to_string(".ralph/jsoncheck/state.json")?;
            // Keys must exist
            assert!(raw.contains("\"name\""), "JSON must have 'name' key");
            assert!(raw.contains("\"status\""), "JSON must have 'status' key");
            assert!(
                raw.contains("\"iteration\""),
                "JSON must have 'iteration' key"
            );
            assert!(
                raw.contains("\"max_iterations\""),
                "JSON must have 'max_iterations' key"
            );
            assert!(
                raw.contains("\"reflection_interval\""),
                "JSON must have 'reflection_interval' key"
            );
            // Status must be snake_case
            assert!(
                raw.contains("\"max_iterations_reached\""),
                "MaxIterationsReached must serialize as snake_case 'max_iterations_reached'"
            );
            Ok(())
        })
        .unwrap();
    }

    // ── Status enum serialization ─────────────────────────────────────────────

    #[test]
    fn status_serialization() {
        assert_eq!(
            serde_json::to_string(&RalphStatus::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&RalphStatus::Paused).unwrap(),
            "\"paused\""
        );
        assert_eq!(
            serde_json::to_string(&RalphStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&RalphStatus::MaxIterationsReached).unwrap(),
            "\"max_iterations_reached\""
        );
    }

    #[test]
    fn status_deserialization() {
        assert_eq!(
            serde_json::from_str::<RalphStatus>("\"active\"").unwrap(),
            RalphStatus::Active
        );
        assert_eq!(
            serde_json::from_str::<RalphStatus>("\"paused\"").unwrap(),
            RalphStatus::Paused
        );
        assert_eq!(
            serde_json::from_str::<RalphStatus>("\"completed\"").unwrap(),
            RalphStatus::Completed
        );
        assert_eq!(
            serde_json::from_str::<RalphStatus>("\"max_iterations_reached\"").unwrap(),
            RalphStatus::MaxIterationsReached
        );
    }

    // ── list_loops ────────────────────────────────────────────────────────────

    #[test]
    fn list_loops_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            // No .ralph directory at all.
            let loops = list_loops()?;
            assert!(loops.is_empty(), "no .ralph dir → empty vec");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_loops_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            let names = ["alpha", "beta", "gamma"];
            for name in &names {
                let mut s = make_state(name);
                s.status = RalphStatus::Paused;
                save_state(&s)?;
            }
            let loops = list_loops()?;
            assert_eq!(loops.len(), 3, "should return all 3 loops");
            let mut found: Vec<String> = loops.into_iter().map(|s| s.name).collect();
            found.sort();
            assert_eq!(found, vec!["alpha", "beta", "gamma"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_loops_skips_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            // Create a real loop.
            save_state(&make_state("realloop"))?;
            // Create archive directory with a loop inside it (should be skipped).
            let archive = std::path::Path::new(".ralph/archive/oldloop");
            std::fs::create_dir_all(archive)?;
            let archived = make_state("oldloop");
            let json = serde_json::to_string_pretty(&archived)?;
            std::fs::write(archive.join("state.json"), json)?;

            let loops = list_loops()?;
            assert_eq!(loops.len(), 1, "archive subdir must be skipped");
            assert_eq!(loops[0].name, "realloop");
            Ok(())
        })
        .unwrap();
    }

    // ── archive ───────────────────────────────────────────────────────────────

    #[test]
    fn archive_moves_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            save_state(&make_state("toarchive"))?;
            assert!(std::path::Path::new(".ralph/toarchive").exists());

            archive_loop("toarchive")?;

            assert!(
                !std::path::Path::new(".ralph/toarchive").exists(),
                "source directory should be gone after archive"
            );
            assert!(
                std::path::Path::new(".ralph/archive/toarchive").exists(),
                "destination in archive/ should exist"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn archive_nonexistent_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        with_tmpdir(tmp.path(), || {
            let err = archive_loop("ghost");
            assert!(
                err.is_err(),
                "archiving non-existent loop must return error"
            );
            let msg = format!("{}", err.unwrap_err());
            assert!(
                msg.contains("ghost"),
                "error message should mention the loop name"
            );
            Ok(())
        })
        .unwrap();
    }

    // ── Completion marker ─────────────────────────────────────────────────────

    /// The completion check lives inline in run_ralph_loop; we test the exact
    /// string used there via a helper closure that mirrors the production code.
    fn is_complete(text: &str) -> bool {
        text.contains("<complete>DONE</complete>")
    }

    #[test]
    fn completion_marker_detected() {
        assert!(
            is_complete("all done <complete>DONE</complete> finished"),
            "<complete>DONE</complete> must be detected"
        );
    }

    #[test]
    fn completion_marker_not_in_partial() {
        assert!(
            !is_complete("still working <complete>"),
            "opening tag alone must NOT be detected"
        );
        assert!(
            !is_complete("<complete>DON"),
            "incomplete content must NOT be detected"
        );
    }

    #[test]
    fn completion_marker_case_sensitive() {
        assert!(
            !is_complete("<complete>done</complete>"),
            "lowercase 'done' must NOT match — marker is case-sensitive"
        );
        assert!(
            !is_complete("<COMPLETE>DONE</COMPLETE>"),
            "uppercase tags must NOT match"
        );
    }

    // ── Reflection prompt ─────────────────────────────────────────────────────

    #[test]
    fn reflection_prompt_content() {
        let prompt = reflection_prompt(10, 50);
        assert!(
            prompt.contains("REFLECTION CHECKPOINT"),
            "reflection prompt must contain 'REFLECTION CHECKPOINT'"
        );
        assert!(
            prompt.contains("10"),
            "reflection prompt must contain current iteration number"
        );
        assert!(
            prompt.contains("50"),
            "reflection prompt must contain max iterations"
        );
    }

    #[test]
    fn reflection_at_correct_intervals() {
        // interval=5: multiples of 5 get reflection, others don't.
        let interval = 5u32;
        let max = 20u32;

        for iter in 1..=max {
            let should_reflect = iter % interval == 0;
            let prompt = if should_reflect {
                let rp = reflection_prompt(iter, max);
                // Build the full prompt like run_ralph_loop does.
                format!("{rp}## Task\n...")
            } else {
                "## Task\n...".to_string()
            };

            if should_reflect {
                assert!(
                    prompt.contains("REFLECTION CHECKPOINT"),
                    "iteration {iter} should include reflection (interval={interval})"
                );
            } else {
                assert!(
                    !prompt.contains("REFLECTION CHECKPOINT"),
                    "iteration {iter} should NOT include reflection (interval={interval})"
                );
            }
        }
    }

    // ── parse_ralph_start_args ────────────────────────────────────────────────

    #[test]
    fn parse_name_only_uses_defaults() {
        let parsed = parse_ralph_start_args("myloop").unwrap();
        assert_eq!(parsed.name, "myloop");
        assert_eq!(parsed.max_iterations, None, "no --max flag → None");
        assert_eq!(parsed.reflection_interval, None, "no --reflect flag → None");
    }

    #[test]
    fn parse_with_max_flag() {
        let parsed = parse_ralph_start_args("loopmax --max 100").unwrap();
        assert_eq!(parsed.name, "loopmax");
        assert_eq!(parsed.max_iterations, Some(100));
        assert_eq!(parsed.reflection_interval, None);
    }

    #[test]
    fn parse_with_reflect_flag() {
        let parsed = parse_ralph_start_args("loopreflect --reflect 3").unwrap();
        assert_eq!(parsed.name, "loopreflect");
        assert_eq!(parsed.max_iterations, None);
        assert_eq!(parsed.reflection_interval, Some(3));
    }

    #[test]
    fn parse_with_both_flags() {
        let parsed = parse_ralph_start_args("myloop --max 20 --reflect 2").unwrap();
        assert_eq!(parsed.name, "myloop");
        assert_eq!(parsed.max_iterations, Some(20));
        assert_eq!(parsed.reflection_interval, Some(2));
    }

    #[test]
    fn parse_empty_args() {
        let result = parse_ralph_start_args("");
        assert!(result.is_err(), "empty args → error (name is required)");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Usage"),
            "error message should contain usage hint"
        );
    }

    #[test]
    fn parse_max_zero() {
        let parsed = parse_ralph_start_args("myloop --max 0").unwrap();
        assert_eq!(
            parsed.max_iterations,
            Some(0),
            "--max 0 should parse to Some(0)"
        );
    }

    #[test]
    fn parse_invalid_number() {
        let result = parse_ralph_start_args("myloop --max notanumber");
        assert!(result.is_err(), "non-numeric --max value → error");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Invalid --max value"),
            "error should mention the bad value"
        );
    }

    #[test]
    fn parse_unknown_flag() {
        // Unknown flags are silently ignored; the parse should succeed.
        let parsed = parse_ralph_start_args("myloop --unknown-flag foo").unwrap();
        assert_eq!(parsed.name, "myloop");
        assert_eq!(parsed.max_iterations, None);
        assert_eq!(parsed.reflection_interval, None);
    }

    #[test]
    fn parse_max_without_value() {
        // --max at end of args with no following value → max_iterations stays None (no crash).
        let parsed = parse_ralph_start_args("myloop --max").unwrap();
        assert_eq!(
            parsed.max_iterations, None,
            "--max with no value → None, no panic"
        );
    }

    // ── Integration tests with mock tool ────────────────────────────────────

    use async_trait::async_trait;
    use pi_tools::{ToolContent, ToolResult};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A mock tool that returns a configurable response.
    /// Completes on the Nth call by including the DONE marker.
    struct MockSpawnTool {
        complete_on_iteration: u32,
        call_count: AtomicU32,
    }

    impl MockSpawnTool {
        fn new(complete_on: u32) -> Self {
            Self {
                complete_on_iteration: complete_on,
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl Tool for MockSpawnTool {
        fn name(&self) -> &str {
            "spawn_agent"
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> ToolResult {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
            let text = if n >= self.complete_on_iteration {
                format!("Iteration {n} done. <complete>DONE</complete>")
            } else {
                format!("Iteration {n} progress...")
            };
            ToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: false,
            }
        }
    }

    #[tokio::test]
    async fn run_ralph_loop_completes_on_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Setup task file
        std::fs::create_dir_all(".ralph/integ/").unwrap();
        std::fs::write(".ralph/integ/task.md", "do something").unwrap();

        let mock = MockSpawnTool::new(3); // complete on 3rd iteration
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = run_ralph_loop("integ", &mock, cancel).await;
        assert!(result.is_ok(), "loop should complete: {result:?}");

        let state = load_state("integ").unwrap().unwrap();
        assert_eq!(state.status, RalphStatus::Completed, "should be Completed");
        assert_eq!(state.iteration, 3, "should have run 3 iterations");

        let _ = std::env::set_current_dir(&original);
    }

    #[tokio::test]
    async fn run_ralph_loop_hits_max_iterations() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Setup with max_iterations=3, but mock never completes
        std::fs::create_dir_all(".ralph/maxloop/").unwrap();
        std::fs::write(".ralph/maxloop/task.md", "infinite task").unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        let initial = RalphState {
            name: "maxloop".to_string(),
            status: RalphStatus::Paused,
            iteration: 0,
            max_iterations: 3,
            reflection_interval: 5,
            task_file: ".ralph/maxloop/task.md".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        save_state(&initial).unwrap();

        let mock = MockSpawnTool::new(999); // never completes
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = run_ralph_loop("maxloop", &mock, cancel).await;
        assert!(result.is_ok());

        let state = load_state("maxloop").unwrap().unwrap();
        assert_eq!(state.status, RalphStatus::MaxIterationsReached);
        assert_eq!(state.iteration, 3);

        let _ = std::env::set_current_dir(&original);
    }

    #[tokio::test]
    async fn run_ralph_loop_cancellation() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".ralph/cancelloop/").unwrap();
        std::fs::write(".ralph/cancelloop/task.md", "task").unwrap();

        let mock = MockSpawnTool::new(999);
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel(); // cancel immediately

        let result = run_ralph_loop("cancelloop", &mock, cancel).await;
        assert!(result.is_ok());

        let state = load_state("cancelloop").unwrap().unwrap();
        assert_eq!(state.status, RalphStatus::Paused, "cancelled → Paused");

        let _ = std::env::set_current_dir(&original);
    }

    #[tokio::test]
    async fn run_ralph_loop_completed_cannot_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut state = make_state("doneloop");
        state.status = RalphStatus::Completed;
        save_state(&state).unwrap();

        let mock = MockSpawnTool::new(1);
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = run_ralph_loop("doneloop", &mock, cancel).await;
        assert!(result.is_err(), "completed loop should fail to restart");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("cannot be started again"), "error: {msg}");

        let _ = std::env::set_current_dir(&original);
    }

    #[tokio::test]
    async fn run_ralph_loop_no_task_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Don't create task.md
        let mock = MockSpawnTool::new(1);
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = run_ralph_loop("nofile", &mock, cancel).await;
        assert!(result.is_err(), "missing task.md should error");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("Task file not found"), "error: {msg}");

        let _ = std::env::set_current_dir(&original);
    }

    #[tokio::test]
    async fn run_ralph_loop_resume_from_paused() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = TEST_LOCK.lock().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Create a paused state at iteration 2
        std::fs::create_dir_all(".ralph/resumeloop/").unwrap();
        std::fs::write(".ralph/resumeloop/task.md", "resume task").unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let paused = RalphState {
            name: "resumeloop".to_string(),
            status: RalphStatus::Paused,
            iteration: 2,
            max_iterations: 5,
            reflection_interval: 5,
            task_file: ".ralph/resumeloop/task.md".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        save_state(&paused).unwrap();

        // Mock completes on call 2 (which is iteration 4, since we resume from 2)
        let mock = MockSpawnTool::new(2);
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = run_ralph_loop("resumeloop", &mock, cancel).await;
        assert!(result.is_ok());

        let state = load_state("resumeloop").unwrap().unwrap();
        assert_eq!(state.status, RalphStatus::Completed);
        assert_eq!(state.iteration, 4, "resumed from 2, completed on 4th");

        let _ = std::env::set_current_dir(&original);
    }
}
