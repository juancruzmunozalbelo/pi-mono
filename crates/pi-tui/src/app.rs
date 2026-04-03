//! Main TUI application state and event loop.

use std::io;

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use pi_agent::AgentEvent;
use pi_ai::ChatEvent;
use pi_tools::ToolContent;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::StatefulWidget,
    Frame, Terminal,
};
use tokio::sync::mpsc;

use crate::{
    theme::Theme,
    widgets::{
        input::{InputState, InputWidget},
        messages::{DisplayMessage, MessagesState, MessagesWidget},
        status_bar::{StatusBarWidget, StatusState},
        tool_output::ToolOutput,
    },
};

/// Focus target for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Messages,
}

/// Main application state.
pub struct App {
    pub messages: Vec<DisplayMessage>,
    pub input: InputState,
    pub status: StatusState,
    pub tool_outputs: Vec<ToolOutput>,
    pub messages_state: MessagesState,
    pub streaming_text: String,
    pub thinking_text: String,
    pub is_streaming: bool,
    pub should_quit: bool,
    pub focus: Focus,
    pub model_name: String,
    agent_event_rx: mpsc::UnboundedReceiver<AgentEvent>,
    user_tx: mpsc::UnboundedSender<String>,
    pub theme: Theme,
}

impl App {
    pub fn new(
        agent_event_rx: mpsc::UnboundedReceiver<AgentEvent>,
        user_tx: mpsc::UnboundedSender<String>,
        theme: Theme,
        model_name: String,
    ) -> Self {
        let status = StatusState {
            model_name: model_name.clone(),
            ..Default::default()
        };
        Self {
            messages: Vec::new(),
            input: InputState::default(),
            status,
            tool_outputs: Vec::new(),
            messages_state: MessagesState::default(),
            streaming_text: String::new(),
            thinking_text: String::new(),
            is_streaming: false,
            should_quit: false,
            focus: Focus::Input,
            model_name,
            agent_event_rx,
            user_tx,
            theme,
        }
    }

    /// Run the TUI event loop.
    pub async fn run(mut self) -> anyhow::Result<()> {
        // Set up terminal.
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal).await;

        // Restore terminal.
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        let mut event_stream = EventStream::new();

        loop {
            terminal.draw(|frame| self.render(frame))?;

            tokio::select! {
                Some(Ok(crossterm_event)) = event_stream.next() => {
                    if let Event::Key(key) = crossterm_event {
                        if key.kind == KeyEventKind::Press {
                            self.handle_key(key);
                        }
                    }
                }
                Some(agent_event) = self.agent_event_rx.recv() => {
                    self.handle_agent_event(agent_event);
                }
                else => break,
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Render the full UI.
    fn render(&mut self, frame: &mut Frame) {
        let size = frame.area();

        // Layout: messages (most space), input (up to 5 lines), status bar (1 line).
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),
                Constraint::Length(5),
                Constraint::Length(1),
            ])
            .split(size);

        // Messages widget.
        let messages_widget = MessagesWidget::new(&self.messages, &self.theme);
        StatefulWidget::render(
            messages_widget,
            chunks[0],
            frame.buffer_mut(),
            &mut self.messages_state,
        );

        // Input widget.
        let input_widget = InputWidget::new(&self.input, &self.theme, self.focus == Focus::Input);
        ratatui::widgets::Widget::render(input_widget, chunks[1], frame.buffer_mut());

        // Status bar widget.
        let status_widget = StatusBarWidget::new(&self.status, &self.theme);
        ratatui::widgets::Widget::render(status_widget, chunks[2], frame.buffer_mut());
    }

    /// Handle a keyboard event.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            // Quit / abort.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.is_streaming {
                    // TODO: send abort signal when abort channel is available.
                    tracing::info!("Abort requested");
                } else {
                    self.should_quit = true;
                }
            }

            // Clear conversation view.
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.messages.clear();
                self.messages_state = MessagesState::default();
            }

            // Tab: cycle focus.
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Input => Focus::Messages,
                    Focus::Messages => Focus::Input,
                };
            }

            // Scroll messages.
            KeyCode::PageUp => {
                self.messages_state.scroll_up(10);
            }
            KeyCode::PageDown => {
                self.messages_state.scroll_down(10, u16::MAX);
            }

            // Escape: cancel input / unfocus.
            KeyCode::Esc => {
                if self.focus == Focus::Input && !self.input.is_empty() {
                    self.input = InputState::default();
                } else {
                    self.focus = Focus::Messages;
                }
            }

            // Toggle thinking block visibility (only when not in input mode).
            KeyCode::Char('t') if self.focus == Focus::Messages => {
                // Toggle the most recent thinking block.
                for msg in self.messages.iter_mut().rev() {
                    if let DisplayMessage::Thinking { collapsed, .. } = msg {
                        *collapsed = !*collapsed;
                        break;
                    }
                }
            }

            // Enter: handled in input mode.
            KeyCode::Enter if self.focus == Focus::Input => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Enter: submit.
                    self.submit_input();
                } else {
                    // Plain Enter: insert newline.
                    self.input.newline();
                }
            }

            // Backspace.
            KeyCode::Backspace if self.focus == Focus::Input => {
                self.input.backspace();
            }

            // Delete.
            KeyCode::Delete if self.focus == Focus::Input => {
                self.input.delete_forward();
            }

            // Arrow keys.
            KeyCode::Left if self.focus == Focus::Input => {
                self.input.move_left();
            }
            KeyCode::Right if self.focus == Focus::Input => {
                self.input.move_right();
            }
            KeyCode::Home if self.focus == Focus::Input => {
                self.input.move_home();
            }
            KeyCode::End if self.focus == Focus::Input => {
                self.input.move_end();
            }

            // Up/Down in messages mode = scroll.
            KeyCode::Up if self.focus == Focus::Messages => {
                self.messages_state.scroll_up(3);
            }
            KeyCode::Down if self.focus == Focus::Messages => {
                self.messages_state.scroll_down(3, u16::MAX);
            }

            // Regular character input.
            KeyCode::Char(c) if self.focus == Focus::Input => {
                self.input.insert_char(c);
            }

            _ => {}
        }
    }

    /// Submit the current input text to the agent.
    fn submit_input(&mut self) {
        let text = self.input.take();
        if text.trim().is_empty() {
            return;
        }
        // Push to message list.
        self.messages.push(DisplayMessage::User(text.clone()));
        // Send to agent.
        let _ = self.user_tx.send(text);
    }

    /// Handle an event from the agent.
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStart => {
                tracing::debug!("Agent started");
                self.status.is_streaming = false;
            }

            AgentEvent::TurnStart => {
                self.is_streaming = true;
                self.status.is_streaming = true;
                self.streaming_text.clear();
                self.thinking_text.clear();
            }

            AgentEvent::MessageStart { message: _ } => {
                // A new assistant message began streaming.
            }

            AgentEvent::MessageUpdate { event } => {
                self.handle_chat_event(event);
            }

            AgentEvent::MessageEnd { message } => {
                // Finalize the streaming message.
                self.finalize_streaming_message();

                // Update token counts from the message.
                if let pi_ai::Message::Assistant {
                    usage: Some(usage), ..
                } = &message
                {
                    self.status.input_tokens += usage.input;
                    self.status.output_tokens += usage.output;
                }
            }

            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
            } => {
                self.messages.push(DisplayMessage::ToolCall {
                    name: tool_name.clone(),
                    id: tool_call_id.clone(),
                });
                // Initialize a placeholder tool output.
                self.tool_outputs.push(ToolOutput::new(
                    tool_call_id,
                    tool_name,
                    "(running…)",
                    false,
                ));
            }

            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
            } => {
                if let Some(tool) = self
                    .tool_outputs
                    .iter_mut()
                    .rev()
                    .find(|t| t.tool_call_id == tool_call_id)
                {
                    tool.output = extract_tool_text(&partial_result);
                    tool.is_error = partial_result.is_error;
                }
            }

            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
            } => {
                let output_text = extract_tool_text(&result);
                let is_error = result.is_error;

                // Update the live tool output.
                if let Some(tool) = self
                    .tool_outputs
                    .iter_mut()
                    .rev()
                    .find(|t| t.tool_call_id == tool_call_id)
                {
                    tool.output = output_text.clone();
                    tool.is_error = is_error;
                }

                // Also push as a ToolResult display message.
                self.messages.push(DisplayMessage::ToolResult {
                    name: tool_name,
                    output: output_text,
                    is_error,
                    collapsed: true,
                });
            }

            AgentEvent::TurnEnd { error } => {
                self.is_streaming = false;
                self.status.is_streaming = false;
                if let Some(err) = error {
                    self.messages.push(DisplayMessage::Error(err));
                }
            }

            AgentEvent::AgentEnd { stop_reason: _ } => {
                self.is_streaming = false;
                self.status.is_streaming = false;
                self.finalize_streaming_message();
            }
        }
    }

    /// Handle a streaming ChatEvent.
    fn handle_chat_event(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::TextDelta { text } => {
                self.streaming_text.push_str(&text);
                // Update or push the streaming assistant message.
                self.update_streaming_display();
            }

            ChatEvent::ThinkingDelta { text } => {
                self.thinking_text.push_str(&text);
                self.update_thinking_display();
            }

            ChatEvent::ToolCallStart { id, name } => {
                // Tool call starts are also sent via ToolExecutionStart agent events,
                // but we note it here for completeness.
                tracing::debug!("Tool call start: {} ({})", name, id);
            }

            ChatEvent::Done { usage, .. } => {
                self.status.input_tokens += usage.input;
                self.status.output_tokens += usage.output;
            }

            ChatEvent::Error { message } => {
                self.messages.push(DisplayMessage::Error(message));
            }

            _ => {}
        }
    }

    /// Update the streaming assistant display message.
    fn update_streaming_display(&mut self) {
        let text = self.streaming_text.clone();
        // Find and update the last StreamingAssistant message, or push a new one.
        if let Some(DisplayMessage::StreamingAssistant(t)) = self.messages.last_mut() {
            *t = text;
        } else {
            self.messages.push(DisplayMessage::StreamingAssistant(text));
        }
    }

    /// Update the thinking display block.
    fn update_thinking_display(&mut self) {
        let text = self.thinking_text.clone();
        // Find and update the last Thinking message, or push a new one.
        if let Some(DisplayMessage::Thinking { text: t, .. }) = self.messages.last_mut() {
            *t = text;
        } else {
            self.messages.push(DisplayMessage::Thinking {
                text,
                collapsed: true,
            });
        }
    }

    /// Convert the current StreamingAssistant message to a final Assistant message.
    fn finalize_streaming_message(&mut self) {
        if let Some(DisplayMessage::StreamingAssistant(text)) = self.messages.last().cloned() {
            *self.messages.last_mut().unwrap() = DisplayMessage::Assistant(text);
        }
        self.streaming_text.clear();
    }
}

/// Extract text content from a ToolResult.
fn extract_tool_text(result: &pi_tools::ToolResult) -> String {
    result
        .content
        .iter()
        .map(|c| match c {
            ToolContent::Text { text } => text.as_str(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
