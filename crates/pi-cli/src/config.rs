use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub sub_agent: Option<SubAgentConfigToml>,
}

/// `[sub_agent]` table in `~/.pi/config.toml`.
///
/// Example:
/// ```toml
/// [sub_agent]
/// provider = "minimax"
/// model = "MiniMax-M2.7-highspeed"
/// api_key = "your-minimax-key"
/// system_prompt = "You are a fast coding sub-agent. Execute tasks precisely."
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct SubAgentConfigToml {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".pi")
}

pub fn load_config() -> Config {
    let path = config_dir().join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}
