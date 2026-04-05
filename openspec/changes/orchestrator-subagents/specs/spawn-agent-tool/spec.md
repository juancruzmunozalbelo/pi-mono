## ADDED Requirements

### Requirement: Tool Registration
The spawn_agent tool SHALL be registered with the orchestrator agent if and only if a sub-agent provider is configured. If no sub-agent provider is present, the tool SHALL NOT appear in the orchestrator's tool list.

#### Scenario: Sub-agent provider configured
- **WHEN** a `[sub_agent]` section with a valid provider and API key is present in config (or supplied via CLI/env)
- **THEN** the orchestrator's tool list SHALL include `spawn_agent`

#### Scenario: No sub-agent provider configured
- **WHEN** no sub-agent provider is configured in config, CLI flags, or environment variables
- **THEN** the orchestrator's tool list SHALL NOT include `spawn_agent`

---

### Requirement: Task Parameter
The spawn_agent tool SHALL accept a required `task` string parameter that describes what the sub-agent should do.

#### Scenario: Task parameter provided
- **WHEN** the orchestrator invokes `spawn_agent` with a non-empty `task` string
- **THEN** the sub-agent SHALL receive that string as its primary prompt and begin execution

#### Scenario: Task parameter missing
- **WHEN** the orchestrator invokes `spawn_agent` without a `task` parameter
- **THEN** the tool SHALL return an error ToolResult indicating the parameter is required

---

### Requirement: Context Parameter
The spawn_agent tool SHALL accept an optional `context` string parameter containing code snippets, types, or constraints relevant to the task.

#### Scenario: Context provided
- **WHEN** the orchestrator passes a non-empty `context` string alongside `task`
- **THEN** the sub-agent SHALL receive the context as additional input (appended to or alongside the task prompt) before starting execution

#### Scenario: Context omitted
- **WHEN** the orchestrator invokes `spawn_agent` without a `context` parameter
- **THEN** the sub-agent SHALL start with only the `task` prompt and no error SHALL occur

---

### Requirement: Ephemeral Agent
Each invocation of spawn_agent SHALL create a new Agent instance with fresh state. No memory, conversation history, or tool state SHALL be shared between separate invocations.

#### Scenario: Two sequential invocations
- **WHEN** `spawn_agent` is called twice in the same orchestrator session
- **THEN** the second sub-agent SHALL have no knowledge of the first sub-agent's inputs, outputs, or intermediate tool calls

---

### Requirement: Sub-Agent Tool Access
The sub-agent SHALL have access to exactly the 7 basic tools: read, write, edit, bash, grep, find, and ls. The sub-agent SHALL NOT have access to `spawn_agent`, preventing recursive sub-agent creation.

#### Scenario: Sub-agent attempts to use spawn_agent
- **WHEN** a sub-agent's LLM attempts to call `spawn_agent`
- **THEN** the tool SHALL not be present in the sub-agent's tool list and the call SHALL be rejected or produce an unknown-tool error

#### Scenario: Sub-agent uses a basic tool
- **WHEN** a sub-agent invokes read, write, edit, bash, grep, find, or ls during execution
- **THEN** the tool call SHALL succeed normally and its result SHALL be returned to the sub-agent

---

### Requirement: Return Value
The spawn_agent tool SHALL return the sub-agent's final text output as a ToolResult. If the sub-agent completed using only tool calls and produced no final text message, the tool SHALL return a summary of the actions taken instead.

#### Scenario: Sub-agent produces final text
- **WHEN** the sub-agent finishes and its last message contains text
- **THEN** `spawn_agent` SHALL return that text as a successful ToolResult

#### Scenario: Sub-agent produces no final text
- **WHEN** the sub-agent finishes having only called tools and emitted no final text response
- **THEN** `spawn_agent` SHALL return a synthesized summary of the tool actions taken as a successful ToolResult

---

### Requirement: Timeout Enforcement
The spawn_agent tool SHALL enforce a 5-minute (300-second) timeout per invocation. If the sub-agent has not completed within this limit, the tool SHALL terminate the sub-agent and return an error ToolResult.

#### Scenario: Sub-agent completes before timeout
- **WHEN** the sub-agent finishes within 5 minutes
- **THEN** `spawn_agent` SHALL return the result normally with no timeout error

#### Scenario: Sub-agent exceeds timeout
- **WHEN** the sub-agent is still running after 5 minutes
- **THEN** `spawn_agent` SHALL cancel the sub-agent and return an error ToolResult indicating a timeout occurred

---

### Requirement: Cancellation Propagation
The spawn_agent tool SHALL propagate the parent orchestrator's CancellationToken to the sub-agent. When the parent is cancelled, the sub-agent SHALL also be cancelled.

#### Scenario: Parent orchestrator is cancelled mid-invocation
- **WHEN** the orchestrator's CancellationToken is cancelled while a sub-agent is running
- **THEN** the sub-agent SHALL stop execution promptly and `spawn_agent` SHALL return an error ToolResult reflecting the cancellation

---

### Requirement: Error Handling
Provider errors, tool errors, and panics that occur inside the sub-agent SHALL be caught by spawn_agent and returned as error ToolResults to the orchestrator. The orchestrator process SHALL NOT crash due to sub-agent failures.

#### Scenario: Sub-agent provider returns an API error
- **WHEN** the sub-agent's LLM provider returns a non-retryable error during execution
- **THEN** `spawn_agent` SHALL catch the error and return an error ToolResult describing the failure

#### Scenario: Sub-agent tool call panics
- **WHEN** a tool called by the sub-agent panics at runtime
- **THEN** `spawn_agent` SHALL recover the panic and return an error ToolResult; the orchestrator SHALL continue running
