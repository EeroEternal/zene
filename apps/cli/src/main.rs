use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zene_config::{ensure_home, ZeneConfig};
use zene_core::{Agent, PermissionMode};
use zene_sandbox::LocalSandbox;
use zene_session::{export_session, list_sessions_for_workdir, SessionRecord};

mod repl;
mod model_config;
mod tui;

/// Shared cancel token for the in-flight REPL turn (Ctrl+C or `/cancel`).
static ACTIVE_CANCEL: Mutex<Option<CancellationToken>> = Mutex::new(None);

pub(crate) fn set_active_cancel(token: Option<CancellationToken>) {
    *ACTIVE_CANCEL.lock() = token;
}

pub(crate) fn cancel_active_turn() -> bool {
    let mut guard = ACTIVE_CANCEL.lock();
    if let Some(token) = guard.take() {
        token.cancel();
        true
    } else {
        false
    }
}

#[derive(Parser)]
#[command(name = "zene", about = "Local coding agent CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Working directory for the agent session
    #[arg(long, default_value = ".")]
    workdir: PathBuf,

    /// Resume a previous session by id
    #[arg(long)]
    session: Option<String>,

    /// Disable streaming output
    #[arg(long)]
    no_stream: bool,

    /// Auto-approve Write / Edit / Bash (yolo permission mode)
    #[arg(long)]
    yolo: bool,

    /// Launch ratatui TUI instead of line REPL (now default)
    #[arg(long)]
    tui: bool,

    /// Launch line REPL instead of ratatui TUI
    #[arg(long)]
    repl: bool,

    /// Print tool_call / tool_result events to stderr
    #[arg(long)]
    verbose_events: bool,

    /// Hide per-turn token usage line
    #[arg(long)]
    quiet_usage: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// List saved sessions for the current workdir
    Sessions,
    /// Print config path and defaults
    Config,
    /// Export a session and its record to a zip file
    Export {
        /// Session id to export
        #[arg(long)]
        session: String,
        /// Output zip path
        #[arg(long)]
        output: PathBuf,
    },
}

fn init_tracing(use_tui: bool) {
    if use_tui {
        // Ratatui uses the alternate screen on stdout; stderr writes also corrupt the UI.
        tracing_subscriber::fmt()
            .with_env_filter("off")
            .with_target(false)
            .with_writer(std::io::sink)
            .init();
        return;
    }
    tracing_subscriber::fmt()
        .with_env_filter("zene=info")
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli_args: Vec<String> = std::env::args().collect();
    let is_repl = cli_args.iter().any(|a| a == "--repl");
    init_tracing(!is_repl);

    if is_repl {
        ctrlc::set_handler(|| {
            if cancel_active_turn() {
                eprintln!("\n[cancelled]");
            }
        })
        .context("install Ctrl+C handler")?;
    }

    ensure_home().map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let cli = Cli::parse();
    let workdir = std::env::current_dir().context("resolve current directory")?;
    let workdir = if cli.workdir.as_os_str() == std::ffi::OsStr::new(".") {
        workdir
    } else {
        workdir.join(&cli.workdir)
    };
    let workdir = workdir
        .canonicalize()
        .with_context(|| format!("invalid workdir: {}", workdir.display()))?;

    match cli.command {
        Some(Commands::Sessions) => {
            let sessions = list_sessions_for_workdir(&workdir)?;
            if sessions.is_empty() {
                println!("No saved sessions for {}", workdir.display());
            } else {
                for session in sessions {
                    println!(
                        "{}  {}  {}",
                        session.id,
                        session.updated_at.format("%Y-%m-%d %H:%M"),
                        session.title
                    );
                }
            }
            return Ok(());
        }
        Some(Commands::Config) => {
            let config = ZeneConfig::load(&workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
            println!("config: {}", zene_config::config_path().display());
            println!(
                "project config: {}",
                zene_config::project_config_path(&workdir).display()
            );
            println!("hooks: {}", zene_config::hooks_path().display());
            println!("mcp: {}", zene_config::mcp_config_path().display());
            println!("home: {}", zene_config::zene_home().display());
            println!("model: {}", config.model);
            println!("base_url: {}", config.base_url);
            println!("permission_mode: {}", config.permission_mode);
            return Ok(());
        }
        Some(Commands::Export { session, output }) => {
            export_session(&session, &output).context("export session")?;
            println!("Exported session {} to {}", session, output.display());
            return Ok(());
        }
        None => {}
    }

    let config = ZeneConfig::load(&workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let session = if let Some(ref id) = cli.session {
        SessionRecord::load(id).context("load session")?
    } else {
        SessionRecord::new(&workdir)
    };

    let permission_mode = if cli.yolo {
        PermissionMode::Yolo
    } else {
        PermissionMode::parse(&config.permission_mode)
    };

    let sandbox = LocalSandbox::new(&workdir);
    let mut agent = Agent::new(config.clone(), sandbox, session, permission_mode).await?;

    if !cli.repl {
        return tui::run(agent, &config, &cli).await;
    }

    repl::run_repl(&mut agent, &cli).await?;
    agent.shutdown().await?;

    Ok(())
}
