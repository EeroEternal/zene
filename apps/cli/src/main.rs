use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use zene_config::{ensure_home, ZeneConfig};
use zene_core::{Agent, PromptOptions};
use zene_eval::{
    append_decision_record, decide_floor, decide_win_margin, run_paired_episodes, DecisionRecord,
    EpisodeExecutor, EpisodeRun, EpisodeTask, ExactAnswerScorer,
};
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

    /// Override sandbox profile (`off` | `workspace` | `read-only` | `strict` | custom)
    #[arg(long, global = true)]
    sandbox_profile: Option<String>,

    /// Extra egress allowlist hosts (comma-separated or multiple flags)
    #[arg(long, global = true, value_delimiter = ',')]
    allow_hosts: Vec<String>,
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
    /// Evaluate a harness change with paired episodes (harness evolution)
    Eval {
        #[command(subcommand)]
        command: EvalCommands,
    },
    /// Speak Agent Client Protocol (ACP) over stdio JSON-RPC
    Acp,
}

#[derive(Subcommand)]
enum McpCommands {
    /// List configured MCP servers and attempt a short connect
    Doctor,
}

#[derive(Subcommand)]
enum EvalCommands {
    /// Run paired episodes for a task set, decide, and record the decision
    Run {
        /// JSON task set: [{"id": "...", "prompt": "...", "expected": "..."}]
        #[arg(long)]
        tasks: PathBuf,
        /// Directory holding the incumbent harness files
        #[arg(long)]
        incumbent: PathBuf,
        /// Directory holding the candidate harness files
        #[arg(long)]
        candidate: PathBuf,
        /// Root for episode workdirs (default: <workdir>/.zene/eval-runs)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Selection policy: win_margin | floor
        #[arg(long, default_value = "win_margin")]
        policy: String,
        /// win_margin: candidate wins must exceed losses by more than this
        #[arg(long, default_value_t = 0)]
        margin: i64,
        /// floor: minimum acceptable score on every task (policy=floor)
        #[arg(long)]
        floor: Option<f64>,
        /// Baseline identity recorded with the decision (e.g. git sha)
        #[arg(long, default_value = "unspecified")]
        baseline: String,
        /// Candidate harness identity recorded with the decision
        #[arg(long, default_value = "candidate")]
        candidate_tree: String,
        /// Incumbent harness identity recorded with the decision
        #[arg(long, default_value = "incumbent")]
        incumbent_tree: String,
        /// Append the decision record (JSONL) to this path
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(serde::Deserialize)]
struct CliTask {
    id: String,
    prompt: String,
    #[serde(default)]
    expected: Option<String>,
}

/// Runs one episode in-process: fresh workdir with the harness rendered in,
/// yolo permissions, fixed config. Returns the run's evidence.
struct CoreEpisodeExecutor {
    config: ZeneConfig,
}

#[async_trait]
impl EpisodeExecutor for CoreEpisodeExecutor {
    async fn run(&self, task: &EpisodeTask, workdir: &std::path::Path) -> Result<EpisodeRun> {
        let mut agent = Agent::builder(workdir)
            .config(self.config.clone())
            .core_tools()
            .bypass_permissions()
            .without_mcp()
            .build()
            .await?;
        let final_text = agent
            .prompt(
                &task.prompt,
                PromptOptions {
                    quiet: true,
                    ..Default::default()
                },
            )
            .await?;
        let trajectory = agent.execution_record_writer().read_all()?;
        Ok(EpisodeRun {
            final_text,
            trajectory,
        })
    }
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
        Some(Commands::Eval { command }) => match command {
            EvalCommands::Run {
                tasks,
                incumbent,
                candidate,
                root,
                policy,
                margin,
                floor,
                baseline,
                candidate_tree,
                incumbent_tree,
                out,
            } => {
                let config =
                    ZeneConfig::load(&workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
                let raw = std::fs::read_to_string(&tasks)
                    .with_context(|| format!("read task set {}", tasks.display()))?;
                let wire: Vec<CliTask> = serde_json::from_str(&raw).context("parse task set")?;
                let mut expected = std::collections::BTreeMap::new();
                let mut task_list = Vec::new();
                for task in wire {
                    if let Some(answer) = task.expected {
                        expected.insert(task.id.clone(), answer);
                    }
                    task_list.push(EpisodeTask {
                        id: task.id,
                        prompt: task.prompt,
                    });
                }
                let run_root = root.unwrap_or_else(|| workdir.join(".zene/eval-runs"));
                let executor = CoreEpisodeExecutor { config };
                let scorer = ExactAnswerScorer::new(expected);
                let outcome = run_paired_episodes(
                    &task_list, &incumbent, &candidate, &run_root, &executor, &scorer,
                )
                .await?;
                let decision = match policy.as_str() {
                    "win_margin" => decide_win_margin(
                        &outcome.candidate_scores,
                        &outcome.incumbent_scores,
                        margin,
                    )?,
                    "floor" => decide_floor(
                        &outcome.candidate_scores,
                        floor.context("--floor is required for policy=floor")?,
                    )?,
                    other => bail!("unknown policy `{other}` (win_margin | floor)"),
                };
                println!("task_ids: {:?}", outcome.task_ids);
                println!("incumbent_scores: {:?}", outcome.incumbent_scores);
                println!("candidate_scores: {:?}", outcome.candidate_scores);
                println!("{}", serde_json::to_string_pretty(&decision)?);
                if let Some(out) = out {
                    let record = DecisionRecord {
                        task_ids: outcome.task_ids,
                        candidate_tree,
                        incumbent_tree,
                        decision,
                        baseline_commit: baseline,
                        ts: chrono::Utc::now(),
                    };
                    append_decision_record(&out, &record)?;
                    println!("decision record: {}", out.display());
                }
                Ok(())
            }
        },
        Some(Commands::Acp) => {
            let sandbox_profile = cli.sandbox_profile.or_else(|| {
                std::env::var("ZENE_SANDBOX_PROFILE")
                    .or_else(|_| std::env::var("ZENE_SANDBOX"))
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            });
            let mut allow_hosts = cli.allow_hosts;
            if allow_hosts.is_empty() {
                if let Ok(hosts) = std::env::var("ZENE_SANDBOX_ALLOW_HOSTS") {
                    allow_hosts = hosts
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
            }
            acp::run_acp(workdir, cli.yolo, sandbox_profile, allow_hosts).await?;
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
            println!("  eval      Run paired harness episodes and record the decision");
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
