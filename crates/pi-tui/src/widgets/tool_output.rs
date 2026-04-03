//! Collapsible tool output panel widget.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::theme::Theme;

/// A single tool output entry.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
    pub collapsed: bool,
}

impl ToolOutput {
    pub fn new(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            output: output.into(),
            is_error,
            collapsed: true,
        }
    }

    /// Toggle collapsed/expanded state.
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }
}

/// Widget for a single tool output panel.
pub struct ToolOutputWidget<'a> {
    pub tool: &'a ToolOutput,
    pub theme: &'a Theme,
    pub focused: bool,
}

impl<'a> ToolOutputWidget<'a> {
    pub fn new(tool: &'a ToolOutput, theme: &'a Theme, focused: bool) -> Self {
        Self {
            tool,
            theme,
            focused,
        }
    }
}

impl<'a> Widget for ToolOutputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let output_style = if self.tool.is_error {
            self.theme.error_style
        } else {
            self.theme.tool_output_style
        };

        let border_style = if self.focused {
            self.theme.tool_name_style
        } else {
            self.theme.border_style
        };

        if self.tool.collapsed {
            let first_line = self.tool.output.lines().next().unwrap_or("(empty)");
            let truncated = if first_line.len() > 60 {
                format!("{}...", &first_line[..60])
            } else {
                first_line.to_string()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("[Tool: {}] ▶ ", self.tool.tool_name),
                    self.theme.tool_name_style,
                ),
                Span::styled(truncated, output_style),
            ]);

            let paragraph = Paragraph::new(line);
            Widget::render(paragraph, area, buf);
        } else {
            let mut lines: Vec<Line<'static>> = Vec::new();
            for out_line in self.tool.output.lines() {
                lines.push(Line::from(vec![Span::styled(
                    out_line.to_string(),
                    output_style,
                )]));
            }

            let title = format!(" Tool: {} ▼ ", self.tool.tool_name);
            let paragraph = Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(title),
                );

            Widget::render(paragraph, area, buf);
        }
    }
}
