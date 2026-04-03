use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pi", version, about = "Pi coding agent")]
pub struct Cli {
    /// LLM provider and model (e.g., "github-copilot:gpt-4o")
    #[arg(short, long)]
    pub model: Option<String>,

    /// Resume or create a named session
    #[arg(short, long)]
    pub session: Option<String>,

    /// Run mode
    #[arg(long, default_value = "interactive")]
    pub mode: Mode,

    /// Non-interactive prompt (implies --mode print)
    #[arg(short, long)]
    pub prompt: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Clone, clap::ValueEnum)]
pub enum Mode {
    Interactive,
    Print,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List saved sessions
    Sessions,
}
