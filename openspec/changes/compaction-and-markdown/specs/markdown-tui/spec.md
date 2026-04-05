## ADDED Requirements

### Requirement: Heading Rendering
SHALL render ATX headings with the following styles: `#` (H1) as Bold text, `##` (H2) as Bold text, `###` (H3) and deeper as plain text with a bold prefix indicator, so headings are visually distinct from body text.

#### Scenario: H1 rendered bold
- **WHEN** an Assistant message contains `# My Heading`
- **THEN** the heading text is rendered with the Bold modifier applied

#### Scenario: H2 rendered bold
- **WHEN** an Assistant message contains `## Section`
- **THEN** the heading text is rendered with the Bold modifier applied

#### Scenario: H3 rendered with bold prefix
- **WHEN** an Assistant message contains `### Subsection`
- **THEN** the heading text is rendered with a bold prefix marker and no additional style modifier

---

### Requirement: Bold and Italic Rendering
SHALL render `**text**` spans with the Bold text modifier and `*text*` spans with the Italic text modifier, allowing both to appear within the same line and to be nested.

#### Scenario: Bold span rendered
- **WHEN** an Assistant message contains `**important**`
- **THEN** the word "important" is rendered with Bold applied

#### Scenario: Italic span rendered
- **WHEN** an Assistant message contains `*emphasis*`
- **THEN** the word "emphasis" is rendered with Italic applied

#### Scenario: Bold and italic coexist
- **WHEN** a line contains both `**bold**` and `*italic*` spans
- **THEN** each span is rendered with its respective modifier independently

---

### Requirement: Inline Code Rendering
SHALL render `` `code` `` spans with a distinct foreground color that differs from the default body text color, making inline code visually identifiable without a bordered block.

#### Scenario: Inline code has distinct color
- **WHEN** an Assistant message contains `` `someFunction()` ``
- **THEN** the text "someFunction()" is rendered with a foreground color different from surrounding prose

#### Scenario: Inline code within prose
- **WHEN** inline code appears mid-sentence alongside regular text
- **THEN** only the backtick-delimited span receives the code color; surrounding text remains unstyled

---

### Requirement: Fenced Code Block Rendering
SHALL render triple-backtick fenced code blocks inside a bordered block widget, applying syntax highlighting when a language identifier is present on the opening fence.

#### Scenario: Code block is bordered
- **WHEN** an Assistant message contains a fenced code block
- **THEN** the block is rendered inside a visible border that visually separates it from surrounding prose

#### Scenario: Syntax highlighting applied for known language
- **WHEN** the opening fence specifies a supported language (e.g., ` ```rust `)
- **THEN** the code is rendered with token-level syntax highlighting colors

#### Scenario: No language falls back to plain rendering
- **WHEN** the opening fence has no language identifier (` ``` `)
- **THEN** the code is rendered without highlighting but still inside the bordered block

---

### Requirement: Syntax Highlighting Languages
SHALL support syntax highlighting for at minimum the following languages within fenced code blocks: rust, python, javascript, typescript, bash, sh, json, toml, yaml, go, html, css.

#### Scenario: Rust highlighted
- **WHEN** a code block is fenced with ` ```rust `
- **THEN** Rust keywords, types, and literals are rendered with distinct colors

#### Scenario: JSON highlighted
- **WHEN** a code block is fenced with ` ```json `
- **THEN** JSON keys, string values, and numeric values are rendered with distinct colors

#### Scenario: Unknown language falls back gracefully
- **WHEN** a code block is fenced with an unrecognized language tag (e.g., ` ```brainfuck `)
- **THEN** the content is rendered as plain monospace text without crashing

---

### Requirement: List Rendering
SHALL render unordered list items (`- item`) with a bullet prefix and ordered list items (`1. item`) with a numeric prefix, both indented consistently relative to surrounding prose.

#### Scenario: Unordered list bullet prefix
- **WHEN** an Assistant message contains `- first item`
- **THEN** the item is rendered with a bullet character prefix and indentation

#### Scenario: Ordered list numeric prefix
- **WHEN** an Assistant message contains `1. first\n2. second`
- **THEN** each item is rendered with its number as a prefix and consistent indentation

#### Scenario: Nested list indentation
- **WHEN** a list contains a nested sub-list
- **THEN** the nested items are indented further than the parent items

---

### Requirement: Blockquote Rendering
SHALL render `> text` blockquotes with a vertical bar character prefix on each line and italic text style applied to the quoted content.

#### Scenario: Vertical bar prefix present
- **WHEN** an Assistant message contains `> a quoted line`
- **THEN** the line is rendered with a `|` (or equivalent vertical bar) prefix

#### Scenario: Italic style applied to quote text
- **WHEN** a blockquote line is rendered
- **THEN** the quoted text carries the Italic modifier

---

### Requirement: Link Rendering
SHALL render `[text](url)` inline links with underline style applied to the link text, making links visually distinct without requiring the URL to be visible in the rendered output.

#### Scenario: Link text is underlined
- **WHEN** an Assistant message contains `[click here](https://example.com)`
- **THEN** the text "click here" is rendered with the Underline modifier

#### Scenario: URL not shown inline
- **WHEN** a markdown link is rendered
- **THEN** the raw URL is not displayed as visible text alongside the link text

---

### Requirement: Word Wrapping
SHALL wrap text at the current terminal width without breaking words mid-word, and SHALL preserve inline style spans (bold, italic, code color) across line breaks so that a span beginning on one line continues with the same style on the next.

#### Scenario: Long line wrapped at terminal width
- **WHEN** a prose paragraph exceeds the terminal column width
- **THEN** the text wraps to a new line at a word boundary without splitting any word

#### Scenario: Style preserved across wrap
- **WHEN** a bold span begins near the end of a line and wraps to the next
- **THEN** the continuation on the next line retains Bold styling

#### Scenario: No mid-word breaks
- **WHEN** a single long word would exceed the line width
- **THEN** the word is placed on its own line rather than split across two lines

---

### Requirement: Fallback for Unrecognized Elements
SHALL render any markdown element not explicitly handled as plain unstyled text and SHALL NOT panic or return an error when encountering malformed or unsupported markdown constructs.

#### Scenario: Unknown element rendered as plain text
- **WHEN** an Assistant message contains a markdown construct not in the supported set (e.g., a definition list or footnote)
- **THEN** the raw text is displayed as-is without any styling

#### Scenario: Malformed markdown does not crash
- **WHEN** an Assistant message contains malformed markdown (e.g., unclosed backtick or unclosed bold span)
- **THEN** the renderer produces output without panicking

---

### Requirement: Integration with Messages Widget
SHALL be used by the messages widget to render all Assistant-role messages, replacing the previous plain-text rendering path, so that markdown formatting is applied to every Assistant message displayed in the TUI.

#### Scenario: Assistant messages use markdown renderer
- **WHEN** an Assistant message is displayed in the messages widget
- **THEN** the markdown renderer is invoked and its output is used for display instead of the plain text path

#### Scenario: User messages unaffected
- **WHEN** a User-role message is displayed in the messages widget
- **THEN** the plain text rendering path continues to be used (markdown renderer is not applied)
