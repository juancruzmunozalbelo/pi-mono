## ADDED Requirements

### Requirement: Conversation Context Management
The `rust-agent-core` crate SHALL maintain a mutable conversation transcript as an ordered list of `AgentMessage` values (user, assistant, tool result). The transcript MUST be updated atomically before any event is emitted for that message.

#### Scenario: Adding a user message
- **WHEN** `agent.prompt("hello")` is called
- **THEN** the user message is appended to the transcript and a `MessageStart` event is emitted before the provider is contacted

#### Scenario: Resetting the transcript
- **WHEN** `agent.reset()` is called
- **THEN** the messages vec is cleared, all pending tool-call IDs are removed, and the streaming flag is set to false

---

### Requirement: Tool Execution Loop
The agent SHALL implement a tool execution loop that processes all tool calls from an assistant message before issuing the next LLM request. The loop MUST support both `Sequential` and `Parallel` execution modes.

#### Scenario: Sequential tool execution
- **WHEN** `tool_execution` is set to `Sequential` and the assistant requests two tools
- **THEN** the second tool starts only after the first tool has completed and its result has been appended to the transcript

#### Scenario: Parallel tool execution
- **WHEN** `tool_execution` is set to `Parallel` and the assistant requests two tools
- **THEN** both tools execute concurrently using `tokio::join` and results are appended in assistant source order

---

### Requirement: Event Streaming
The agent SHALL emit structured events to all subscribed listeners throughout a run. Events MUST be emitted in the following order: `AgentStart`, zero or more turns each bracketed by `TurnStart`/`TurnEnd`, then `AgentEnd`.

#### Scenario: Normal turn with tool calls
- **WHEN** an assistant message containing one tool call is processed
- **THEN** the listener receives `TurnStart`, `MessageStart`, one or more `MessageUpdate`, `MessageEnd`, `ToolExecutionStart`, `ToolExecutionEnd`, `TurnEnd`, and finally `AgentEnd`

#### Scenario: Subscriber receives partial streaming message
- **WHEN** a token delta arrives from the provider
- **THEN** a `MessageUpdate` event is emitted with the current partial `AssistantMessage` before the turn ends

---

### Requirement: beforeToolCall Hook
The agent SHALL invoke a `before_tool_call` async callback before executing any tool. Returning `BeforeToolCallResult { block: true }` MUST prevent execution and emit an error tool result instead.

#### Scenario: Hook allows execution
- **WHEN** `before_tool_call` returns `None`
- **THEN** the tool executes normally and its result is appended to the transcript

#### Scenario: Hook blocks execution
- **WHEN** `before_tool_call` returns `BeforeToolCallResult { block: true, reason: Some("denied") }`
- **THEN** the tool is not executed, an error tool result with the reason text is emitted, and the loop continues with the remaining tool calls

---

### Requirement: afterToolCall Hook
The agent SHALL invoke an `after_tool_call` async callback after a tool finishes and before its result is emitted. The hook MAY override `content`, `details`, or `is_error` fields of the tool result.

#### Scenario: Hook replaces content
- **WHEN** `after_tool_call` returns `AfterToolCallResult { content: Some(new_content), .. }`
- **THEN** the transcript and emitted `ToolExecutionEnd` event carry `new_content` instead of the original result

#### Scenario: Hook returns None
- **WHEN** `after_tool_call` returns `None`
- **THEN** the original tool result is used unchanged

---

### Requirement: Steer and FollowUp Queues
The agent SHALL expose `steer(message)` and `follow_up(message)` methods that enqueue messages for injection at different points in the run. Steering messages MUST be injected after the current turn's tool calls complete; follow-up messages MUST be injected only when the agent would otherwise stop.

#### Scenario: Steering mid-run
- **WHEN** `agent.steer(msg)` is called while a run is active and the current assistant turn completes
- **THEN** the steering message is prepended to the context before the next LLM call in the same run

#### Scenario: FollowUp after natural stop
- **WHEN** `agent.follow_up(msg)` is called and the agent reaches a `stop` turn with no pending tool calls
- **THEN** the follow-up message is added to the context and the agent issues one additional LLM call

---

### Requirement: transformContext Hook
The agent SHALL invoke an optional `transform_context` async callback before converting messages to the LLM wire format. The hook receives the full `AgentMessage` slice and MUST return a (possibly pruned or augmented) replacement slice.

#### Scenario: Pruning context for token budget
- **WHEN** `transform_context` returns a shorter slice of messages
- **THEN** the LLM request is built from the pruned slice, leaving the full transcript in agent state untouched

---

### Requirement: Abort Support
The agent SHALL expose an `abort()` method that signals the active run's `CancellationToken`. All in-progress tool executions and provider streams MUST be cancelled on abort.

#### Scenario: Aborting an active run
- **WHEN** `agent.abort()` is called during tool execution
- **THEN** the tool receives a cancel signal, the run ends with `stopReason: "aborted"`, and `agent.is_streaming()` returns false after the run settles
