use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use pi_agent::{Agent, AgentConfig, AgentEvent, AgentState, ToolExecutionMode};
use pi_ai::{ApiType, ChatEvent, Model, ModelCost, StopReason};

mod cli;
mod config;
mod session;

use cli::{Cli, Commands, Mode};
use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let config = config::load_config();

    if let Some(Commands::Sessions) = &cli.command {
        return list_sessions_command();
    }

    if cli.prompt.is_some() || matches!(cli.mode, Mode::Print) {
        let prompt = cli.prompt.clone().unwrap_or_else(read_stdin);
        return run_print(&cli, &config, prompt).await;
    }

    run_interactive(&cli, &config).await
}

// ─── Sessions subcommand ──────────────────────────────────────────────────────

fn list_sessions_command() -> Result<()> {
    let sessions = session::list_sessions()?;
    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    let header = format!("{:<36}  {:<20}  {:>8}  LAST MODIFIED", "ID", "MODEL", "MESSAGES");
    println!("{header}");
    println!("{}", "-".repeat(80));
    for s in sessions {
        println!(
            "{:<36}  {:<20}  {:>8}  {}",
            s.id,
            s.model,
            s.messages.len(),
            s.updated_at
        );
    }
    Ok(())
}

// ─── Provider + tools setup ──────────────────────────────────────────────────

fn setup_provider(
    cli: &Cli,
    config: &Config,
) -> Result<(Box<dyn pi_ai::LlmProvider>, Model)> {
    // Resolve provider name; CLI --model may contain "provider:model_id"
    let (provider_name, model_id_from_cli) = match cli.model.as_deref() {
        Some(m) if m.contains(':') => {
            let mut parts = m.splitn(2, ':');
            let prov = parts.next().unwrap_or("github-copilot");
            let mid = parts.next().unwrap_or("gpt-4o");
            (prov.to_string(), Some(mid.to_string()))
        }
        Some(m) => (m.to_string(), None),
        None => (
            config
                .provider
                .clone()
                .unwrap_or_else(|| "github-copilot".to_string()),
            None,
        ),
    };

    let api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var("PI_API_KEY").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No API key found. Set PI_API_KEY or GITHUB_TOKEN, \
                 or add `api_key = \"...\"` to ~/.pi/config.toml"
            )
        })?;

    let provider = pi_ai::get_provider(&provider_name, api_key)
        .map_err(|e| anyhow::anyhow!("Provider error: {e}"))?;

    let model_id = model_id_from_cli
        .or_else(|| config.model.clone())
        .unwrap_or_else(|| "gpt-4o".to_string());

    let model = Model {
        id: model_id,
        name: "Default".to_string(),
        provider: provider_name,
        api: ApiType::OpenaiCompletions,
        base_url: String::new(),
        reasoning: false,
        input_types: vec!["text".to_string()],
        cost: ModelCost::default(),
        context_window: 128000,
        max_tokens: 16384,
        headers: Default::default(),
    };

    Ok((provider, model))
}

fn create_tools() -> Vec<Arc<dyn pi_tools::Tool>> {
    vec![
        Arc::new(pi_tools::ReadFileTool),
        Arc::new(pi_tools::WriteFileTool),
        Arc::new(pi_tools::EditFileTool),
        Arc::new(pi_tools::BashTool),
        Arc::new(pi_tools::GrepTool),
        Arc::new(pi_tools::FindTool),
        Arc::new(pi_tools::LsTool),
    ]
}

fn build_agent(cli: &Cli, config: &Config) -> Result<Agent> {
    let (provider, model) = setup_provider(cli, config)?;
    let tools = create_tools();

    let agent_config = AgentConfig {
        provider: Arc::from(provider),
        tools,
        tool_execution_mode: ToolExecutionMode::Sequential,
        hooks: pi_agent::Hooks::default(),
    };

    let system_prompt = config.system_prompt.clone();

    let thinking_level = config.thinking_level.as_deref().and_then(|l| match l {
        "minimal" => Some(pi_ai::ThinkingLevel::Minimal),
        "low" => Some(pi_ai::ThinkingLevel::Low),
        "medium" => Some(pi_ai::ThinkingLevel::Medium),
        "high" => Some(pi_ai::ThinkingLevel::High),
        "xhigh" => Some(pi_ai::ThinkingLevel::Xhigh),
        _ => None,
    });

    let agent_state = AgentState {
        messages: vec![],
        model,
        system_prompt,
        thinking_level,
        is_streaming: false,
        error_message: None,
    };

    Ok(Agent::new(agent_config, agent_state))
}

// ─── Print mode ───────────────────────────────────────────────────────────────

async fn run_print(cli: &Cli, config: &Config, prompt: String) -> Result<()> {
    let mut agent = build_agent(cli, config)?;
    let mut event_rx = agent
        .take_event_receiver()
        .expect("event receiver available");

    // Run agent in background
    let agent_handle = tokio::spawn(async move { agent.prompt(prompt).await });

    // Consume events, print text to stdout
    while let Some(event) = event_rx.recv().await {
        match event {
            AgentEvent::MessageUpdate {
                event: ChatEvent::TextDelta { text },
            } => {
                use std::io::Write;
                print!("{text}");
                std::io::stdout().flush()?;
            }
            AgentEvent::AgentEnd { stop_reason } => {
                if stop_reason == StopReason::Error {
                    eprintln!("\nAgent ended with error.");
                    std::process::exit(1);
                }
                break;
            }
            _ => {}
        }
    }

    agent_handle.await??;
    println!();
    Ok(())
}

// ─── Interactive mode ─────────────────────────────────────────────────────────

async fn run_interactive(cli: &Cli, config: &Config) -> Result<()> {
    let (provider, model) = setup_provider(cli, config)?;
    let provider: Arc<dyn pi_ai::LlmProvider> = Arc::from(provider);
    let system_prompt = config.system_prompt.clone();
    let thinking_level = config.thinking_level.as_deref().and_then(|l| match l {
        "minimal" => Some(pi_ai::ThinkingLevel::Minimal),
        "low" => Some(pi_ai::ThinkingLevel::Low),
        "medium" => Some(pi_ai::ThinkingLevel::Medium),
        "high" => Some(pi_ai::ThinkingLevel::High),
        "xhigh" => Some(pi_ai::ThinkingLevel::Xhigh),
        _ => None,
    });

    let agent_config = AgentConfig {
        provider: Arc::clone(&provider),
        tools: create_tools(),
        tool_execution_mode: ToolExecutionMode::Sequential,
        hooks: pi_agent::Hooks::default(),
    };
    let agent_state = AgentState {
        messages: vec![],
        model,
        system_prompt,
        thinking_level,
        is_streaming: false,
        error_message: None,
    };
    let agent = Agent::new(agent_config, agent_state);

    // pi-tui is currently a stub (lib.rs has no public API).
    // Fall back to a simple REPL until the TUI is wired up.
    run_repl(agent, provider).await
}

async fn run_repl(mut agent: Agent, provider: Arc<dyn pi_ai::LlmProvider>) -> Result<()> {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let stdout = io::stdout();

    // Re-take the event receiver so we can print streaming output.
    let mut event_rx = agent
        .take_event_receiver()
        .expect("event receiver available");

    println!("Pi agent — type your message, /model <id> to switch model, or Ctrl-C to quit.");

    // Track the current model id for /model switching.
    // We rebuild the agent when the model changes so the new model id is used.
    let mut pending_model: Option<String> = None;

    loop {
        {
            let mut out = stdout.lock();
            write!(out, "> ")?;
            out.flush()?;
        }

        let mut line = String::new();
        let bytes_read = stdin.lock().read_line(&mut line)?;
        if bytes_read == 0 {
            // EOF
            break;
        }

        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }

        // Handle /model slash command (task 8.7)
        if let Some(new_model_id) = input.strip_prefix("/model ") {
            let new_model_id = new_model_id.trim().to_string();
            if new_model_id.is_empty() {
                println!("Usage: /model <model-id>");
                continue;
            }
            pending_model = Some(new_model_id.clone());
            println!("Model switched to: {new_model_id}");
            continue;
        }

        // If a /model switch was requested, rebuild the agent before the next prompt.
        if let Some(new_id) = pending_model.take() {
            let current_state = agent.state().await;
            let mut new_model = current_state.model.clone();
            new_model.id = new_id;
            let messages = current_state.messages.clone();
            let system_prompt = current_state.system_prompt.clone();
            let thinking_level = current_state.thinking_level;
            drop(current_state);

            let tools = create_tools();
            let new_config = AgentConfig {
                provider: Arc::clone(&provider),
                tools,
                tool_execution_mode: ToolExecutionMode::Sequential,
                hooks: pi_agent::Hooks::default(),
            };
            let new_state = AgentState {
                messages,
                model: new_model,
                system_prompt,
                thinking_level,
                is_streaming: false,
                error_message: None,
            };
            agent = Agent::new(new_config, new_state);
            event_rx = agent.take_event_receiver().expect("event receiver available");
        }

        // Run prompt and drain events concurrently.
        let prompt_input = input.clone();
        let prompt_fut = agent.prompt(prompt_input);
        let drain_fut = drain_events_until_end(&mut event_rx);

        let (prompt_result, _) = tokio::join!(prompt_fut, drain_fut);

        if let Err(e) = prompt_result {
            eprintln!("Error: {e}");
        }

        println!();
    }

    Ok(())
}

/// Drain events from the receiver, printing text deltas until AgentEnd.
async fn drain_events_until_end(
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
) {
    use std::io::Write;
    loop {
        match event_rx.recv().await {
            Some(AgentEvent::MessageUpdate {
                event: ChatEvent::TextDelta { text },
            }) => {
                print!("{text}");
                let _ = std::io::stdout().flush();
            }
            Some(AgentEvent::AgentEnd { stop_reason }) => {
                if stop_reason == StopReason::Error {
                    eprintln!("\nAgent ended with error.");
                }
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
}

// ─── Stdin helper ─────────────────────────────────────────────────────────────

fn read_stdin() -> String {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap_or(0);
    buf.trim().to_string()
}
