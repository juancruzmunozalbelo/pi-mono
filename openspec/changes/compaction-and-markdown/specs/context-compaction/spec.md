## ADDED Requirements

### Requirement: Token Estimation
SHALL estimate the token count of each message using a chars/4 heuristic applied to all text content in the message (role, text parts, and tool result content combined), with a minimum of 1 token per message regardless of content length.

#### Scenario: Text message token estimation
- **WHEN** a message contains 400 characters of text content
- **THEN** the estimated token count is 100

#### Scenario: Minimum token floor
- **WHEN** a message contains 0 or fewer than 4 characters of text
- **THEN** the estimated token count is at least 1

#### Scenario: Tool result token estimation
- **WHEN** a message contains tool result content with 800 characters
- **THEN** the tool result characters are included in the chars/4 calculation

---

### Requirement: Budget Calculation
SHALL compute the available token budget as `model.context_window` minus the estimated token count of the system prompt minus `model.max_tokens` (reserved for output), yielding the maximum tokens that may be used by the conversation messages.

#### Scenario: Standard budget computation
- **WHEN** the model context window is 128000, the system prompt is estimated at 2000 tokens, and max_tokens is 4096
- **THEN** the available message budget is 121904

#### Scenario: Budget accounts for system prompt size
- **WHEN** the system prompt is large (e.g., 10000 estimated tokens)
- **THEN** the available budget decreases proportionally, leaving less room for conversation messages

---

### Requirement: Threshold Detection
SHALL trigger compaction when the sum of estimated tokens across all current conversation messages exceeds 75% of the available budget.

#### Scenario: Compaction triggered at threshold
- **WHEN** total estimated message tokens exceed 75% of the available budget
- **THEN** compaction is initiated before the messages are forwarded to the provider

#### Scenario: No compaction below threshold
- **WHEN** total estimated message tokens are at or below 75% of the available budget
- **THEN** messages are forwarded to the provider unchanged

---

### Requirement: Message Preservation Window
SHALL keep the last N messages untouched during compaction, where N is configurable and defaults to 10, summarizing only the messages before that trailing window.

#### Scenario: Default preservation window
- **WHEN** compaction is triggered and no custom window size is configured
- **THEN** the last 10 messages are excluded from summarization and preserved verbatim

#### Scenario: Custom preservation window
- **WHEN** the preservation window is configured to 5
- **THEN** only the last 5 messages are preserved, and all earlier messages are candidates for summarization

#### Scenario: Fewer messages than the window
- **WHEN** the total number of messages is less than or equal to the preservation window
- **THEN** no compaction occurs (nothing to summarize)

---

### Requirement: Summarization via LLM
SHALL call the configured LLM provider with a summarization prompt to produce a concise summary of the messages that fall outside the preservation window, capturing the key decisions, information, and outcomes from that portion of the conversation.

#### Scenario: Summarization call is made
- **WHEN** compaction is triggered with messages outside the preservation window
- **THEN** a request is sent to the LLM provider with those messages and a summarization system prompt

#### Scenario: Summary includes tool outcomes
- **WHEN** the messages to be summarized include tool calls with results
- **THEN** the summary includes the tool name and a brief description of the outcome, not the full raw tool output

---

### Requirement: Summary Injection
SHALL replace all compacted messages with a single User-role message whose content is the LLM-produced summary prefixed with `[Previous conversation summary]:`, followed by the preserved trailing messages in their original order.

#### Scenario: Replacement produces a single summary message
- **WHEN** 40 messages are compacted into a summary
- **THEN** the resulting context contains exactly 1 summary User message followed by the preserved trailing messages

#### Scenario: Summary prefix is present
- **WHEN** the summary message is injected
- **THEN** its content begins with the exact string `[Previous conversation summary]:`

---

### Requirement: Recursive Guard
SHALL detect when the context still exceeds the available budget after compaction and, in that case, truncate the summary text to fit within the remaining budget rather than sending an oversized context to the provider.

#### Scenario: Summary fits after compaction
- **WHEN** the summary plus preserved messages fit within the available budget
- **THEN** no truncation is applied

#### Scenario: Summary truncated when still over budget
- **WHEN** even after replacing old messages with the summary the total tokens still exceed the budget
- **THEN** the summary content is truncated so that the total token estimate fits within the budget

---

### Requirement: Hook Integration
SHALL be implemented as a `TransformContextHook` and registered on `AgentConfig.hooks.transform_context` so that it is invoked automatically before every provider call without modifying the core agent loop.

#### Scenario: Hook is registered at startup
- **WHEN** the agent is initialized via `AgentConfig`
- **THEN** the compaction hook is present in `hooks.transform_context`

#### Scenario: Hook receives and returns messages
- **WHEN** the hook is invoked by the agent runtime
- **THEN** it receives the full message list, applies compaction if needed, and returns the (possibly modified) message list

---

### Requirement: Transparency via Tracing
SHALL emit a `tracing::info` log event when compaction occurs, including the message count before compaction, the message count after compaction, and the estimated token savings.

#### Scenario: Log emitted on compaction
- **WHEN** compaction is triggered and completes successfully
- **THEN** a `tracing::info` event is emitted containing before-count, after-count, and estimated token savings

#### Scenario: No log when compaction is skipped
- **WHEN** the context is below the 75% threshold and compaction does not run
- **THEN** no compaction log event is emitted

---

### Requirement: Tool Result Summarization
SHALL include tool names and brief outcomes in the produced summary rather than reproducing full tool output verbatim, keeping the summary concise and within token budget.

#### Scenario: Tool name included in summary
- **WHEN** the messages being summarized include a tool call to `read_file` that returned file contents
- **THEN** the summary references the tool by name and describes the outcome (e.g., "read_file returned contents of foo.rs") without including the raw file contents

#### Scenario: Large tool output is not reproduced
- **WHEN** a tool result contains thousands of tokens of output
- **THEN** the summary does not reproduce the full output, only a brief description of what the tool returned
