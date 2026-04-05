## ADDED Requirements

### Requirement: ralph-loop-create
`/ralph start <name>` SHALL create a loop directory at `.ralph/<name>/` containing a state JSON file and task file before the first iteration begins.

#### Scenario: Start a new loop
- **WHEN** the user runs `/ralph start <name>` with a task description
- **THEN** the system creates `.ralph/<name>/` directory, writes `state.json` with initial fields `{ name, status: "running", iteration: 0, max_iterations: 50, reflection_interval: 5, task_file, created_at, updated_at }`, and writes the task content to the task file

---

### Requirement: ralph-loop-spawn
`/ralph start <name>` SHALL invoke `spawn_agent` for each iteration, passing the task content concatenated with the current iteration number as the prompt.

#### Scenario: Spawn agent per iteration
- **WHEN** a ralph loop is active and an iteration begins
- **THEN** `spawn_agent` is called with a prompt combining the task content and the current iteration number, and the system waits for the sub-agent to return output before evaluating the next step

---

### Requirement: ralph-loop-reflection
Every N iterations (default 5, configurable via `reflection_interval` in state), the system SHALL inject a reflection prompt before spawning the next iteration's sub-agent.

#### Scenario: Reflection checkpoint reached
- **WHEN** the current iteration count is a non-zero multiple of `reflection_interval`
- **THEN** the system prepends a reflection prompt (asking the sub-agent to assess progress and adjust strategy) to the next iteration's task prompt before calling `spawn_agent`

---

### Requirement: ralph-loop-completion
When the sub-agent output contains the marker `<complete>DONE</complete>`, the loop SHALL end immediately and the state SHALL be updated to `status: "completed"`.

#### Scenario: Sub-agent signals completion
- **WHEN** a sub-agent returns output that contains the exact string `<complete>DONE</complete>`
- **THEN** the loop stops iterating, `state.json` is updated with `status: "completed"` and the final `iteration` count, and no further `spawn_agent` calls are made

---

### Requirement: ralph-loop-max-iterations
The loop SHALL stop after reaching `max_iterations` (default 50) and set `status: "max_iterations_reached"` in the state file.

#### Scenario: Loop reaches iteration limit
- **WHEN** the iteration counter equals `max_iterations` and the sub-agent has not signaled completion
- **THEN** the loop stops, `state.json` is updated with `status: "max_iterations_reached"`, and the user is notified that the limit was reached

---

### Requirement: ralph-status
`/ralph status` SHALL display all loops found under `.ralph/` with their name, status, current iteration count, and last-updated timestamp.

#### Scenario: Listing all loops
- **WHEN** the user runs `/ralph status`
- **THEN** the system reads every `state.json` under `.ralph/*/` and prints a formatted table with columns: Name, Status, Iteration, Last Updated — including both active and paused loops

#### Scenario: No loops exist
- **WHEN** the user runs `/ralph status` and `.ralph/` is empty or does not exist
- **THEN** the system prints a message indicating no loops are present

---

### Requirement: ralph-stop
`/ralph stop <name>` SHALL pause an active loop and persist its current state to `.ralph/<name>/state.json` with `status: "paused"` so that it can be resumed later.

#### Scenario: Stopping a running loop
- **WHEN** the user runs `/ralph stop <name>` and the named loop has `status: "running"`
- **THEN** the current iteration count and all other state fields are written to `state.json` with `status: "paused"`, and the loop ceases spawning further agents

---

### Requirement: ralph-resume
`/ralph start <name>` on an existing paused loop SHALL resume from the saved iteration rather than starting from zero.

#### Scenario: Resuming a paused loop
- **WHEN** the user runs `/ralph start <name>` and `.ralph/<name>/state.json` exists with `status: "paused"`
- **THEN** the system loads the saved state, sets `status: "running"`, and continues iterating from the persisted `iteration` value without resetting the task or configuration

---

### Requirement: ralph-archive
`/ralph archive <name>` SHALL move the loop directory from `.ralph/<name>/` to `.ralph/archive/<name>/`.

#### Scenario: Archiving a completed loop
- **WHEN** the user runs `/ralph archive <name>` and the loop exists under `.ralph/<name>/`
- **THEN** the entire directory is moved to `.ralph/archive/<name>/`, making it absent from `/ralph status` output while still accessible for inspection

---

### Requirement: ralph-state-schema
The state file `state.json` MUST conform to the schema `{ name, status, iteration, max_iterations, reflection_interval, task_file, created_at, updated_at }` and SHALL be updated atomically after every iteration.

#### Scenario: State written after each iteration
- **WHEN** an iteration completes (sub-agent returns output)
- **THEN** `state.json` is rewritten with the incremented `iteration` value and a refreshed `updated_at` timestamp before the next iteration begins or the loop terminates
