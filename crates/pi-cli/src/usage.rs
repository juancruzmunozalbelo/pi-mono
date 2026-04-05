use std::collections::{HashMap, HashSet};

use chrono::{Local, NaiveDate};
use pi_ai::Message;

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Period {
    Today,
    Week,
    All,
}

#[derive(Debug, Default)]
pub struct ModelStats {
    pub sessions: u32,
    pub messages: u32,
    pub cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Debug, Default)]
pub struct ProviderStats {
    pub models: HashMap<String, ModelStats>,
    pub totals: ModelStats,
}

pub struct UsageReport {
    pub providers: HashMap<String, ProviderStats>,
    pub grand_total: ModelStats,
}

// ─── Session analysis ─────────────────────────────────────────────────────────

/// Analyze all sessions from `~/.pi/sessions/` and produce a `UsageReport`.
pub fn analyze_sessions(period: Period) -> anyhow::Result<UsageReport> {
    analyze_sessions_from_dir(&crate::session::sessions_dir(), period)
}

/// Analyze all sessions from the given directory and produce a `UsageReport`.
pub fn analyze_sessions_from_dir(
    sessions_dir: &std::path::Path,
    period: Period,
) -> anyhow::Result<UsageReport> {
    let mut report = UsageReport {
        providers: HashMap::new(),
        grand_total: ModelStats::default(),
    };

    if !sessions_dir.exists() {
        return Ok(report);
    }

    let today: NaiveDate = Local::now().date_naive();

    for entry in std::fs::read_dir(sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        // Period filtering: use the file's modification time as a proxy.
        let include = match period {
            Period::All => true,
            Period::Today | Period::Week => {
                let mtime = entry.metadata()?.modified()?;
                let mtime_local: chrono::DateTime<Local> = mtime.into();
                let mdate = mtime_local.date_naive();
                match period {
                    Period::Today => mdate == today,
                    Period::Week => (0..7).contains(&(today - mdate).num_days()),
                    Period::All => unreachable!(),
                }
            }
        };

        if !include {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let session: crate::session::Session = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Provider comes from the session's top-level `model` field (format
        // may be "provider:model_id" or just "model_id").
        let (provider_name, model_name) = split_model(&session.model);

        // Deduplicate within this session by (input, output, total_tokens).
        let mut seen: HashSet<(u64, u64, u64)> = HashSet::new();
        let mut session_had_messages = false;

        for msg in &session.messages {
            if let Message::Assistant {
                usage: Some(usage), ..
            } = msg
            {
                let dedup_key = (usage.input, usage.output, usage.total_tokens);
                if !seen.insert(dedup_key) {
                    // Duplicate entry — skip.
                    continue;
                }

                session_had_messages = true;

                let pstats = report.providers.entry(provider_name.clone()).or_default();
                let mstats = pstats.models.entry(model_name.clone()).or_default();

                mstats.messages += 1;
                mstats.input_tokens += usage.input;
                mstats.output_tokens += usage.output;
                mstats.cache_read_tokens += usage.cache_read;
                mstats.cache_write_tokens += usage.cache_write;
                // Cost is not tracked per-message in our current format.

                pstats.totals.messages += 1;
                pstats.totals.input_tokens += usage.input;
                pstats.totals.output_tokens += usage.output;
                pstats.totals.cache_read_tokens += usage.cache_read;
                pstats.totals.cache_write_tokens += usage.cache_write;

                report.grand_total.messages += 1;
                report.grand_total.input_tokens += usage.input;
                report.grand_total.output_tokens += usage.output;
                report.grand_total.cache_read_tokens += usage.cache_read;
                report.grand_total.cache_write_tokens += usage.cache_write;
            }
        }

        // Count sessions: one session contributes 1 to each group it appears in,
        // but only if it had at least one valid assistant message.
        if session_had_messages {
            let pstats = report.providers.entry(provider_name.clone()).or_default();
            let mstats = pstats.models.entry(model_name.clone()).or_default();
            mstats.sessions += 1;
            pstats.totals.sessions += 1;
            report.grand_total.sessions += 1;
        }
    }

    Ok(report)
}

/// Split a model string like `"provider:model_id"` or `"model_id"` into
/// `(provider, model_id)`. Falls back to `"unknown"` for the provider.
fn split_model(model: &str) -> (String, String) {
    if let Some((prov, mid)) = model.split_once(':') {
        (prov.to_string(), mid.to_string())
    } else {
        ("unknown".to_string(), model.to_string())
    }
}

// ─── Display ──────────────────────────────────────────────────────────────────

/// Write a usage report to any `impl Write`.  Used by [`display_usage`] and by
/// tests that capture output to a `Vec<u8>` buffer.
pub fn display_usage_to_writer(
    report: &UsageReport,
    w: &mut impl std::io::Write,
) -> std::io::Result<()> {
    if report.providers.is_empty() {
        writeln!(w, "No session data available for the selected period.")?;
        return Ok(());
    }

    writeln!(
        w,
        "{:<25} {:>8} {:>8} {:>10} {:>12} {:>12}",
        "Provider / Model", "Sess", "Msgs", "Cost", "In Tokens", "Out Tokens"
    )?;
    writeln!(w, "{}", "─".repeat(77))?;

    // Sort providers for stable output.
    let mut providers: Vec<(&String, &ProviderStats)> = report.providers.iter().collect();
    providers.sort_by_key(|(name, _)| name.as_str());

    for (provider, pstats) in providers {
        // Provider total row.
        writeln!(
            w,
            "{:<25} {:>8} {:>8} {:>10.4} {:>12} {:>12}",
            provider,
            pstats.totals.sessions,
            pstats.totals.messages,
            pstats.totals.cost,
            pstats.totals.input_tokens,
            pstats.totals.output_tokens,
        )?;

        // Model rows (indented), sorted for stability.
        let mut models: Vec<(&String, &ModelStats)> = pstats.models.iter().collect();
        models.sort_by_key(|(name, _)| name.as_str());

        for (model, mstats) in models {
            writeln!(
                w,
                "  {:<23} {:>8} {:>8} {:>10.4} {:>12} {:>12}",
                model,
                mstats.sessions,
                mstats.messages,
                mstats.cost,
                mstats.input_tokens,
                mstats.output_tokens,
            )?;
        }
    }

    // Grand total row.
    writeln!(w, "{}", "─".repeat(77))?;
    writeln!(
        w,
        "{:<25} {:>8} {:>8} {:>10.4} {:>12} {:>12}",
        "TOTAL",
        report.grand_total.sessions,
        report.grand_total.messages,
        report.grand_total.cost,
        report.grand_total.input_tokens,
        report.grand_total.output_tokens,
    )?;
    Ok(())
}

pub fn display_usage(report: &UsageReport) {
    display_usage_to_writer(report, &mut std::io::stdout().lock()).unwrap();
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run_usage(period: Period) -> anyhow::Result<()> {
    let report = analyze_sessions(period)?;
    display_usage(&report);
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_usage(input: u64, output: u64, total_tokens: u64) -> serde_json::Value {
        serde_json::json!({
            "input": input,
            "output": output,
            "cache_read": 0,
            "cache_write": 0,
            "total_tokens": total_tokens
        })
    }

    fn assistant_msg(usage: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}],
            "usage": usage
        })
    }

    fn assistant_msg_no_usage() -> serde_json::Value {
        serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}]
        })
    }

    fn user_msg() -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })
    }

    fn tool_result_msg() -> serde_json::Value {
        serde_json::json!({
            "role": "tool_result",
            "tool_call_id": "call_1",
            "tool_name": "bash",
            "content": [],
            "is_error": false
        })
    }

    fn write_test_session(dir: &Path, id: &str, model: &str, messages: Vec<serde_json::Value>) {
        let session = serde_json::json!({
            "id": id,
            "model": model,
            "messages": messages,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        });
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_string(&session).unwrap(),
        )
        .unwrap();
    }

    // ── Session parsing tests ─────────────────────────────────────────────────

    #[test]
    fn empty_sessions_dir() {
        let dir = tempdir().unwrap();
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(report.providers.is_empty());
        assert_eq!(report.grand_total.sessions, 0);
        assert_eq!(report.grand_total.messages, 0);
    }

    #[test]
    fn nonexistent_sessions_dir() {
        let dir = tempdir().unwrap();
        let nonexistent = dir.path().join("does_not_exist");
        let report = analyze_sessions_from_dir(&nonexistent, Period::All).unwrap();
        assert!(report.providers.is_empty());
        assert_eq!(report.grand_total.sessions, 0);
    }

    #[test]
    fn single_session_one_message() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "copilot:gpt-4o",
            vec![assistant_msg(make_usage(100, 50, 150))],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.sessions, 1);
        assert_eq!(report.grand_total.messages, 1);
        assert_eq!(report.grand_total.input_tokens, 100);
        assert_eq!(report.grand_total.output_tokens, 50);
    }

    #[test]
    fn single_session_multiple_messages() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![
                assistant_msg(make_usage(100, 50, 150)),
                assistant_msg(make_usage(200, 80, 280)),
            ],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.sessions, 1);
        assert_eq!(report.grand_total.messages, 2);
        assert_eq!(report.grand_total.input_tokens, 300);
        assert_eq!(report.grand_total.output_tokens, 130);
    }

    #[test]
    fn user_messages_ignored() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![user_msg(), user_msg()],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(report.providers.is_empty());
        assert_eq!(report.grand_total.sessions, 0);
        assert_eq!(report.grand_total.messages, 0);
    }

    #[test]
    fn tool_result_messages_ignored() {
        let dir = tempdir().unwrap();
        write_test_session(dir.path(), "sess1", "openai:gpt-4", vec![tool_result_msg()]);
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(report.providers.is_empty());
        assert_eq!(report.grand_total.sessions, 0);
    }

    #[test]
    fn assistant_without_usage_ignored() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![assistant_msg_no_usage()],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(report.providers.is_empty());
        assert_eq!(report.grand_total.sessions, 0);
        assert_eq!(report.grand_total.messages, 0);
    }

    #[test]
    fn deduplication() {
        // Two assistant messages with identical (input, output, total_tokens) → counted once
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![
                assistant_msg(make_usage(100, 50, 150)),
                assistant_msg(make_usage(100, 50, 150)),
            ],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.messages, 1);
        assert_eq!(report.grand_total.input_tokens, 100);
        assert_eq!(report.grand_total.output_tokens, 50);
    }

    #[test]
    fn deduplication_different_values() {
        // Two messages with different tokens → both counted
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![
                assistant_msg(make_usage(100, 50, 150)),
                assistant_msg(make_usage(101, 50, 151)),
            ],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.messages, 2);
        assert_eq!(report.grand_total.input_tokens, 201);
    }

    #[test]
    fn multiple_sessions() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![assistant_msg(make_usage(100, 50, 150))],
        );
        write_test_session(
            dir.path(),
            "sess2",
            "openai:gpt-4",
            vec![assistant_msg(make_usage(200, 80, 280))],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.sessions, 2);
        assert_eq!(report.grand_total.messages, 2);
        assert_eq!(report.grand_total.input_tokens, 300);
        assert_eq!(report.grand_total.output_tokens, 130);
    }

    #[test]
    fn model_split_with_colon() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "copilot:gpt-4o",
            vec![assistant_msg(make_usage(10, 5, 15))],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(
            report.providers.contains_key("copilot"),
            "expected provider 'copilot'"
        );
        let pstats = &report.providers["copilot"];
        assert!(
            pstats.models.contains_key("gpt-4o"),
            "expected model 'gpt-4o'"
        );
    }

    #[test]
    fn model_split_without_colon() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess1",
            "gpt-4o",
            vec![assistant_msg(make_usage(10, 5, 15))],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(
            report.providers.contains_key("unknown"),
            "expected provider 'unknown'"
        );
        let pstats = &report.providers["unknown"];
        assert!(
            pstats.models.contains_key("gpt-4o"),
            "expected model 'gpt-4o'"
        );
    }

    #[test]
    fn invalid_json_session_skipped() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("bad.json"), b"not valid json {{{{").unwrap();
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert!(report.providers.is_empty());
        assert_eq!(report.grand_total.sessions, 0);
    }

    #[test]
    fn non_json_files_skipped() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"some text").unwrap();
        // Also add a valid session to confirm only .json is processed
        write_test_session(
            dir.path(),
            "sess1",
            "openai:gpt-4",
            vec![assistant_msg(make_usage(10, 5, 15))],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.sessions, 1);
    }

    // ── Display tests (output-capturing) ─────────────────────────────────────

    fn capture_display(report: &UsageReport) -> String {
        let mut buf: Vec<u8> = Vec::new();
        display_usage_to_writer(report, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn make_report_with_provider(
        provider: &str,
        model: &str,
        input: u64,
        output: u64,
    ) -> UsageReport {
        let mut report = UsageReport {
            providers: HashMap::new(),
            grand_total: ModelStats::default(),
        };
        let mut pstats = ProviderStats::default();
        let mut mstats = ModelStats::default();
        mstats.sessions = 1;
        mstats.messages = 1;
        mstats.input_tokens = input;
        mstats.output_tokens = output;
        pstats.models.insert(model.to_string(), mstats);
        pstats.totals.sessions = 1;
        pstats.totals.messages = 1;
        pstats.totals.input_tokens = input;
        pstats.totals.output_tokens = output;
        report.providers.insert(provider.to_string(), pstats);
        report.grand_total.sessions = 1;
        report.grand_total.messages = 1;
        report.grand_total.input_tokens = input;
        report.grand_total.output_tokens = output;
        report
    }

    #[test]
    fn display_empty_shows_no_data() {
        let report = UsageReport {
            providers: HashMap::new(),
            grand_total: ModelStats::default(),
        };
        let output = capture_display(&report);
        assert!(
            output.contains("No session data"),
            "empty report should output 'No session data'; got: {output:?}"
        );
    }

    #[test]
    fn display_shows_provider_model_total() {
        let report = make_report_with_provider("openai", "gpt-4o", 100, 50);
        let output = capture_display(&report);
        assert!(
            output.contains("openai"),
            "output should contain provider name"
        );
        assert!(
            output.contains("gpt-4o"),
            "output should contain model name"
        );
        assert!(
            output.contains("TOTAL"),
            "output should contain 'TOTAL' row"
        );
    }

    #[test]
    fn display_shows_token_counts() {
        let report = make_report_with_provider("anthropic", "claude-3", 12345, 6789);
        let output = capture_display(&report);
        assert!(
            output.contains("12345"),
            "output should contain input token count 12345; got: {output:?}"
        );
        assert!(
            output.contains("6789"),
            "output should contain output token count 6789; got: {output:?}"
        );
    }

    #[test]
    fn display_sorted_providers() {
        let mut report = UsageReport {
            providers: HashMap::new(),
            grand_total: ModelStats::default(),
        };
        for name in &["zzz-provider", "aaa-provider", "mmm-provider"] {
            let mut pstats = ProviderStats::default();
            let mut mstats = ModelStats::default();
            mstats.sessions = 1;
            mstats.messages = 2;
            pstats.models.insert("model-x".to_string(), mstats);
            pstats.totals.sessions = 1;
            pstats.totals.messages = 2;
            report.providers.insert(name.to_string(), pstats);
            report.grand_total.sessions += 1;
            report.grand_total.messages += 2;
        }
        let output = capture_display(&report);
        let aaa_pos = output.find("aaa-provider").unwrap();
        let mmm_pos = output.find("mmm-provider").unwrap();
        let zzz_pos = output.find("zzz-provider").unwrap();
        assert!(
            aaa_pos < mmm_pos,
            "aaa-provider should appear before mmm-provider"
        );
        assert!(
            mmm_pos < zzz_pos,
            "mmm-provider should appear before zzz-provider"
        );
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn session_with_zero_tokens() {
        let dir = tempdir().unwrap();
        write_test_session(
            dir.path(),
            "sess_zero",
            "openai:gpt-4",
            vec![assistant_msg(make_usage(0, 0, 0))],
        );
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.input_tokens, 0);
        assert_eq!(report.grand_total.output_tokens, 0);
        assert_eq!(report.grand_total.messages, 1);

        // Display should not panic on zero-token stats.
        let output = capture_display(&report);
        assert!(
            output.contains("TOTAL"),
            "zero-token report should still show TOTAL"
        );
    }

    #[test]
    fn large_token_counts_no_overflow() {
        let dir = tempdir().unwrap();
        let large = u64::MAX / 2;
        write_test_session(
            dir.path(),
            "sess_large",
            "openai:gpt-4",
            // Use distinct (input, output, total) to avoid deduplication.
            vec![assistant_msg(serde_json::json!({
                "input": large,
                "output": large / 2,
                "cache_read": 0,
                "cache_write": 0,
                "total_tokens": large
            }))],
        );
        // Should not panic.
        let report = analyze_sessions_from_dir(dir.path(), Period::All).unwrap();
        assert_eq!(report.grand_total.input_tokens, large);
        let output = capture_display(&report);
        assert!(
            output.contains("TOTAL"),
            "large token report should still display"
        );
    }

    // ── split_model unit tests ─────────────────────────────────────────────────

    #[test]
    fn split_model_with_colon() {
        let (provider, model) = split_model("copilot:gpt-4o");
        assert_eq!(provider, "copilot");
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn split_model_without_colon() {
        let (provider, model) = split_model("gpt-4o");
        assert_eq!(provider, "unknown");
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn split_model_multiple_colons_uses_first() {
        // Only the first colon is used as the split point.
        let (provider, model) = split_model("provider:model:extra");
        assert_eq!(provider, "provider");
        assert_eq!(model, "model:extra");
    }
}
