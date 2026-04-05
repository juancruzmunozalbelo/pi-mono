//! Integration test: verify that assistant text renders with visible foreground colors.

use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::Color,
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
    Terminal,
};

#[test]
fn assistant_text_has_visible_foreground() {
    // Simulate what the messages widget does
    let theme_fg = Color::Rgb(220, 220, 220);
    let style = ratatui::style::Style::default().fg(theme_fg);

    let lines = vec![
        Line::from(vec![Span::styled("Hello world", style)]),
        Line::from(vec![Span::styled("• item one", style)]),
    ];

    let paragraph = Paragraph::new(lines.clone())
        .style(style)
        .wrap(Wrap { trim: false });

    let area = Rect::new(0, 0, 40, 5);
    let mut buf = Buffer::empty(area);
    paragraph.render(area, &mut buf);

    // Check first cell of "Hello world"
    let cell = buf.cell((0, 0)).unwrap();
    println!(
        "Cell (0,0): char='{}' fg={:?} bg={:?}",
        cell.symbol(),
        cell.fg,
        cell.bg
    );
    assert_ne!(cell.fg, Color::Reset, "fg should not be Reset");
    assert_ne!(cell.fg, Color::Black, "fg should not be Black");

    // Check bullet line
    let cell2 = buf.cell((0, 1)).unwrap();
    println!(
        "Cell (0,1): char='{}' fg={:?} bg={:?}",
        cell2.symbol(),
        cell2.fg,
        cell2.bg
    );
    assert_ne!(cell2.fg, Color::Reset, "bullet fg should not be Reset");
    assert_ne!(cell2.fg, Color::Black, "bullet fg should not be Black");

    // Now test with Style::default() (the broken case)
    let default_lines = vec![Line::from(vec![Span::raw("Default style text")])];
    let default_para = Paragraph::new(default_lines);
    let mut buf2 = Buffer::empty(area);
    default_para.render(area, &mut buf2);

    let cell3 = buf2.cell((0, 0)).unwrap();
    println!(
        "Default cell (0,0): char='{}' fg={:?} bg={:?}",
        cell3.symbol(),
        cell3.fg,
        cell3.bg
    );
    // This will show what Color unstyled text gets — likely Color::Reset
    println!("This is the 'broken' case: fg={:?}", cell3.fg);
}

#[test]
fn markdown_renderer_sets_foreground() {
    use pi_tui::theme::Theme;
    use pi_tui::widgets::markdown::MarkdownRenderer;

    let theme = Theme::default();
    let renderer = MarkdownRenderer::new(&theme);

    let text = "Hello world\n\n- item one\n- item two\n\nParagraph text";
    let lines = renderer.render(text, 80);

    println!("=== Rendered lines ===");
    for (i, line) in lines.iter().enumerate() {
        for span in &line.spans {
            println!(
                "Line {i}: content='{}' fg={:?} bg={:?} modifiers={:?}",
                span.content, span.style.fg, span.style.bg, span.style.add_modifier
            );
        }
    }

    // Check that NO span has fg=None (which would inherit terminal default)
    for (i, line) in lines.iter().enumerate() {
        for span in &line.spans {
            if span.content.trim().is_empty() {
                continue; // skip whitespace-only spans
            }
            assert!(
                span.style.fg.is_some(),
                "Line {i} span '{}' has fg=None — will be invisible on dark terminals!",
                span.content
            );
        }
    }
}
