//! Scrollable message list widget.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, StatefulWidget, Widget, Wrap},
};

use crate::theme::Theme;
use crate::widgets::markdown;

/// A single displayable message in the conversation view.
#[derive(Debug, Clone)]
pub enum DisplayMessage {
    User(String),
    Assistant(String),
    ToolCall {
        name: String,
        id: String,
    },
    ToolResult {
        name: String,
        output: String,
        is_error: bool,
        collapsed: bool,
    },
    Thinking {
        text: String,
        collapsed: bool,
    },
    StreamingAssistant(String),
    Error(String),
}

/// State held by the messages widget (scroll position, focused item).
#[derive(Debug, Default)]
pub struct MessagesState {
    pub scroll_offset: u16,
    pub focused_item: Option<usize>,
}

impl MessagesState {
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: u16, max: u16) {
        self.scroll_offset = (self.scroll_offset + amount).min(max);
    }
}

/// The messages list widget.
pub struct MessagesWidget<'a> {
    pub messages: &'a [DisplayMessage],
    pub theme: &'a Theme,
}

impl<'a> MessagesWidget<'a> {
    pub fn new(messages: &'a [DisplayMessage], theme: &'a Theme) -> Self {
        Self { messages, theme }
    }

    /// Build styled lines for all messages.
    fn build_lines(&self) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();

        for msg in self.messages {
            match msg {
                DisplayMessage::User(text) => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "> ".to_string(),
                            self.theme.user_style.add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(text.clone(), self.theme.user_style),
                    ]));
                    lines.push(Line::default());
                }

                DisplayMessage::Assistant(text) | DisplayMessage::StreamingAssistant(text) => {
                    let renderer = markdown::MarkdownRenderer::new(self.theme);
                    let md_lines = renderer.render(text, 80);
                    for line in md_lines {
                        lines.push(line);
                    }
                    lines.push(Line::default());
                }

                DisplayMessage::ToolCall { name, id: _ } => {
                    lines.push(Line::from(vec![Span::styled(
                        format!("[Tool: {}]", name),
                        self.theme.tool_name_style,
                    )]));
                }

                DisplayMessage::ToolResult {
                    name,
                    output,
                    is_error,
                    collapsed,
                } => {
                    let style = if *is_error {
                        self.theme.error_style
                    } else {
                        self.theme.tool_output_style
                    };

                    if *collapsed {
                        let first_line = output.lines().next().unwrap_or("(empty)");
                        let truncated = if first_line.len() > 60 {
                            format!("{}...", &first_line[..60])
                        } else {
                            first_line.to_string()
                        };
                        lines.push(Line::from(vec![
                            Span::styled(format!("[{}] ▶ ", name), self.theme.tool_name_style),
                            Span::styled(truncated, style),
                        ]));
                    } else {
                        lines.push(Line::from(vec![Span::styled(
                            format!("[{}] ▼", name),
                            self.theme.tool_name_style,
                        )]));
                        for out_line in output.lines() {
                            lines.push(Line::from(vec![Span::styled(
                                format!("  {}", out_line),
                                style,
                            )]));
                        }
                    }
                    lines.push(Line::default());
                }

                DisplayMessage::Thinking { text, collapsed } => {
                    if *collapsed {
                        lines.push(Line::from(vec![Span::styled(
                            format!("[Reasoning: {} chars]", text.len()),
                            self.theme.thinking_style,
                        )]));
                    } else {
                        lines.push(Line::from(vec![Span::styled(
                            "[Reasoning] ▼".to_string(),
                            self.theme.thinking_style.add_modifier(Modifier::BOLD),
                        )]));
                        for think_line in text.lines() {
                            lines.push(Line::from(vec![Span::styled(
                                format!("  {}", think_line),
                                self.theme.thinking_style,
                            )]));
                        }
                    }
                    lines.push(Line::default());
                }

                DisplayMessage::Error(msg) => {
                    lines.push(Line::from(vec![Span::styled(
                        format!("Error: {}", msg),
                        self.theme.error_style,
                    )]));
                    lines.push(Line::default());
                }
            }
        }

        lines
    }
}

impl<'a> StatefulWidget for MessagesWidget<'a> {
    type State = MessagesState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let lines = self.build_lines();
        let total_lines = lines.len() as u16;

        // Clamp scroll to valid range.
        let visible = area.height.saturating_sub(2); // account for borders
        let max_scroll = total_lines.saturating_sub(visible);
        if state.scroll_offset > max_scroll {
            state.scroll_offset = max_scroll;
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style)
                    .title(" Messages "),
            )
            .wrap(Wrap { trim: false })
            .scroll((state.scroll_offset, 0));

        Widget::render(paragraph, area, buf);
    }
}
