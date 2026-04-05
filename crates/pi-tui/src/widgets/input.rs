//! Multiline input editor widget.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::theme::Theme;

/// State for the input editor.
#[derive(Debug, Default, Clone)]
pub struct InputState {
    /// The current text content.
    pub text: String,
    /// Cursor position (byte offset into `text`).
    pub cursor: usize,
}

impl InputState {
    /// Insert a character at the cursor.
    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            // Step back one character.
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(prev);
            self.cursor = prev;
        }
    }

    /// Delete the character at the cursor (delete key).
    pub fn delete_forward(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    /// Move cursor left one character.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    /// Move cursor right one character.
    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            let ch = self.text[self.cursor..].chars().next().unwrap();
            self.cursor += ch.len_utf8();
        }
    }

    /// Move cursor to the beginning of the current line.
    pub fn move_home(&mut self) {
        let before = &self.text[..self.cursor];
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        self.cursor = line_start;
    }

    /// Move cursor to the end of the current line.
    pub fn move_end(&mut self) {
        let after = &self.text[self.cursor..];
        let line_end = after
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.text.len());
        self.cursor = line_end;
    }

    /// Insert a newline.
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// Take the current text and reset the editor.
    pub fn take(&mut self) -> String {
        let text = self.text.clone();
        self.text.clear();
        self.cursor = 0;
        text
    }

    /// Returns true if the text is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// The input editor widget.
pub struct InputWidget<'a> {
    pub state: &'a InputState,
    pub theme: &'a Theme,
    pub focused: bool,
}

impl<'a> InputWidget<'a> {
    pub fn new(state: &'a InputState, theme: &'a Theme, focused: bool) -> Self {
        Self {
            state,
            theme,
            focused,
        }
    }
}

impl<'a> Widget for InputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            self.theme.input_style
        } else {
            self.theme.border_style
        };

        let title = if self.focused {
            " Input (Enter to send, Shift+Enter for newline) "
        } else {
            " Input (Tab to focus) "
        };

        // Build lines with cursor marker.
        let text_with_cursor = build_text_with_cursor(&self.state.text, self.state.cursor);

        let paragraph = Paragraph::new(text_with_cursor)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title),
            )
            .style(self.theme.input_style);

        Widget::render(paragraph, area, buf);
    }
}

/// Build lines with a block cursor character rendered at the cursor position.
fn build_text_with_cursor(text: &str, cursor: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Split text at cursor position.
    let before_cursor = &text[..cursor];
    let after_cursor = &text[cursor..];

    // Determine if cursor is at end of string.
    let (cursor_char, rest_after): (&str, &str) = if after_cursor.is_empty() {
        (" ", "")
    } else {
        let ch = after_cursor.chars().next().unwrap();
        if ch == '\n' {
            (" ", &after_cursor[ch.len_utf8()..])
        } else {
            let len = ch.len_utf8();
            (&after_cursor[..len], &after_cursor[len..])
        }
    };

    // Combine before + [cursor_char] + rest
    let full = format!("{}\x00{}\x01{}", before_cursor, cursor_char, rest_after);

    for line_text in full.split('\n') {
        let mut spans: Vec<Span<'static>> = Vec::new();
        // Split on cursor marker \x00 and \x01.
        if let Some(cur_start) = line_text.find('\x00') {
            let before = line_text[..cur_start].to_string();
            let rest = &line_text[cur_start + 1..];
            if let Some(cur_end) = rest.find('\x01') {
                let cursor_str = rest[..cur_end].to_string();
                let after = rest[cur_end + 1..].to_string();
                if !before.is_empty() {
                    spans.push(Span::raw(before));
                }
                spans.push(Span::styled(
                    cursor_str,
                    ratatui::style::Style::default().add_modifier(Modifier::REVERSED),
                ));
                if !after.is_empty() {
                    spans.push(Span::raw(after));
                }
            } else {
                // No end marker on this line.
                spans.push(Span::raw(line_text.replace(['\x00', '\x01'], "")));
            }
        } else {
            // Remove any stray \x01 markers.
            spans.push(Span::raw(line_text.replace('\x01', "")));
        }
        lines.push(Line::from(spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " ",
            ratatui::style::Style::default().add_modifier(Modifier::REVERSED),
        )]));
    }

    lines
}
