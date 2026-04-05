## ADDED Requirements

### Requirement: tab-title-on-agent-start
The system SHALL update the terminal tab title to `pi - [dirname]:running...` when an AgentStart event is received, where `[dirname]` is the basename of the current working directory.

#### Scenario: Agent begins a new session
- **WHEN** an AgentStart event fires
- **THEN** the OSC escape sequence `\x1b]0;pi - [dirname]:running...\x07` is written to stdout, updating the terminal tab title

---

### Requirement: git-commit-detection
The system SHALL detect git commit tool calls by matching the regex `\bgit\b.*\bcommit\b` against tool call content and track whether a commit occurred via a `sawCommit` boolean flag.

#### Scenario: Tool call contains a git commit command
- **WHEN** a tool call event is received and its content matches `\bgit\b.*\bcommit\b`
- **THEN** the internal `sawCommit` flag is set to `true` for the current agent session

#### Scenario: Tool call does not contain a git commit command
- **WHEN** a tool call event is received and its content does not match the git commit regex
- **THEN** the `sawCommit` flag remains unchanged

---

### Requirement: tab-title-on-agent-end
The system SHALL update the terminal tab title on AgentEnd based on the session outcome: `:✅` if `sawCommit` is true, `:🚧` if the agent ended without a commit, or `:🛑` if the agent ended with an error or was aborted.

#### Scenario: Agent ends after making a commit
- **WHEN** an AgentEnd event fires and `sawCommit` is `true`
- **THEN** the tab title is updated to `pi - [dirname]:✅`

#### Scenario: Agent ends without making a commit
- **WHEN** an AgentEnd event fires and `sawCommit` is `false` and no error occurred
- **THEN** the tab title is updated to `pi - [dirname]:🚧`

#### Scenario: Agent ends with an error or is aborted
- **WHEN** an AgentEnd event fires with an error condition or the session is aborted
- **THEN** the tab title is updated to `pi - [dirname]:🛑`

---

### Requirement: inactivity-timeout
The system SHALL set the tab title to `pi - [dirname]:🛑` after 180 seconds of inactivity, defined as no AgentStart, AgentEnd, or tool call events being received.

#### Scenario: No events received for 180 seconds
- **WHEN** 180 seconds elapse since the last agent event
- **THEN** the tab title is updated to `pi - [dirname]:🛑` automatically

#### Scenario: Event received before timeout elapses
- **WHEN** an agent event arrives before the 180-second inactivity timer expires
- **THEN** the timer is reset and no `:🛑` title update is emitted

---

### Requirement: osc-escape-and-tty-guard
The system SHALL write tab title updates using the OSC escape sequence `\x1b]0;{title}\x07` and SHALL no-op gracefully when stdout is not a terminal.

#### Scenario: stdout is a TTY
- **WHEN** any tab title update is triggered and stdout is a terminal (isatty returns true)
- **THEN** the OSC sequence `\x1b]0;{title}\x07` is written to stdout

#### Scenario: stdout is not a TTY
- **WHEN** any tab title update is triggered and stdout is not a terminal (e.g., piped to a file or another process)
- **THEN** no OSC sequence is written and no error is returned
