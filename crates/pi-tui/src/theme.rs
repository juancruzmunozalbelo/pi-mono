//! Theme — color palette and styles for the TUI.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// All visual styles used by the TUI.
#[derive(Debug, Clone)]
pub struct Theme {
    pub user_style: Style,
    pub assistant_style: Style,
    pub tool_name_style: Style,
    pub tool_output_style: Style,
    pub error_style: Style,
    pub thinking_style: Style,
    pub status_bar_style: Style,
    pub input_style: Style,
    pub code_style: Style,
    pub border_style: Style,
    pub highlight_style: Style,
    pub muted_style: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            user_style: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            assistant_style: Style::default().fg(Color::White),
            tool_name_style: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            tool_output_style: Style::default().fg(Color::Gray),
            error_style: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            thinking_style: Style::default().fg(Color::Magenta),
            status_bar_style: Style::default().fg(Color::Black).bg(Color::DarkGray),
            input_style: Style::default().fg(Color::White),
            code_style: Style::default().fg(Color::Green).bg(Color::Rgb(30, 30, 30)),
            border_style: Style::default().fg(Color::DarkGray),
            highlight_style: Style::default().fg(Color::Black).bg(Color::Cyan),
            muted_style: Style::default().fg(Color::DarkGray),
        }
    }
}

/// TOML-deserializable color config.
#[derive(Debug, Deserialize, Serialize, Default)]
struct ThemeConfig {
    user_fg: Option<String>,
    assistant_fg: Option<String>,
    tool_name_fg: Option<String>,
    tool_output_fg: Option<String>,
    error_fg: Option<String>,
    thinking_fg: Option<String>,
    status_bar_fg: Option<String>,
    status_bar_bg: Option<String>,
    input_fg: Option<String>,
    code_fg: Option<String>,
    code_bg: Option<String>,
    border_fg: Option<String>,
}

fn parse_color(s: &str) -> Option<Color> {
    match s.to_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        s if s.starts_with('#') && s.len() == 7 => {
            let r = u8::from_str_radix(&s[1..3], 16).ok()?;
            let g = u8::from_str_radix(&s[3..5], 16).ok()?;
            let b = u8::from_str_radix(&s[5..7], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

impl Theme {
    /// Load from a TOML config file, falling back to defaults for missing values.
    pub fn from_config_file(path: &Path) -> Self {
        let mut theme = Self::default();
        let Ok(content) = std::fs::read_to_string(path) else {
            return theme;
        };
        let Ok(cfg) = toml::from_str::<ThemeConfig>(&content) else {
            return theme;
        };

        if let Some(c) = cfg.user_fg.as_deref().and_then(parse_color) {
            theme.user_style = theme.user_style.fg(c);
        }
        if let Some(c) = cfg.assistant_fg.as_deref().and_then(parse_color) {
            theme.assistant_style = theme.assistant_style.fg(c);
        }
        if let Some(c) = cfg.tool_name_fg.as_deref().and_then(parse_color) {
            theme.tool_name_style = theme.tool_name_style.fg(c);
        }
        if let Some(c) = cfg.tool_output_fg.as_deref().and_then(parse_color) {
            theme.tool_output_style = theme.tool_output_style.fg(c);
        }
        if let Some(c) = cfg.error_fg.as_deref().and_then(parse_color) {
            theme.error_style = theme.error_style.fg(c);
        }
        if let Some(c) = cfg.thinking_fg.as_deref().and_then(parse_color) {
            theme.thinking_style = theme.thinking_style.fg(c);
        }
        if let Some(c) = cfg.status_bar_fg.as_deref().and_then(parse_color) {
            theme.status_bar_style = theme.status_bar_style.fg(c);
        }
        if let Some(c) = cfg.status_bar_bg.as_deref().and_then(parse_color) {
            theme.status_bar_style = theme.status_bar_style.bg(c);
        }
        if let Some(c) = cfg.input_fg.as_deref().and_then(parse_color) {
            theme.input_style = theme.input_style.fg(c);
        }
        if let Some(c) = cfg.code_fg.as_deref().and_then(parse_color) {
            theme.code_style = theme.code_style.fg(c);
        }
        if let Some(c) = cfg.code_bg.as_deref().and_then(parse_color) {
            theme.code_style = theme.code_style.bg(c);
        }
        if let Some(c) = cfg.border_fg.as_deref().and_then(parse_color) {
            theme.border_style = theme.border_style.fg(c);
        }

        theme
    }

    /// Try to load from `~/.config/pi/theme.toml`, otherwise use defaults.
    pub fn load() -> Self {
        if let Some(config_dir) = dirs::config_dir() {
            let path = config_dir.join("pi").join("theme.toml");
            if path.exists() {
                return Self::from_config_file(&path);
            }
        }
        Self::default()
    }
}
