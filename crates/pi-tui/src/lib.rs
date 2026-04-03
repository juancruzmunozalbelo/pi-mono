//! Pi TUI — Terminal user interface for the pi agent.

pub mod app;
pub mod theme;
pub mod widgets;

pub use app::App;
pub use theme::Theme;

use pi_agent::AgentEvent;
use tokio::sync::mpsc;

/// Run the TUI, consuming events from the agent and sending user input back.
///
/// # Parameters
/// - `agent_event_rx`: receives [`AgentEvent`]s from the agent loop.
/// - `user_tx`: sends the user's typed messages back to the agent loop.
/// - `theme`: visual theme (use [`Theme::default()`] or [`Theme::load()`]).
/// - `model_name`: display name for the status bar.
pub async fn run_tui(
    agent_event_rx: mpsc::UnboundedReceiver<AgentEvent>,
    user_tx: mpsc::UnboundedSender<String>,
    theme: Theme,
    model_name: String,
) -> anyhow::Result<()> {
    let app = App::new(agent_event_rx, user_tx, theme, model_name);
    app.run().await
}
