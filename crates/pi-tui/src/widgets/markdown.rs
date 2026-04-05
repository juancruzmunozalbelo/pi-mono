//! Markdown renderer — converts markdown text to styled ratatui Lines.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme as SyntectTheme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Stateless markdown-to-ratatui renderer.
///
/// Construct once (loads syntect data), then call [`render`] as many times
/// as needed.
pub struct MarkdownRenderer {
    syntax_set: SyntaxSet,
    syntect_theme: SyntectTheme,
    base_style: Style,
    heading_style: Style,
    bold_style: Style,
    italic_style: Style,
    code_style: Style,
    code_block_border: Style,
    quote_style: Style,
    link_style: Style,
}

impl MarkdownRenderer {
    /// Build a renderer from the app [`Theme`].
    pub fn new(theme: &crate::theme::Theme) -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let syntect_theme = theme_set.themes["base16-ocean.dark"].clone();

        Self {
            syntax_set,
            syntect_theme,
            base_style: theme.assistant_style,
            heading_style: theme
                .assistant_style
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            bold_style: theme.assistant_style.add_modifier(Modifier::BOLD),
            italic_style: theme.assistant_style.add_modifier(Modifier::ITALIC),
            code_style: theme.code_style,
            code_block_border: theme.border_style,
            quote_style: theme.assistant_style.add_modifier(Modifier::ITALIC),
            link_style: theme.assistant_style.add_modifier(Modifier::UNDERLINED),
        }
    }

    /// Render markdown text into a list of styled [`Line`]s.
    ///
    /// `width` is the available terminal width (used for word-wrap hints;
    /// ratatui's `Paragraph` widget handles actual reflowing).
    pub fn render(&self, text: &str, _width: u16) -> Vec<Line<'static>> {
        if text.is_empty() {
            return Vec::new();
        }

        let mut output: Vec<Line<'static>> = Vec::new();

        // Inline spans accumulating for the current block-level element.
        let mut current_spans: Vec<Span<'static>> = Vec::new();

        // Style stack for nested inline elements.
        let mut style_stack: Vec<Style> = Vec::new();

        // Whether we are inside a block-quote.
        let mut in_blockquote = false;
        // Set to true when a `│ ` prefix for the quote has already been
        // added to `current_spans` for the current line.
        let mut quote_prefix_emitted = false;

        // List tracking: (ordered, item_counter).
        let mut list_stack: Vec<(bool, u64)> = Vec::new();

        // Whether the *next* Text/Code event is the first inside a list item
        // (so we should emit the bullet/number prefix first).
        let mut list_item_pending = false;

        // Code block state.
        let mut in_code_block = false;
        let mut code_lang: Option<String> = None;
        // Accumulated raw text for the current code block.
        let mut code_buf = String::new();

        for event in Parser::new(text) {
            match event {
                // ── Block / inline opens ─────────────────────────────────────
                Event::Start(Tag::Heading { level, .. }) => {
                    let style = match level {
                        HeadingLevel::H1 | HeadingLevel::H2 => self.heading_style,
                        _ => self.bold_style,
                    };
                    style_stack.push(style);
                }

                Event::Start(Tag::Paragraph) => {
                    // Nothing; text events will populate current_spans.
                }

                Event::Start(Tag::BlockQuote(_)) => {
                    in_blockquote = true;
                    quote_prefix_emitted = false;
                }

                Event::Start(Tag::List(start)) => {
                    let ordered = start.is_some();
                    let counter = start.unwrap_or(1);
                    list_stack.push((ordered, counter));
                }

                Event::Start(Tag::Item) => {
                    list_item_pending = true;
                }

                Event::Start(Tag::CodeBlock(kind)) => {
                    in_code_block = true;
                    code_buf.clear();
                    code_lang = match kind {
                        CodeBlockKind::Fenced(lang) => {
                            let s = lang.to_string();
                            if s.is_empty() {
                                None
                            } else {
                                Some(s)
                            }
                        }
                        CodeBlockKind::Indented => None,
                    };
                    // Opening border.
                    let label = code_lang
                        .as_deref()
                        .map(|l| format!(" {} ", l))
                        .unwrap_or_else(|| " code ".to_string());
                    output.push(Line::from(vec![Span::styled(
                        format!("┌──{}──┐", label),
                        self.code_block_border,
                    )]));
                }

                Event::Start(Tag::Strong) => {
                    style_stack.push(self.bold_style);
                }

                Event::Start(Tag::Emphasis) => {
                    style_stack.push(self.italic_style);
                }

                Event::Start(Tag::Link { .. }) => {
                    style_stack.push(self.link_style);
                }

                // For images, just render the alt text as plain.
                Event::Start(Tag::Image { .. }) => {}

                // ── Text / leaf events ───────────────────────────────────────
                Event::Text(t) => {
                    let s = t.into_string();

                    if in_code_block {
                        // Accumulate raw code; emit on End(CodeBlock).
                        code_buf.push_str(&s);
                        continue;
                    }

                    // Emit list-item prefix before the first text in an item.
                    if list_item_pending {
                        list_item_pending = false;
                        let depth = list_stack.len();
                        let indent = "  ".repeat(depth.saturating_sub(1));
                        let prefix = match list_stack.last() {
                            Some((true, counter)) => format!("{}{}. ", indent, counter),
                            Some((false, _)) => format!("{}• ", indent),
                            None => String::new(),
                        };
                        if !prefix.is_empty() {
                            current_spans.push(Span::styled(prefix, self.bold_style));
                        }
                    }

                    // Emit block-quote bar prefix on the first text of the line.
                    if in_blockquote && !quote_prefix_emitted {
                        current_spans.push(Span::styled("│ ".to_string(), self.code_block_border));
                        quote_prefix_emitted = true;
                    }

                    let style = if in_blockquote {
                        self.quote_style
                    } else {
                        style_stack.last().copied().unwrap_or(self.base_style)
                    };

                    current_spans.push(Span::styled(s, style));
                }

                Event::Code(t) => {
                    // Inline code spans.
                    if list_item_pending {
                        list_item_pending = false;
                        let depth = list_stack.len();
                        let indent = "  ".repeat(depth.saturating_sub(1));
                        let prefix = match list_stack.last() {
                            Some((true, counter)) => format!("{}{}. ", indent, counter),
                            Some((false, _)) => format!("{}• ", indent),
                            None => String::new(),
                        };
                        if !prefix.is_empty() {
                            current_spans.push(Span::styled(prefix, self.bold_style));
                        }
                    }
                    current_spans.push(Span::styled(t.into_string(), self.code_style));
                }

                Event::SoftBreak => {
                    if in_code_block {
                        code_buf.push('\n');
                    } else {
                        // Treat as a space so ratatui can word-wrap naturally.
                        let style = style_stack.last().copied().unwrap_or_default();
                        current_spans.push(Span::styled(" ".to_string(), style));
                    }
                }

                Event::HardBreak => {
                    if in_code_block {
                        code_buf.push('\n');
                    } else {
                        let line = Line::from(std::mem::take(&mut current_spans));
                        output.push(line);
                        quote_prefix_emitted = false;
                    }
                }

                // ── Block / inline ends ──────────────────────────────────────
                Event::End(TagEnd::Heading(_)) => {
                    style_stack.pop();
                    output.push(Line::from(std::mem::take(&mut current_spans)));
                    output.push(Line::default());
                }

                Event::End(TagEnd::Paragraph) => {
                    if !current_spans.is_empty() {
                        output.push(Line::from(std::mem::take(&mut current_spans)));
                    }
                    output.push(Line::default());
                }

                Event::End(TagEnd::Item) => {
                    if !current_spans.is_empty() {
                        output.push(Line::from(std::mem::take(&mut current_spans)));
                    }
                    // Advance ordered counter.
                    if let Some((true, counter)) = list_stack.last_mut() {
                        *counter += 1;
                    }
                    list_item_pending = false;
                }

                Event::End(TagEnd::List(_)) => {
                    list_stack.pop();
                    if list_stack.is_empty() {
                        output.push(Line::default());
                    }
                }

                Event::End(TagEnd::BlockQuote(_)) => {
                    if !current_spans.is_empty() {
                        output.push(Line::from(std::mem::take(&mut current_spans)));
                    }
                    in_blockquote = false;
                    quote_prefix_emitted = false;
                }

                Event::End(TagEnd::CodeBlock) => {
                    // Highlight and emit.
                    let lines = self.highlight_code(&code_buf, code_lang.as_deref());
                    output.extend(lines);
                    // Closing border.
                    output.push(Line::from(vec![Span::styled(
                        "└─────────────────────────────────────────┘".to_string(),
                        self.code_block_border,
                    )]));
                    output.push(Line::default());
                    in_code_block = false;
                    code_lang = None;
                    code_buf.clear();
                }

                Event::End(TagEnd::Strong) | Event::End(TagEnd::Emphasis) => {
                    style_stack.pop();
                }

                Event::End(TagEnd::Link) => {
                    style_stack.pop();
                }

                // Ignored events.
                Event::Html(_)
                | Event::InlineHtml(_)
                | Event::Rule
                | Event::FootnoteReference(_)
                | Event::TaskListMarker(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Start(_)
                | Event::End(_) => {}
            }
        }

        // Flush any leftover spans (e.g. malformed / unclosed elements).
        if !current_spans.is_empty() {
            output.push(Line::from(current_spans));
        }
        // If a code block was never closed, emit what we have.
        if in_code_block && !code_buf.is_empty() {
            let lines = self.highlight_code(&code_buf, code_lang.as_deref());
            output.extend(lines);
            output.push(Line::from(vec![Span::styled(
                "└─────────────────────────────────────────┘".to_string(),
                self.code_block_border,
            )]));
        }

        output
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Syntax-highlight a block of code and return styled ratatui [`Line`]s.
    ///
    /// Falls back to plain-text rendering when the language is unknown.
    fn highlight_code(&self, code: &str, lang: Option<&str>) -> Vec<Line<'static>> {
        let syntax = lang
            .and_then(|l| self.syntax_set.find_syntax_by_token(l))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, &self.syntect_theme);
        let mut output: Vec<Line<'static>> = Vec::new();

        // Ensure the input ends with a newline so `LinesWithEndings` picks up
        // the last line reliably.
        let owned;
        let src: &str = if code.ends_with('\n') {
            code
        } else {
            owned = format!("{}\n", code);
            &owned
        };

        for line in LinesWithEndings::from(src) {
            let ranges = highlighter
                .highlight_line(line, &self.syntax_set)
                .unwrap_or_default();

            let mut spans: Vec<Span<'static>> =
                vec![Span::styled("│ ".to_string(), self.code_block_border)];

            for (style, text) in ranges {
                let content = text.trim_end_matches('\n').to_string();
                if content.is_empty() {
                    continue;
                }
                spans.push(Span::styled(content, syntect_to_ratatui(style)));
            }

            output.push(Line::from(spans));
        }

        output
    }
}

/// Convert a `syntect` highlight style to a ratatui [`Style`].
fn syntect_to_ratatui(s: syntect::highlighting::Style) -> Style {
    Style::default().fg(Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> MarkdownRenderer {
        MarkdownRenderer::new(&crate::theme::Theme::default())
    }

    fn all_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect()
    }

    #[test]
    fn empty_input() {
        assert!(renderer().render("", 80).is_empty());
    }

    #[test]
    fn plain_text() {
        let lines = renderer().render("hello", 80);
        assert!(!lines.is_empty());
        assert!(all_text(&lines).contains("hello"));
    }

    #[test]
    fn bold_text() {
        let lines = renderer().render("**bold**", 80);
        let has_bold = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.style.add_modifier.contains(Modifier::BOLD) && s.content.contains("bold")
            })
        });
        assert!(has_bold, "expected a Bold span containing 'bold'");
    }

    #[test]
    fn italic_text() {
        let lines = renderer().render("*italic*", 80);
        let has_italic = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.style.add_modifier.contains(Modifier::ITALIC) && s.content.contains("italic")
            })
        });
        assert!(has_italic, "expected an Italic span containing 'italic'");
    }

    #[test]
    fn inline_code() {
        let r = renderer();
        let lines = r.render("`code`", 80);
        let expected_fg = r.code_style.fg;
        let has_code = lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.fg == expected_fg && s.content.contains("code"))
        });
        assert!(has_code, "expected inline code span with code_style fg");
    }

    #[test]
    fn heading_h1() {
        let lines = renderer().render("# Title", 80);
        let has_heading = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.style.add_modifier.contains(Modifier::BOLD) && s.content.contains("Title")
            })
        });
        assert!(has_heading, "expected H1 rendered with Bold");
    }

    #[test]
    fn unordered_list() {
        let lines = renderer().render("- alpha\n- beta", 80);
        let text = all_text(&lines);
        assert!(
            text.contains('•') || text.contains('-'),
            "expected bullet prefix"
        );
        assert!(text.contains("alpha"));
        assert!(text.contains("beta"));
    }

    #[test]
    fn ordered_list() {
        let lines = renderer().render("1. first\n2. second", 80);
        let text = all_text(&lines);
        assert!(text.contains("first"));
        assert!(text.contains("second"));
    }

    #[test]
    fn blockquote() {
        let lines = renderer().render("> quoted line", 80);
        let has_bar = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains('│')));
        assert!(has_bar, "expected blockquote '│' prefix");
    }

    #[test]
    fn link() {
        let lines = renderer().render("[click here](https://example.com)", 80);
        let has_underline = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.style.add_modifier.contains(Modifier::UNDERLINED)
                    && s.content.contains("click here")
            })
        });
        assert!(has_underline, "expected link text with Underlined modifier");
    }

    #[test]
    fn code_block_renders() {
        let lines = renderer().render("```rust\nfn main() {}\n```", 80);
        assert!(lines.len() >= 3, "expected border + code line(s) + border");
        let has_rust_label = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("rust")));
        assert!(has_rust_label, "expected opening border with 'rust' label");
    }

    #[test]
    fn code_block_unknown_lang() {
        // Must not panic and must produce output.
        let lines = renderer().render("```brainfuck\n++++\n```", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn malformed_no_panic() {
        // Unclosed bold and mixed backtick must not panic.
        let _ = renderer().render("**unclosed bold and `mixed", 80);
    }
}
