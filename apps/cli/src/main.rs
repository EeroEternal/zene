use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zene_config::{ensure_home, ZeneConfig};
use zene_core::{ensure_session_worktree, Agent, AgentEvent, PermissionMode, PromptOptions};
use zene_sandbox::LocalSandbox;
use zene_session::{export_session, list_sessions_for_workdir, SessionRecord};
use std::sync::Arc;

mod acp;
mod model_config;
mod repl;
mod sandbox_opts;
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

    /// Run a single prompt headlessly (no TUI/REPL) and exit
    #[arg(short = 'p', long = "prompt")]
    prompt: Option<String>,

    /// Headless output format: `text` (default) or `json`
    #[arg(long, default_value = "text")]
    output_format: String,

    /// Run the session inside a dedicated git worktree under `.zene/worktrees/`
    #[arg(long)]
    worktree: bool,

    /// Keel sandbox profile: `off`, `workspace`, `read-only`, `strict`, or a custom
    /// name from `~/.zene/sandbox.toml` / `.zene/sandbox.toml`
    #[arg(long)]
    sandbox: Option<String>,
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
    /// Launch the local Web Agent UI via `zene-gateway`
    Web {
        /// Extra arguments forwarded to `zene-gateway` (e.g. `--port 8787 --yolo`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        gateway_args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum McpCommands {
    /// List configured MCP servers and attempt a short connect
    Doctor,
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
        init_tracing(!is_repl);
    }

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
            println!(
                "sandbox.profile: {} (effective)",
                config.sandbox.effective_profile(config.agent_profile)
            );
            if !config.sandbox.allow_hosts.is_empty() {
                println!("sandbox.allow_hosts: {:?}", config.sandbox.allow_hosts);
            }
            println!("sandbox.auto_allow_bash: {}", config.sandbox.auto_allow_bash);
            return Ok(());
        }
        Some(Commands::Export { session, output }) => {
            export_session(&session, &output).context("export session")?;
            println!("Exported session {} to {}", session, output.display());
            return Ok(());
        }
        Some(Commands::Mcp { command }) => {
            match command {
                McpCommands::Doctor => {
                    run_mcp_doctor(&workdir).await?;
                }
            }
            return Ok(());
        }
        Some(Commands::Acp) => {
            acp::run_acp(workdir, cli.yolo).await?;
            return Ok(());
        }
        Some(Commands::Web { gateway_args }) => {
            run_web_gateway(gateway_args)?;
            return Ok(());
        }
        None => {}
    }

    let config = ZeneConfig::load(&workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let mut session = if let Some(ref id) = cli.session {
        SessionRecord::load(id).context("load session")?
    } else {
        SessionRecord::new(&workdir)
    };

    let agent_workdir = if cli.worktree {
        let wt = ensure_session_worktree(&workdir, &session.meta.id)
            .context("create session git worktree")?;
        eprintln!("Using git worktree: {}", wt.display());
        session.meta.workdir = wt.display().to_string();
        wt
    } else {
        workdir.clone()
    };

    let permission_mode = if cli.yolo {
        PermissionMode::BypassPermissions
    } else {
        PermissionMode::parse(&config.permission_mode)
    };

    let sandbox_opts = sandbox_opts::build_sandbox_options(&config, cli.sandbox.as_deref());
    eprintln!(
        "Sandbox profile: {}{}",
        sandbox_opts.profile,
        if sandbox_opts.is_off() {
            " (no Keel enforcement)"
        } else {
            ""
        }
    );
    let sandbox = LocalSandbox::with_options(&agent_workdir, sandbox_opts)
        .await
        .context("initialize Keel execution layer")?;
    let mut agent = Agent::new(config.clone(), sandbox, session, permission_mode).await?;

    if let Some(prompt) = cli.prompt.as_deref() {
        run_headless(&mut agent, prompt, &cli).await?;
        agent.shutdown().await?;
        return Ok(());
    }

    if !cli.repl {
        return tui::run(agent, &config, &cli).await;
    }

    repl::run_repl(&mut agent, &cli).await?;
    agent.shutdown().await?;

    Ok(())
}

async fn run_headless(agent: &mut Agent, prompt: &str, cli: &Cli) -> Result<()> {
    let json_mode = cli.output_format.eq_ignore_ascii_case("json");
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_handler = Arc::clone(&events);
    let event_handler: Option<zene_core::EventHandler> = if json_mode {
        Some(Arc::new(move |event: AgentEvent| {
            if let Some(value) = headless_event_json(&event) {
                events_for_handler.lock().push(value);
            }
        }))
    } else {
        None
    };

    let text = agent
        .prompt(
            prompt,
            PromptOptions {
                stream: !cli.no_stream && !json_mode,
                cancel: None,
                event_handler,
                quiet: json_mode,
            },
        )
        .await?;

    if json_mode {
        let payload = serde_json::json!({
            "sessionId": agent.session().meta.id,
            "model": agent.config().model,
            "text": text,
            "usage": {
                "prompt_tokens": agent.turn_usage().prompt_tokens,
                "completion_tokens": agent.turn_usage().completion_tokens,
                "total_tokens": agent.turn_usage().total_tokens,
            },
            "context": {
                "percent": agent.context_water().usage_percent(),
                "window": agent.config().compaction.context_window_tokens,
            },
            "events": *events.lock(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if cli.no_stream {
        println!("{text}");
    } else if !cli.quiet_usage {
        let usage = agent.turn_usage();
        eprintln!(
            "tokens: input {} / output {} ({}) | ctx {}%",
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens,
            agent.context_water().usage_percent()
        );
    }
    Ok(())
}

fn headless_event_json(event: &AgentEvent) -> Option<serde_json::Value> {
    match event {
        AgentEvent::ToolCall { id, name, arguments } => Some(serde_json::json!({
            "type": "tool_call",
            "id": id,
            "name": name,
            "arguments": arguments,
        })),
        AgentEvent::ToolResult {
            id,
            name,
            content,
            is_error,
            duration_ms,
        } => Some(serde_json::json!({
            "type": "tool_result",
            "id": id,
            "name": name,
            "content": content,
            "is_error": is_error,
            "duration_ms": duration_ms,
        })),
        AgentEvent::Error { message } => Some(serde_json::json!({
            "type": "error",
            "message": message,
        })),
        _ => None,
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
    let defs = tools.definitions();
    println!("Connected MCP tools: {}", defs.len());
    for def in defs {
        println!("  - {}", def.name);
    }
    Ok(())
}

fn run_web_gateway(gateway_args: Vec<String>) -> Result<()> {
    let gateway = resolve_gateway_bin();
    let mut cmd = std::process::Command::new(&gateway);
    if !gateway_args
        .iter()
        .any(|arg| arg == "--zene-bin" || arg.starts_with("--zene-bin="))
    {
        if let Ok(zene) = std::env::current_exe() {
            cmd.arg("--zene-bin").arg(zene);
        }
    }
    cmd.args(&gateway_args);
    let status = cmd
        .status()
        .with_context(|| format!("failed to launch {}", gateway.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn resolve_gateway_bin() -> PathBuf {
    if let Ok(path) = std::env::var("ZENE_GATEWAY_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("zene-gateway");
        if sibling.exists() {
            return sibling;
        }
        #[cfg(windows)]
        {
            let sibling = exe.with_file_name("zene-gateway.exe");
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from("zene-gateway")
}
