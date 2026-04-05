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
    Loading(String),
    Error(String),
}

/// State held by the messages widget (scroll position, focused item).
#[derive(Debug)]
pub struct MessagesState {
    pub scroll_offset: u16,
    pub focused_item: Option<usize>,
    /// When true, auto-scroll to bottom on new content. Disabled when user scrolls up.
    pub auto_scroll: bool,
}

impl Default for MessagesState {
    fn default() -> Self {
        Self {
            scroll_offset: 0,
            focused_item: None,
            auto_scroll: true,
        }
    }
}

impl MessagesState {
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        self.auto_scroll = false; // user scrolled up → disable auto-scroll
    }

    pub fn scroll_down(&mut self, amount: u16, max: u16) {
        self.scroll_offset = (self.scroll_offset + amount).min(max);
        // Re-enable auto-scroll if we're near the bottom
        if self.scroll_offset >= max.saturating_sub(2) {
            self.auto_scroll = true;
        }
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
                    let base = self.theme.assistant_style;
                    for line in md_lines {
                        // Set the LINE style so all spans inherit it
                        let mut styled_line = line.style(base);
                        // Also patch each span that has no fg
                        styled_line.spans = styled_line
                            .spans
                            .into_iter()
                            .map(|mut span| {
                                if span.style.fg.is_none() {
                                    span.style.fg = base.fg;
                                }
                                span
                            })
                            .collect();
                        lines.push(styled_line);
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

                DisplayMessage::Loading(msg) => {
                    // Animated dots based on time
                    let dots = match (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        / 500)
                        % 4
                    {
                        0 => "   ",
                        1 => ".  ",
                        2 => ".. ",
                        _ => "...",
                    };
                    lines.push(Line::from(vec![Span::styled(
                        format!("⏳ {msg}{dots}"),
                        self.theme.thinking_style,
                    )]));
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

        // Auto-scroll: set to a very large value — the Paragraph widget
        // internally clamps scroll to valid range after wrapping.
        if state.auto_scroll {
            state.scroll_offset = u16::MAX;
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
