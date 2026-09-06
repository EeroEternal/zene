use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use zene_config::{ensure_home, ZeneConfig};
use zene_session::{export_session, list_sessions_for_workdir};

mod acp;

#[derive(Parser)]
#[command(
    name = "zene",
    about = "Zene agent binary (ACP for Cloud workers / editors)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Working directory for the agent session
    #[arg(long, default_value = ".", global = true)]
    workdir: PathBuf,

    /// Auto-approve Write / Edit / Bash (yolo permission mode; used by `zene acp`)
    #[arg(long, global = true)]
    yolo: bool,
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
    /// Probe configured MCP servers (stdio connectivity)
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    /// Speak Agent Client Protocol (ACP) over stdio JSON-RPC
    Acp,
}

#[derive(Subcommand)]
enum McpCommands {
    /// List configured MCP servers and attempt a short connect
    Doctor,
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter("zene=info")
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli_args: Vec<String> = std::env::args().collect();
    let is_acp = cli_args.iter().any(|a| a == "acp");
    if is_acp {
        // Keep ACP stdout reserved for NDJSON; send logs to stderr.
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("zene=warn")),
            )
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        init_tracing();
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
            Ok(())
        }
        Some(Commands::Config) => {
            let config =
                ZeneConfig::load(&workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
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
            println!(
                "sandbox.profile: {} (effective)",
                config.sandbox.effective_profile(config.agent_profile)
            );
            if !config.sandbox.allow_hosts.is_empty() {
                println!("sandbox.allow_hosts: {:?}", config.sandbox.allow_hosts);
            }
            println!(
                "sandbox.auto_allow_bash: {}",
                config.sandbox.auto_allow_bash
            );
            Ok(())
        }
        Some(Commands::Export { session, output }) => {
            export_session(&session, &output).context("export session")?;
            println!("Exported session {} to {}", session, output.display());
            Ok(())
        }
        Some(Commands::Mcp { command }) => {
            match command {
                McpCommands::Doctor => {
                    run_mcp_doctor(&workdir).await?;
                }
            }
            Ok(())
        }
        Some(Commands::Acp) => {
            acp::run_acp(workdir, cli.yolo).await?;
            Ok(())
        }
        None => {
            println!("Zene (Zen Engine) — The Open, Minimalist Agent Harness");
            println!();
            println!("Usage: zene [OPTIONS] <COMMAND>");
            println!();
            println!("Commands:");
            println!("  acp       Speak Agent Client Protocol (ACP) over stdio JSON-RPC");
            println!("  sessions  List saved sessions for the current workdir");
            println!("  config    Print config path and defaults");
            println!("  export    Export a session and its record to a zip file");
            println!("  mcp       Probe configured MCP servers");
            println!();
            println!("Run 'zene --help' for more options.");
            Ok(())
        }
    }
}

async fn run_mcp_doctor(workdir: &std::path::Path) -> Result<()> {
    use zene_mcp::McpManager;
    let (manager, tools) = McpManager::connect(workdir).await?;
    if manager.is_empty() {
        println!("No MCP servers configured.");
        println!(
            "Add servers in {} or {}.",
            zene_config::mcp_config_path().display(),
            workdir.join(".zene").join("mcp.json").display()
        );
        return Ok(());
    }
    let defs = tools.registered_definitions();
    println!("Connected MCP tools: {}", defs.len());
    for def in defs {
        println!("  - {}", def.name);
    }
    Ok(())
}
