use termimad::crossterm::style::Color::*;
use termimad::MadSkin;

/// Create the default Pi skin for terminal markdown rendering.
pub fn make_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.bold.set_fg(Yellow);
    skin.italic.set_fg(Cyan);
    skin.inline_code.set_fg(Green);
    skin.code_block.set_bg(DarkGrey);
    skin.headers[0].set_fg(Cyan);
    skin.headers[0].add_attr(termimad::crossterm::style::Attribute::Bold);
    skin.headers[1].set_fg(Green);
    skin.headers[1].add_attr(termimad::crossterm::style::Attribute::Bold);
    skin.bullet.set_fg(Yellow);
    skin.quote_mark.set_fg(DarkGrey);
    skin
}

/// Render a complete markdown text block to terminal.
pub fn render_text(skin: &MadSkin, text: &str) {
    // Clean \r characters that some models inject
    let clean = text.replace('\r', "");
    if !clean.trim().is_empty() {
        println!(); // blank line before response
        skin.print_text(&clean);
    }
}

/// Render a tool execution start — inline, subtle.
pub fn render_tool_start(name: &str) {
    println!("\x1b[33m  ▸ {name}\x1b[0m");
}

/// Render a tool execution result.
pub fn render_tool_end(name: &str, result_text: &str, is_error: bool) {
    // Clean \r
    let clean = result_text.replace('\r', "");
    let first_line = clean.lines().next().unwrap_or("");
    let truncated = if first_line.chars().count() > 80 {
        format!("{}...", first_line.chars().take(80).collect::<String>())
    } else {
        first_line.to_string()
    };
    if is_error {
        println!("\x1b[31m  ✗ [{name}] {truncated}\x1b[0m");
    } else {
        println!("\x1b[2m  ✓ [{name}] {truncated}\x1b[0m");
    }
}
