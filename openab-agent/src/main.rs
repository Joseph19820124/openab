mod acp;
mod agent;
mod auth;
mod llm;
mod skills;
mod tools;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "openab-agent", about = "Native Rust coding agent with ACP")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with an LLM provider
    Auth {
        #[command(subcommand)]
        provider: AuthProvider,
    },
}

#[derive(Subcommand)]
enum AuthProvider {
    /// OpenAI Codex via browser PKCE flow (recommended, full scopes)
    CodexOauth {
        /// Print URL instead of opening browser
        #[arg(long)]
        no_browser: bool,
    },
    /// OpenAI Codex via device code (headless servers)
    CodexDevice,
    /// Show stored credentials
    Status,
}

#[tokio::main]
async fn main() {
    // When OPENAB_AGENT_LOG_FILE is set, route tracing output to that file
    // (appended). Useful when the agent is spawned by openab, which swallows
    // child stderr.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("openab_agent=info"));
    if let Ok(path) = std::env::var("OPENAB_AGENT_LOG_FILE") {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(std::sync::Mutex::new(file))
                    .with_ansi(false)
                    .init();
            }
            Err(e) => {
                eprintln!(
                    "OPENAB_AGENT_LOG_FILE={path}: open failed ({e}), falling back to stderr"
                );
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(std::io::stderr)
                    .init();
            }
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }

    let cli = Cli::parse();

    match cli.command {
        None => {
            // Default: run ACP server
            let mut server = acp::AcpServer::new();
            server.run().await;
        }
        Some(Commands::Auth { provider }) => match provider {
            AuthProvider::CodexOauth { no_browser } => {
                if let Err(e) = auth::login_browser_flow(no_browser).await {
                    eprintln!("❌ Authentication failed: {e}");
                    std::process::exit(1);
                }
            }
            AuthProvider::CodexDevice => {
                if let Err(e) = auth::login_codex_device_flow().await {
                    eprintln!("❌ Authentication failed: {e}");
                    std::process::exit(1);
                }
            }
            AuthProvider::Status => {
                auth::show_status();
            }
        },
    }
}
