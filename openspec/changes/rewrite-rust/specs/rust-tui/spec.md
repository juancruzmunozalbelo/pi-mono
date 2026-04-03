## ADDED Requirements

### Requirement: Message Display with Markdown Rendering
The TUI SHALL render conversation messages in a scrollable viewport. Assistant messages MUST have Markdown rendered to terminal-compatible styled spans (bold, italic, inline code, fenced code blocks with syntax highlighting).

#### Scenario: Rendering a fenced code block
- **WHEN** an assistant message contains a triple-backtick code block with a language tag
- **THEN** the block is displayed with appropriate syntax highlighting colors and a visible border separating it from prose

#### Scenario: Streaming token display
- **WHEN** a `MessageUpdate` event arrives with a new text delta
- **THEN** the viewport redraws with the appended text within the same frame cycle, without flickering the preceding content

---

### Requirement: Tool Output Display
The TUI SHALL render tool execution results in collapsible panels. Each panel MUST show the tool name and a summary line when collapsed; expanding it MUST reveal the full output. Panels MUST be collapsible by default once the tool call is complete.

#### Scenario: Tool panel collapses after completion
- **WHEN** a `ToolExecutionEnd` event is received
- **THEN** the tool panel transitions to collapsed state showing only the tool name and first line of output

#### Scenario: Expanding a tool panel
- **WHEN** the user presses the configured expand key while a tool panel is focused
- **THEN** the panel expands to show the complete tool output without truncation

---

### Requirement: Multiline Input Editor
The TUI SHALL provide an inline text editor that supports multiline input. The editor MUST handle `Enter` for newline insertion and a configurable submit chord (default `Ctrl+Enter`) for submission.

#### Scenario: Inserting a newline
- **WHEN** the user presses `Enter` in the input editor
- **THEN** a newline character is inserted at the cursor position and the editor height grows to accommodate the new line

#### Scenario: Submitting input
- **WHEN** the user presses the submit chord
- **THEN** the editor content is cleared, the message is dispatched to the agent, and the editor returns to single-line height

---

### Requirement: Status Bar
The TUI SHALL render a fixed status bar at the bottom of the terminal showing the active model name, cumulative token count for the session, and the session ID. All three values MUST update in real time as turns complete.

#### Scenario: Token count updates after a turn
- **WHEN** a `TurnEnd` event is received with updated usage data
- **THEN** the status bar token count increments by the turn's total token count within the same render frame

#### Scenario: Status bar during streaming
- **WHEN** the agent is streaming a response
- **THEN** the status bar displays a visual indicator (e.g., spinner or "…") next to the model name

---

### Requirement: Theming Support
The TUI SHALL support a configurable color theme loaded from `~/.pi/config.toml`. Theme values MUST include foreground, background, accent, and code-block background colors. A built-in default theme MUST be used when no theme is configured.

#### Scenario: Custom theme applied
- **WHEN** `~/.pi/config.toml` contains a `[theme]` section with a `code_bg` color
- **THEN** all code block backgrounds in the conversation view use that color

#### Scenario: Missing theme config
- **WHEN** no `[theme]` section exists in the config file
- **THEN** the TUI renders using the built-in default theme without error

---

### Requirement: Keyboard Shortcuts
The TUI SHALL support the following default keyboard shortcuts: `Ctrl+C` to abort the active run or quit if idle, `Ctrl+L` to clear the conversation view, `PgUp`/`PgDn` to scroll the message viewport, and `Tab` to cycle focus between the input editor and message list.

#### Scenario: Aborting an active run
- **WHEN** `Ctrl+C` is pressed while the agent is streaming
- **THEN** `agent.abort()` is called and the UI transitions to idle state without exiting the process

#### Scenario: Quitting when idle
- **WHEN** `Ctrl+C` is pressed while the agent is idle
- **THEN** the TUI tears down cleanly and the process exits with code 0

---

### Requirement: Thinking / Reasoning Block Display
The TUI SHALL render `thinking` content blocks from assistant messages in a visually distinct collapsed section labeled "Reasoning". The section MUST be expandable by the user.

#### Scenario: Thinking block rendered collapsed by default
- **WHEN** an assistant message contains a `thinking` content block
- **THEN** a collapsed "Reasoning" panel appears above the text content, showing only the label and character count

#### Scenario: Expanding the thinking block
- **WHEN** the user expands the "Reasoning" panel
- **THEN** the full thinking text is displayed with a dimmed or italicized style distinct from the main response text
