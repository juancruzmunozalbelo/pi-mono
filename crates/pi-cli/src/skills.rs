//! Skills system — loads SKILL.md files and exposes them as tools.
//!
//! Each skill runs as an isolated sub-agent (Option B) with the SKILL.md
//! instructions as the system prompt.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use pi_agent::{Agent, AgentConfig, AgentState, ToolExecutionMode};
use pi_ai::ChatEvent;
use pi_tools::{error_result, text_result, Tool, ToolResult};

use crate::spawn_agent::SubAgentConfig;

// ─── Skill definition ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub disable_model_invocation: bool,
    pub source_path: PathBuf,
}

// ─── SKILL.md parser ────────────────────────────────────────────────────────

/// Parse a SKILL.md file into a Skill struct.
/// Format:
/// ```
/// ---
/// name: my-skill
/// description: Does something
/// disable-model-invocation: false
/// ---
///
/// Markdown instructions...
/// ```
fn parse_skill_md(content: &str, path: &Path) -> Option<Skill> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }

    // Find the closing ---
    let rest = &content[3..];
    let end = rest.find("---")?;
    let frontmatter = &rest[..end];
    let instructions = rest[end + 3..].trim().to_string();

    // Parse YAML-like frontmatter (simple key: value parsing)
    let mut name = None;
    let mut description = None;
    let mut disable_model_invocation = false;

    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match key {
                "name" => name = Some(value.to_string()),
                "description" => description = Some(value.to_string()),
                "disable-model-invocation" => {
                    disable_model_invocation = value == "true";
                }
                _ => {} // ignore unknown keys
            }
        }
    }

    let name = name?;
    let description = description.unwrap_or_else(|| format!("Skill: {name}"));

    Some(Skill {
        name,
        description,
        instructions,
        disable_model_invocation,
        source_path: path.to_path_buf(),
    })
}

// ─── Skill discovery ────────────────────────────────────────────────────────

/// Discover skills from project-local and global directories.
/// Project-local skills override global ones with the same name.
pub fn load_skills(cwd: &Path) -> Vec<Skill> {
    let mut skills = std::collections::HashMap::new();

    // 1. Global skills (~/.pi/skills/)
    let global_dir = crate::config::config_dir().join("skills");
    if let Ok(found) = scan_skills_dir(&global_dir) {
        for skill in found {
            skills.insert(skill.name.clone(), skill);
        }
    }

    // 2. Project-local skills (.pi/skills/) — override global
    let local_dir = cwd.join(".pi").join("skills");
    if let Ok(found) = scan_skills_dir(&local_dir) {
        for skill in found {
            skills.insert(skill.name.clone(), skill);
        }
    }

    let result: Vec<Skill> = skills.into_values().collect();
    tracing::debug!("Loaded {} skills", result.len());
    for s in &result {
        tracing::debug!("  skill: {} ({})", s.name, s.source_path.display());
    }
    result
}

/// Scan a directory for SKILL.md files.
/// Each skill is in a subdirectory: skills/<name>/SKILL.md
fn scan_skills_dir(dir: &Path) -> anyhow::Result<Vec<Skill>> {
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut skills = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let skill_file = entry.path().join("SKILL.md");
        if skill_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&skill_file) {
                if let Some(skill) = parse_skill_md(&content, &skill_file) {
                    skills.push(skill);
                }
            }
        }
    }
    Ok(skills)
}

// ─── SkillTool — wraps a skill as a Tool ────────────────────────────────────

/// A tool that executes a skill by creating an isolated sub-agent
/// with the skill's instructions as system prompt.
pub struct SkillTool {
    skill: Skill,
    sub_config: Arc<SubAgentConfig>,
}

impl SkillTool {
    pub fn new(skill: Skill, sub_config: Arc<SubAgentConfig>) -> Self {
        Self { skill, sub_config }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        &self.skill.name
    }

    fn description(&self) -> &str {
        &self.skill.description
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "The user's request or context for this skill"
                }
            },
            "required": ["input"]
        })
    }

    async fn execute(&self, params: serde_json::Value, cancel: CancellationToken) -> ToolResult {
        let input = params
            .get("input")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Build user message combining skill instructions + user input
        let user_message = if input.is_empty() {
            self.skill.instructions.clone()
        } else {
            format!(
                "{}\n\n---\n\nUser request: {}",
                self.skill.instructions, input
            )
        };

        // Create isolated sub-agent with skill instructions as system prompt
        let config = AgentConfig {
            provider: Arc::clone(&self.sub_config.provider),
            tools: self.sub_config.tools.clone(),
            tool_execution_mode: ToolExecutionMode::Sequential,
            hooks: pi_agent::Hooks::default(),
        };

        let state = AgentState {
            messages: vec![],
            model: self.sub_config.model.clone(),
            system_prompt: Some(self.skill.instructions.clone()),
            thinking_level: None,
            is_streaming: false,
            error_message: None,
        };

        let mut agent = Agent::new(config, state);
        let mut event_rx = match agent.take_event_receiver() {
            Some(rx) => rx,
            None => return error_result("Failed to create skill sub-agent"),
        };

        let mut result_text = String::new();

        let run_fut = async {
            let prompt_handle = tokio::spawn(async move { agent.prompt(user_message).await });

            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        prompt_handle.abort();
                        return Err("Cancelled".to_string());
                    }
                    event = event_rx.recv() => {
                        match event {
                            Some(pi_agent::AgentEvent::MessageUpdate {
                                event: ChatEvent::TextDelta { text },
                            }) => {
                                result_text.push_str(&text);
                            }
                            Some(pi_agent::AgentEvent::AgentEnd { .. }) => break,
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
            }

            match prompt_handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(format!("Skill error: {e}")),
                Err(e) => Err(format!("Skill join error: {e}")),
            }
        };

        // 5 minute timeout
        match tokio::time::timeout(std::time::Duration::from_secs(300), run_fut).await {
            Ok(Ok(())) => {
                if result_text.is_empty() {
                    text_result("Skill completed but produced no text output.")
                } else {
                    text_result(result_text)
                }
            }
            Ok(Err(msg)) => error_result(msg),
            Err(_) => error_result("Skill timed out after 5 minutes"),
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_skill() {
        let content = r#"---
name: test-skill
description: A test skill
disable-model-invocation: false
---

Do something useful.

## Steps
1. First step
2. Second step
"#;
        let skill = parse_skill_md(content, Path::new("/tmp/test/SKILL.md")).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill");
        assert!(!skill.disable_model_invocation);
        assert!(skill.instructions.contains("Do something useful"));
        assert!(skill.instructions.contains("## Steps"));
    }

    #[test]
    fn parse_minimal_skill() {
        let content = "---\nname: minimal\n---\nJust do it.";
        let skill = parse_skill_md(content, Path::new("/tmp/SKILL.md")).unwrap();
        assert_eq!(skill.name, "minimal");
        assert_eq!(skill.description, "Skill: minimal");
        assert_eq!(skill.instructions, "Just do it.");
    }

    #[test]
    fn parse_no_frontmatter_returns_none() {
        let content = "Just text, no frontmatter.";
        assert!(parse_skill_md(content, Path::new("/tmp/SKILL.md")).is_none());
    }

    #[test]
    fn parse_no_name_returns_none() {
        let content = "---\ndescription: no name\n---\nContent";
        assert!(parse_skill_md(content, Path::new("/tmp/SKILL.md")).is_none());
    }

    #[test]
    fn parse_disable_model_invocation() {
        let content = "---\nname: hidden\ndisable-model-invocation: true\n---\nSecret skill";
        let skill = parse_skill_md(content, Path::new("/tmp/SKILL.md")).unwrap();
        assert!(skill.disable_model_invocation);
    }

    #[test]
    fn load_skills_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = load_skills(tmp.path());
        // May find global skills if ~/.pi/skills/ exists, but project-local should be empty
        // Just verify it doesn't crash
        let _ = skills;
    }

    #[test]
    fn load_skills_with_project_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(".pi").join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: Test\n---\nDo things",
        )
        .unwrap();

        let skills = load_skills(tmp.path());
        assert!(
            skills.iter().any(|s| s.name == "my-skill"),
            "should find project-local skill"
        );
    }
}
