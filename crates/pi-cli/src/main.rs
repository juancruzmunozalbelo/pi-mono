use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use pi_agent::{Agent, AgentConfig, AgentEvent, AgentState, ToolExecutionMode};
use pi_ai::{ApiType, ChatEvent, Model, ModelCost, StopReason};

mod cli;
mod config;
mod guidance;
mod ralph;
mod session;
mod spawn_agent;
mod tab_status;
mod usage;

use cli::{Cli, Commands, Mode};
use config::Config;
use spawn_agent::{minimax_model, SpawnAgentTool, SubAgentConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let config = config::load_config();

    match &cli.command {
        Some(Commands::Sessions) => return list_sessions_command(),
        Some(Commands::Login) => return login_command().await,
        Some(Commands::Usage { period }) => return usage::run_usage(*period),
        None => {}
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

    let header = format!(
        "{:<36}  {:<20}  {:>8}  LAST MODIFIED",
        "ID", "MODEL", "MESSAGES"
    );
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

// ─── Login command ───────────────────────────────────────────────────────────

async fn login_command() -> Result<()> {
    use pi_ai::auth;

    let client = reqwest::Client::new();
    let github_base = "https://github.com";

    println!("Starting GitHub Copilot login...\n");

    // Step 1: Start device flow
    let flow = auth::start_device_flow(&client, github_base)
        .await
        .map_err(|e| anyhow::anyhow!("Device flow failed: {e}"))?;

    println!("Open this URL in your browser:\n");
    println!("  {}\n", flow.verification_uri);
    println!("Enter this code: {}\n", flow.user_code);
    println!("Waiting for authorization...");

    // Step 2: Poll for GitHub access token
    let cancel = tokio_util::sync::CancellationToken::new();
    let github_token = auth::poll_for_token(
        &client,
        github_base,
        &flow.device_code,
        flow.interval,
        flow.expires_in,
        cancel,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Token polling failed: {e}"))?;

    println!("GitHub authorization successful!");

    // Step 3: Exchange for Copilot token (validates Copilot access)
    let copilot_token = auth::exchange_copilot_token(&client, &github_token, None)
        .await
        .map_err(|e| anyhow::anyhow!("Copilot token exchange failed: {e}"))?;

    println!(
        "Copilot access verified! Base URL: {}",
        copilot_token.base_url
    );

    // Step 4: Save credentials
    let creds = auth::StoredCredentials {
        github_token,
        enterprise_domain: None,
    };
    auth::save_credentials(&creds)
        .map_err(|e| anyhow::anyhow!("Failed to save credentials: {e}"))?;

    println!(
        "\nCredentials saved to {}",
        auth::credentials_path().display()
    );
    println!("You can now use: pi -p \"hello\"");

    Ok(())
}

// ─── Provider + tools setup ──────────────────────────────────────────────────

fn setup_provider(cli: &Cli, config: &Config) -> Result<(Box<dyn pi_ai::LlmProvider>, Model)> {
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

    // Resolve API key: config → env → stored Copilot credentials
    let api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var("PI_API_KEY").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .or_else(|| {
            // Try loading stored Copilot credentials and exchanging for a token
            if provider_name == "github-copilot" {
                load_copilot_token_blocking()
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No API key found. Run `pi login` for GitHub Copilot, \
                 set PI_API_KEY, or add `api_key = \"...\"` to ~/.pi/config.toml"
            )
        })?;

    let provider = pi_ai::get_provider(&provider_name, api_key)
        .map_err(|e| anyhow::anyhow!("Provider error: {e}"))?;

    let model_id = model_id_from_cli
        .or_else(|| config.model.clone())
        .unwrap_or_else(|| "gpt-4o".to_string());

    let (api, base_url, context_window, max_tokens) = match provider_name.as_str() {
        "minimax" => (
            ApiType::AnthropicMessages,
            "https://api.minimax.io/anthropic".to_string(),
            204800,
            131072,
        ),
        _ => (
            ApiType::OpenaiCompletions,
            "https://api.githubcopilot.com".to_string(),
            128000,
            16384,
        ),
    };

    let model = Model {
        id: model_id,
        name: "Default".to_string(),
        provider: provider_name,
        api,
        base_url,
        reasoning: false,
        input_types: vec!["text".to_string()],
        cost: ModelCost::default(),
        context_window,
        max_tokens,
        headers: Default::default(),
    };

    Ok((provider, model))
}

/// Resolve the sub-agent configuration from CLI args, env vars, and config file.
///
/// Returns `None` if no sub-agent API key is available (feature is disabled).
fn setup_sub_agent_config(
    cli: &Cli,
    config: &Config,
    basic_tools: Vec<Arc<dyn pi_tools::Tool>>,
) -> Option<Arc<SubAgentConfig>> {
    let sub_cfg = config.sub_agent.as_ref();

    // Resolve the API key: --sub-agent-key > [sub_agent] api_key > MINIMAX_API_KEY
    let api_key = cli
        .sub_agent_key
        .clone()
        .or_else(|| sub_cfg.and_then(|c| c.api_key.clone()))
        .or_else(|| std::env::var("MINIMAX_API_KEY").ok())?;

    let provider_name = sub_cfg
        .and_then(|c| c.provider.as_deref())
        .unwrap_or("minimax")
        .to_string();

    let model_id = sub_cfg
        .and_then(|c| c.model.as_deref())
        .unwrap_or("MiniMax-M2.7-highspeed")
        .to_string();

    let system_prompt = sub_cfg.and_then(|c| c.system_prompt.clone());

    let provider = match pi_ai::get_provider(&provider_name, api_key) {
        Ok(p) => Arc::from(p),
        Err(e) => {
            eprintln!("Warning: sub-agent provider error ({e}); spawn_agent tool disabled.");
            return None;
        }
    };

    let model = minimax_model(&model_id);

    Some(Arc::new(SubAgentConfig {
        provider,
        model,
        tools: basic_tools,
        system_prompt,
    }))
}

/// Build the base set of 7 file/shell tools.
fn create_basic_tools() -> Vec<Arc<dyn pi_tools::Tool>> {
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

/// Build the full orchestrator tool list, optionally including SpawnAgentTool.
fn create_tools(sub_agent_config: Option<Arc<SubAgentConfig>>) -> Vec<Arc<dyn pi_tools::Tool>> {
    let mut tools: Vec<Arc<dyn pi_tools::Tool>> = create_basic_tools();
    if let Some(cfg) = sub_agent_config {
        tools.push(Arc::new(SpawnAgentTool::new(cfg)));
    }
    tools
}

fn build_agent(cli: &Cli, config: &Config) -> Result<Agent> {
    let (provider, model) = setup_provider(cli, config)?;

    let basic_tools = create_basic_tools();
    let sub_config = setup_sub_agent_config(cli, config, basic_tools.clone());
    let tools = create_tools(sub_config);

    let agent_config = AgentConfig {
        provider: Arc::from(provider),
        tools,
        tool_execution_mode: ToolExecutionMode::Sequential,
        hooks: pi_agent::Hooks::default(),
    };

    // Apply guidance based on provider
    let cwd = std::env::current_dir().unwrap_or_default();
    let system_prompt =
        guidance::apply_guidance(config.system_prompt.clone(), &model.provider, &cwd);

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

    let basic_tools = create_basic_tools();
    let sub_config = setup_sub_agent_config(cli, config, basic_tools.clone());
    let tools = create_tools(sub_config);

    let agent_config = AgentConfig {
        provider: Arc::clone(&provider),
        tools,
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
    run_repl(agent, provider, cli, config).await
}

async fn run_repl(
    mut agent: Agent,
    provider: Arc<dyn pi_ai::LlmProvider>,
    cli: &Cli,
    config: &Config,
) -> Result<()> {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let stdout = io::stdout();

    // Re-take the event receiver so we can print streaming output.
    let mut event_rx = agent
        .take_event_receiver()
        .expect("event receiver available");

    // Initialize tab status tracker
    let mut tab = tab_status::TabStatus::new();

    println!("Pi agent — type your message, /model <id>, /ralph <cmd>, or Ctrl-C to quit.");

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

        // Handle /ralph commands
        if input.starts_with("/ralph") {
            let args = input.strip_prefix("/ralph").unwrap().trim();
            let basic_tools = create_basic_tools();
            let sub_config = setup_sub_agent_config(cli, config, basic_tools.clone());

            if let Some(rest) = args.strip_prefix("start") {
                if let Some(sub_cfg) = &sub_config {
                    let spawn_tool = SpawnAgentTool::new(Arc::clone(sub_cfg));
                    let cancel = tokio_util::sync::CancellationToken::new();
                    if let Err(e) =
                        ralph::handle_ralph_start(rest.trim(), &spawn_tool, cancel).await
                    {
                        eprintln!("Ralph error: {e}");
                    }
                } else {
                    eprintln!("Ralph requires a sub-agent provider. Configure [sub_agent] in config.toml.");
                }
            } else if args == "status" {
                if let Err(e) = ralph::handle_ralph_status() {
                    eprintln!("Ralph error: {e}");
                }
            } else if let Some(name) = args.strip_prefix("stop") {
                if let Err(e) = ralph::handle_ralph_stop(name.trim()) {
                    eprintln!("Ralph error: {e}");
                }
            } else if let Some(name) = args.strip_prefix("archive") {
                if let Err(e) = ralph::handle_ralph_archive(name.trim()) {
                    eprintln!("Ralph error: {e}");
                }
            } else {
                println!("Usage: /ralph start <name> | status | stop <name> | archive <name>");
            }
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

            let basic_tools = create_basic_tools();
            let sub_config = setup_sub_agent_config(cli, config, basic_tools.clone());
            let tools = create_tools(sub_config);

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
            event_rx = agent
                .take_event_receiver()
                .expect("event receiver available");
        }

        // Run prompt and drain events concurrently.
        let prompt_input = input.clone();
        let prompt_fut = agent.prompt(prompt_input);
        let drain_fut = drain_events_with_tab(&mut event_rx, &mut tab);

        let (prompt_result, _) = tokio::join!(prompt_fut, drain_fut);

        if let Err(e) = prompt_result {
            eprintln!("Error: {e}");
        }

        println!();
    }

    tab.reset();
    Ok(())
}

/// Drain events, printing text deltas and updating tab status until AgentEnd.
async fn drain_events_with_tab(
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    tab: &mut tab_status::TabStatus,
) {
    use std::io::Write;
    loop {
        match event_rx.recv().await {
            Some(
                ref event @ AgentEvent::MessageUpdate {
                    event: ChatEvent::TextDelta { ref text },
                },
            ) => {
                print!("{text}");
                let _ = std::io::stdout().flush();
                tab.handle_event(event);
            }
            Some(ref event @ AgentEvent::AgentEnd { stop_reason }) => {
                tab.handle_event(event);
                if stop_reason == StopReason::Error {
                    eprintln!("\nAgent ended with error.");
                }
                break;
            }
            Some(ref event) => {
                tab.handle_event(event);
            }
            None => break,
        }
    }
}

// ─── Copilot token helper ────────────────────────────────────────────────────

/// Load stored GitHub credentials and exchange for a Copilot API token (blocking).
fn load_copilot_token_blocking() -> Option<String> {
    use pi_ai::auth;

    let creds = auth::load_credentials().ok()??;
    let rt = tokio::runtime::Handle::try_current().ok()?;

    // We're inside a tokio runtime, so use block_in_place to avoid nesting
    let result = tokio::task::block_in_place(|| {
        rt.block_on(async {
            let client = reqwest::Client::new();
            auth::exchange_copilot_token(
                &client,
                &creds.github_token,
                creds.enterprise_domain.as_deref(),
            )
            .await
        })
    });

    match result {
        Ok(token) => Some(token.token),
        Err(e) => {
            eprintln!(
                "Warning: stored credentials expired or invalid ({e}). Run `pi login` again."
            );
            None
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
