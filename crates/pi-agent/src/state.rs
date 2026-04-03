//! AgentState and Agent structs.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use pi_ai::{Message, Model, ThinkingLevel};

use crate::event::AgentEvent;
use crate::hooks::Hooks;

// ─── Configuration ────────────────────────────────────────────────────────────

/// Immutable configuration that is fixed at agent creation time.
pub struct AgentConfig {
    pub provider: Arc<dyn pi_ai::LlmProvider>,
    pub tools: Vec<Arc<dyn pi_tools::Tool>>,
    pub tool_execution_mode: ToolExecutionMode,
    pub hooks: Hooks,
}

/// Whether tools within a single turn are executed one-by-one or concurrently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

// ─── Mutable conversation state ───────────────────────────────────────────────

/// Mutable state of the running agent (shared behind an `RwLock`).
pub struct AgentState {
    pub messages: Vec<Message>,
    pub model: Model,
    pub system_prompt: Option<String>,
    pub thinking_level: Option<ThinkingLevel>,
    pub is_streaming: bool,
    pub error_message: Option<String>,
}

// ─── Simple unbounded message queue ──────────────────────────────────────────

/// Thread-safe FIFO queue used for steering / follow-up messages.
#[derive(Default)]
pub struct MessageQueue {
    inner: std::sync::Mutex<VecDeque<Message>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a message to the back of the queue.
    pub fn push(&self, msg: Message) {
        self.inner.lock().unwrap().push_back(msg);
    }

    /// Drain all queued messages and return them in arrival order.
    pub fn drain(&self) -> Vec<Message> {
        self.inner.lock().unwrap().drain(..).collect()
    }
}

// ─── Agent ────────────────────────────────────────────────────────────────────

/// The agent — owns state, config, queues, and the event channel.
pub struct Agent {
    pub(crate) state: Arc<RwLock<AgentState>>,
    pub(crate) config: AgentConfig,
    pub(crate) cancel: CancellationToken,
    pub(crate) event_tx: mpsc::UnboundedSender<AgentEvent>,
    pub(crate) event_rx: Option<mpsc::UnboundedReceiver<AgentEvent>>,
    /// Messages injected mid-run via `steer()`.
    pub(crate) steering_queue: Arc<MessageQueue>,
    /// Messages that trigger another outer loop iteration via `follow_up()`.
    pub(crate) follow_up_queue: Arc<MessageQueue>,
}

impl Agent {
    /// Create a new agent with the given configuration and initial state.
    pub fn new(config: AgentConfig, initial_state: AgentState) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            state: Arc::new(RwLock::new(initial_state)),
            config,
            cancel: CancellationToken::new(),
            event_tx: tx,
            event_rx: Some(rx),
            steering_queue: Arc::new(MessageQueue::new()),
            follow_up_queue: Arc::new(MessageQueue::new()),
        }
    }

    /// Add a user message and run the agent loop.
    pub async fn prompt(&mut self, input: String) -> Result<(), crate::AgentError> {
        {
            let mut state = self.state.write().await;
            state.messages.push(Message::User {
                content: vec![pi_ai::ContentBlock::Text { text: input }],
            });
        }
        crate::agent_loop::run_loop(self).await
    }

    /// Cancel the running agent loop.
    pub fn abort(&self) {
        self.cancel.cancel();
    }

    /// Clear the conversation history.
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        state.messages.clear();
        state.is_streaming = false;
        state.error_message = None;
    }

    /// Acquire a read lock on the agent state.
    pub async fn state(&self) -> tokio::sync::RwLockReadGuard<'_, AgentState> {
        self.state.read().await
    }

    /// Take the event receiver (can only be called once).
    pub fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<AgentEvent>> {
        self.event_rx.take()
    }

    /// Inject a steering message — will be prepended to the next inner-loop turn.
    pub fn steer(&self, msg: Message) {
        self.steering_queue.push(msg);
    }

    /// Inject a follow-up message — will trigger an additional outer-loop iteration.
    pub fn follow_up(&self, msg: Message) {
        self.follow_up_queue.push(msg);
    }
}
