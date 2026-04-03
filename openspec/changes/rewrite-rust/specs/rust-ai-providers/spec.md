## ADDED Requirements

### Requirement: Provider Trait Abstraction
The `rust-ai-providers` crate SHALL define a `Provider` async trait that abstracts over all supported LLM backends. Every concrete provider MUST implement this trait to be usable by the agent runtime.

#### Scenario: Requesting a completion via the provider trait
- **WHEN** the agent runtime calls `provider.stream(model, context, options)`
- **THEN** it receives a streaming response channel without knowing the underlying API format

#### Scenario: Swapping providers at runtime
- **WHEN** a different provider implementation is passed to the agent
- **THEN** the agent runtime compiles and runs without modification

---

### Requirement: OpenAI Completions Format Support
The crate SHALL implement an `OpenAICompletionsProvider` that communicates with the GitHub Copilot endpoint using the OpenAI Chat Completions API format (`POST /chat/completions`).

#### Scenario: Sending a user message
- **WHEN** a conversation context with a user message is submitted
- **THEN** the provider serializes it as `{ "role": "user", "content": "..." }` in the `messages` array and sends it to the configured base URL

#### Scenario: Receiving a streamed response
- **WHEN** the provider receives `data: {"choices":[{"delta":{"content":"…"}}]}` SSE lines
- **THEN** each delta is emitted as a token event on the response channel in arrival order

---

### Requirement: Anthropic Messages Format Support
The crate SHALL implement an `AnthropicMessagesProvider` that communicates with the MiniMax endpoint using the Anthropic Messages API format (`POST /messages`).

#### Scenario: Sending a multi-turn conversation
- **WHEN** a conversation with alternating user and assistant messages is submitted
- **THEN** the provider serializes the full `messages` array with correct roles and `content` blocks

#### Scenario: Receiving a streamed Anthropic response
- **WHEN** the provider receives `content_block_delta` SSE events
- **THEN** each text delta is forwarded to the response channel without buffering

---

### Requirement: SSE Streaming Parsing
The crate SHALL include a resilient SSE parser that handles chunked HTTP responses from both provider formats. The parser MUST tolerate partial lines, empty lines, and comment lines.

#### Scenario: Receiving a partial SSE chunk
- **WHEN** an HTTP chunk ends mid-line
- **THEN** the parser buffers the partial line and continues on the next chunk without emitting a partial event

#### Scenario: Receiving an SSE comment line
- **WHEN** a line starting with `:` arrives
- **THEN** the parser silently skips the line and emits no event

---

### Requirement: Tool Call Support
Both provider implementations SHALL parse and emit tool call events when the model requests a function invocation. The tool call MUST include the call ID, function name, and JSON arguments.

#### Scenario: OpenAI tool call in streaming response
- **WHEN** the provider receives `tool_calls` deltas in a Completions stream
- **THEN** it accumulates argument fragments and emits a complete `ToolCallEvent` when the call ends

#### Scenario: Anthropic tool use block
- **WHEN** the provider receives a `tool_use` content block in a Messages stream
- **THEN** it emits a `ToolCallEvent` with the block's `id`, `name`, and `input` fields

---

### Requirement: Token Usage Tracking
Each provider implementation SHALL extract token usage from the API response and populate the `Usage` struct with `input`, `output`, `cache_read`, and `cache_write` counts.

#### Scenario: Usage included in a streaming response
- **WHEN** the provider receives a usage summary event before the stream closes
- **THEN** the final `StreamDone` event carries the populated `Usage` struct

#### Scenario: Usage absent from the response
- **WHEN** the API response does not include a usage object
- **THEN** all usage fields default to zero without causing a parse error

---

### Requirement: Provider-Specific Header Injection
The crate SHALL allow each provider to inject arbitrary HTTP headers (e.g., `Authorization`, `Copilot-Integration-Id`) before the request is sent. Headers configured on the `Model` struct MUST be merged with provider defaults, with model-level headers taking precedence.

#### Scenario: Authorization header for GitHub Copilot
- **WHEN** the GitHub Copilot provider sends a request
- **THEN** the `Authorization: Bearer <token>` header is present and the `editor-version` header is set to the configured value

---

### Requirement: Retry on Transient Errors
The crate SHALL retry requests on HTTP 429 and 5xx responses using exponential back-off with jitter. A configurable `max_retry_delay_ms` MUST cap the total wait; if the server's `Retry-After` value exceeds the cap the request MUST fail immediately with a structured error.

#### Scenario: HTTP 429 within retry cap
- **WHEN** the server responds with 429 and `Retry-After: 2`
- **THEN** the client waits approximately 2 seconds and retries the request automatically

#### Scenario: Retry-After exceeds cap
- **WHEN** the server responds with 429 and `Retry-After: 120` and `max_retry_delay_ms` is 60000
- **THEN** the request fails immediately with an error that includes the requested delay value
