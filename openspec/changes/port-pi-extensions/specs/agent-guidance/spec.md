## ADDED Requirements

### Requirement: guidance-global-lookup
On agent creation, the system SHALL check `~/.pi/guidance/{PROVIDER_NAME}.md` for a guidance file matching the active provider (e.g., `COPILOT.md`, `MINIMAX.md`).

#### Scenario: Global guidance file exists for the provider
- **WHEN** an agent is created with provider `COPILOT` and `~/.pi/guidance/COPILOT.md` exists
- **THEN** the system reads the file contents and prepares them for injection into the system prompt

#### Scenario: No global guidance file exists for the provider
- **WHEN** an agent is created with provider `MINIMAX` and `~/.pi/guidance/MINIMAX.md` does not exist
- **THEN** the system skips global guidance and proceeds to check the project-level override path

---

### Requirement: guidance-project-override
The system SHALL also check `.pi/guidance/{PROVIDER_NAME}.md` in the current working directory, and if found, SHALL use it in place of the global guidance file (project overrides take precedence).

#### Scenario: Project-level guidance file exists
- **WHEN** an agent is created and `.pi/guidance/COPILOT.md` exists in the current working directory
- **THEN** the project-level file is used as the guidance content, overriding any global `~/.pi/guidance/COPILOT.md`

#### Scenario: Only global guidance file exists
- **WHEN** an agent is created and `.pi/guidance/COPILOT.md` does not exist in the current working directory but `~/.pi/guidance/COPILOT.md` does
- **THEN** the global guidance file content is used

---

### Requirement: guidance-system-prompt-injection
The system SHALL prepend the guidance file content to the agent's `system_prompt` when a guidance file is found.

#### Scenario: Guidance content is prepended
- **WHEN** a guidance file is resolved for the current provider
- **THEN** the guidance file text is prepended to the system_prompt before the agent is invoked, so the model receives guidance before any other system instructions

---

### Requirement: guidance-sub-agent-provider
When a sub-agent is spawned via `spawn_agent`, the system SHALL apply the guidance file corresponding to the sub-agent's own provider, not the orchestrator's provider.

#### Scenario: Sub-agent uses a different provider than the orchestrator
- **WHEN** the orchestrator uses provider `COPILOT` but spawns a sub-agent configured with provider `MINIMAX`
- **THEN** the sub-agent's system_prompt is prepended with the content of `MINIMAX.md`, not `COPILOT.md`

---

### Requirement: guidance-tracing-log
The system SHALL emit a `tracing::debug` log entry when a guidance file is successfully loaded and when no guidance file is found for the active provider.

#### Scenario: Guidance file is loaded
- **WHEN** the system resolves and reads a guidance file for the provider
- **THEN** a debug-level log message is emitted indicating the file path that was loaded

#### Scenario: No guidance file is found
- **WHEN** neither the project-level nor the global guidance file exists for the provider
- **THEN** a debug-level log message is emitted indicating that no guidance was found for that provider name
