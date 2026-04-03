## ADDED Requirements

### Requirement: Argument Parsing
The `pi` binary SHALL use `clap` to parse CLI arguments. The following flags MUST be supported: `--model <id>` (override the default model), `--session <id>` (resume or name a session), and `--mode <interactive|print>` (select run mode).

#### Scenario: Launching with a model override
- **WHEN** the user runs `pi --model github-copilot/gpt-4o`
- **THEN** the agent starts with that model ID and the status bar reflects the override

#### Scenario: Invalid mode value
- **WHEN** the user runs `pi --mode batch`
- **THEN** `clap` prints a usage error listing valid values and exits with code 2

---

### Requirement: Session Create and Resume
The CLI SHALL persist sessions to `~/.pi/sessions/<id>.json`. Running with `--session <id>` MUST resume an existing session by loading its transcript; if no session with that ID exists a new one MUST be created.

#### Scenario: Resuming an existing session
- **WHEN** `pi --session abc123` is run and `~/.pi/sessions/abc123.json` exists
- **THEN** the agent is initialized with the stored transcript and the TUI shows the prior conversation

#### Scenario: Creating a new named session
- **WHEN** `pi --session new-feature` is run and no matching file exists
- **THEN** a new empty transcript is created and saved to `~/.pi/sessions/new-feature.json` on first message

---

### Requirement: Session Listing
The CLI SHALL support a `pi sessions` subcommand that lists all saved sessions ordered by last-modified time, showing session ID, model, and message count.

#### Scenario: Listing sessions
- **WHEN** the user runs `pi sessions`
- **THEN** each session appears on its own line as `<id>  <model>  <message-count> messages  <last-modified>`

#### Scenario: No sessions exist
- **WHEN** `~/.pi/sessions/` is empty or does not exist
- **THEN** the command prints "No sessions found." and exits with code 0

---

### Requirement: Config Loading
The CLI SHALL load configuration from `~/.pi/config.toml` at startup. Missing config file MUST be treated as an empty config using defaults. Unknown keys in the config MUST produce a warning but MUST NOT prevent startup.

#### Scenario: Config file present with default model
- **WHEN** `~/.pi/config.toml` contains `default_model = "minimax/MiniMax-Text-01"`
- **THEN** the agent uses that model unless overridden by `--model`

#### Scenario: Config file absent
- **WHEN** `~/.pi/config.toml` does not exist
- **THEN** the CLI starts normally using built-in defaults without any error

---

### Requirement: Interactive Mode
In `interactive` mode (the default) the CLI SHALL launch the `rust-tui` terminal UI, accept user input, and stream agent responses in the TUI viewport. The process MUST remain alive until the user explicitly quits.

#### Scenario: Default startup
- **WHEN** `pi` is run without `--mode`
- **THEN** the TUI is rendered and the process waits for user input

---

### Requirement: Print Mode
In `print` mode the CLI SHALL read a prompt from stdin (or `--prompt`), run the agent to completion, write the final assistant message to stdout as plain text, and exit. No TUI MUST be rendered.

#### Scenario: Print mode with inline prompt
- **WHEN** `pi --mode print --prompt "summarise this file" < file.txt` is run
- **THEN** the assistant response is written to stdout with a trailing newline and the process exits with code 0

#### Scenario: Print mode with non-zero agent error
- **WHEN** the agent returns `stopReason: "error"` in print mode
- **THEN** the error message is written to stderr and the process exits with code 1

---

### Requirement: Model Switching Mid-Session
The CLI SHALL support a `/model <id>` slash command in interactive mode that switches the active model for subsequent turns without ending the session.

#### Scenario: Switching model mid-session
- **WHEN** the user types `/model minimax/MiniMax-Text-01` and submits
- **THEN** `agent.state.model` is updated to the new model, the status bar reflects the change, and the next LLM call uses the new model
