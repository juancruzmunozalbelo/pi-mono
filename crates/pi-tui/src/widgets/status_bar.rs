//! Status bar widget — single line at the bottom of the TUI.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::theme::Theme;

/// Data shown in the status bar.
#[derive(Debug, Default, Clone)]
pub struct StatusState {
    pub model_name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub session_id: Option<String>,
    pub is_streaming: bool,
}

/// The status bar widget.
pub struct StatusBarWidget<'a> {
    pub state: &'a StatusState,
    pub theme: &'a Theme,
}

impl<'a> StatusBarWidget<'a> {
    pub fn new(state: &'a StatusState, theme: &'a Theme) -> Self {
        Self { state, theme }
    }
}

impl<'a> Widget for StatusBarWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Left: model name + streaming indicator.
        let streaming = if self.state.is_streaming { " ●" } else { "" };
        let left = format!(" {}{}", self.state.model_name, streaming);

        // Center: token counts.
        let center = format!("↑{} ↓{}", self.state.input_tokens, self.state.output_tokens);

        // Right: session ID.
        let right = match &self.state.session_id {
            Some(id) => format!("{} ", id),
            None => "new session ".to_string(),
        };

        // Build a line that fills the full width (use char count, not byte len).
        let total_width = area.width as usize;
        let left_chars = left.chars().count();
        let center_chars = center.chars().count();
        let right_chars = right.chars().count();
        let center_start = total_width.saturating_sub(right_chars) / 2;
        let left_pad = center_start.saturating_sub(left_chars);
        let right_pad = total_width
            .saturating_sub(left_chars)
            .saturating_sub(left_pad)
            .saturating_sub(center_chars)
            .saturating_sub(right_chars);

        let composed = format!(
            "{}{}{}{}{}",
            left,
            " ".repeat(left_pad),
            center,
            " ".repeat(right_pad),
            right
        );

        // Truncate to area width (char-safe).
        let composed = if composed.chars().count() > total_width {
            composed.chars().take(total_width).collect::<String>()
        } else {
            // Pad to full width.
            format!("{:<width$}", composed, width = total_width)
        };

        let line = Line::from(vec![Span::styled(composed, self.theme.status_bar_style)]);
        let paragraph = Paragraph::new(line).alignment(Alignment::Left);
        Widget::render(paragraph, area, buf);
    }
}
