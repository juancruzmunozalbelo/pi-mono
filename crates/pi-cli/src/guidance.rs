use std::path::{Path, PathBuf};
use tracing::debug;

/// Load guidance for a given provider from project-local or global guidance directory.
///
/// Lookup order (first found wins):
/// 1. `.pi/guidance/{PROVIDER}.md` (project-local)
/// 2. `~/.pi/guidance/{PROVIDER}.md` (global)
///
/// Returns the guidance content or None if no file found.
pub fn load_guidance(provider_name: &str, cwd: &Path) -> Option<String> {
    load_guidance_from_paths(provider_name, cwd, &global_guidance_dir())
}

/// Inner implementation that accepts an explicit global guidance directory for testability.
fn load_guidance_from_paths(provider_name: &str, cwd: &Path, global_dir: &Path) -> Option<String> {
    let filename = format!("{}.md", provider_name.to_uppercase());

    // 1. Project-local
    let local_path = cwd.join(".pi").join("guidance").join(&filename);
    if local_path.exists() {
        match std::fs::read_to_string(&local_path) {
            Ok(content) => {
                debug!("Loaded project guidance from {}", local_path.display());
                return Some(content);
            }
            Err(e) => {
                debug!("Failed to read {}: {e}", local_path.display());
            }
        }
    }

    // 2. Global
    let global_path = global_dir.join(&filename);
    if global_path.exists() {
        match std::fs::read_to_string(&global_path) {
            Ok(content) => {
                debug!("Loaded global guidance from {}", global_path.display());
                return Some(content);
            }
            Err(e) => {
                debug!("Failed to read {}: {e}", global_path.display());
            }
        }
    }

    debug!("No guidance file found for provider '{provider_name}' (looked for {filename})");
    None
}

/// Prepend guidance to an existing system prompt.
/// If system_prompt is None, returns just the guidance.
/// If guidance is None, returns the original system_prompt.
pub fn apply_guidance(
    system_prompt: Option<String>,
    provider_name: &str,
    cwd: &Path,
) -> Option<String> {
    let guidance = load_guidance(provider_name, cwd);

    match (guidance, system_prompt) {
        (Some(g), Some(sp)) => Some(format!("{g}\n---\n{sp}")),
        (Some(g), None) => Some(g),
        (None, sp) => sp,
    }
}

fn global_guidance_dir() -> PathBuf {
    crate::config::config_dir().join("guidance")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Create `.pi/guidance/<filename>` inside `cwd_dir`.
    fn write_local_guidance(cwd_dir: &std::path::Path, filename: &str, content: &str) {
        let guidance_dir = cwd_dir.join(".pi").join("guidance");
        std::fs::create_dir_all(&guidance_dir).unwrap();
        std::fs::write(guidance_dir.join(filename), content).unwrap();
    }

    /// Create `<filename>` directly in `global_dir`.
    fn write_global_guidance(global_dir: &std::path::Path, filename: &str, content: &str) {
        std::fs::create_dir_all(global_dir).unwrap();
        std::fs::write(global_dir.join(filename), content).unwrap();
    }

    // ── load_guidance_from_paths tests ────────────────────────────────────────

    #[test]
    fn load_from_project_local() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        write_local_guidance(cwd.path(), "COPILOT.md", "local guidance");
        let result = load_guidance_from_paths("copilot", cwd.path(), global.path());
        assert_eq!(result, Some("local guidance".to_string()));
    }

    #[test]
    fn load_from_global() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        write_global_guidance(global.path(), "COPILOT.md", "global guidance");
        let result = load_guidance_from_paths("copilot", cwd.path(), global.path());
        assert_eq!(result, Some("global guidance".to_string()));
    }

    #[test]
    fn project_local_takes_precedence() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        write_local_guidance(cwd.path(), "COPILOT.md", "local wins");
        write_global_guidance(global.path(), "COPILOT.md", "global loses");
        let result = load_guidance_from_paths("copilot", cwd.path(), global.path());
        assert_eq!(result, Some("local wins".to_string()));
    }

    #[test]
    fn no_guidance_returns_none() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        let result = load_guidance_from_paths("copilot", cwd.path(), global.path());
        assert_eq!(result, None);
    }

    #[test]
    fn provider_name_uppercased() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        // File must be named GITHUB-COPILOT.md for provider "github-copilot"
        write_local_guidance(cwd.path(), "GITHUB-COPILOT.md", "uppercased guidance");
        let result = load_guidance_from_paths("github-copilot", cwd.path(), global.path());
        assert_eq!(result, Some("uppercased guidance".to_string()));
    }

    #[test]
    fn empty_guidance_file() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        write_local_guidance(cwd.path(), "COPILOT.md", "");
        let result = load_guidance_from_paths("copilot", cwd.path(), global.path());
        assert_eq!(result, Some("".to_string()));
    }

    #[test]
    fn guidance_with_unicode() {
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        let content = "You are helpful. 🤖\n日本語テスト\nΩ≈ç√∫";
        write_local_guidance(cwd.path(), "COPILOT.md", content);
        let result = load_guidance_from_paths("copilot", cwd.path(), global.path());
        assert_eq!(result, Some(content.to_string()));
    }

    // ── apply_guidance tests ──────────────────────────────────────────────────
    //
    // apply_guidance calls load_guidance (which uses the real global dir).
    // We test it indirectly by placing files at the project-local path so we
    // don't need to mutate the real ~/.pi/guidance dir.

    #[test]
    fn apply_guidance_prepends() {
        let cwd = tempdir().unwrap();
        write_local_guidance(cwd.path(), "COPILOT.md", "guidance content");
        let result = apply_guidance(Some("existing prompt".to_string()), "copilot", cwd.path());
        assert_eq!(result, Some("guidance content\n---\nexisting prompt".to_string()));
    }

    #[test]
    fn apply_guidance_only_guidance() {
        let cwd = tempdir().unwrap();
        write_local_guidance(cwd.path(), "COPILOT.md", "only guidance");
        let result = apply_guidance(None, "copilot", cwd.path());
        assert_eq!(result, Some("only guidance".to_string()));
    }

    #[test]
    fn apply_guidance_only_prompt() {
        // No guidance file — should pass prompt through unchanged.
        let cwd = tempdir().unwrap();
        let _result = apply_guidance(Some("just the prompt".to_string()), "copilot", cwd.path());
        // There may or may not be a real ~/.pi/guidance/COPILOT.md on the host,
        // but the project-local wins; if no local file, result depends on real global.
        // Since we can't guarantee the real global state, only check the path where
        // no local file is present AND we know no global file for a fake provider.
        let cwd2 = tempdir().unwrap();
        let result2 = apply_guidance(Some("just the prompt".to_string()), "DEFINITELY_FAKE_PROVIDER_XYZ", cwd2.path());
        assert_eq!(result2, Some("just the prompt".to_string()));
    }

    #[test]
    fn apply_guidance_neither() {
        // No guidance file, no system prompt.
        let cwd = tempdir().unwrap();
        let result = apply_guidance(None, "DEFINITELY_FAKE_PROVIDER_XYZ", cwd.path());
        assert_eq!(result, None);
    }

    #[test]
    fn provider_with_slash() {
        // Provider names containing "/" are uppercased and used verbatim as the
        // filename stem. On most filesystems "/" is not a valid filename character,
        // so we expect no guidance to be found (not a panic).
        let cwd = tempdir().unwrap();
        let global = tempdir().unwrap();
        // "my/bad" → filename would be "MY/BAD.md" which cannot be created as a
        // single file; load_guidance_from_paths must return None gracefully.
        let result = load_guidance_from_paths("my/bad", cwd.path(), global.path());
        // The call must not panic; the result is None because no such file exists.
        assert!(
            result.is_none(),
            "provider name with '/' should return None (no file found), got: {result:?}"
        );
    }
}
