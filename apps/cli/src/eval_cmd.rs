//! `zene eval` — harness evolution automation: paired evaluation (`run`) and
//! the failure-driven propose loop (`evolve`).
//!
//! Proposals are untrusted input: they pass strict JSON parsing here and
//! `zene_eval::apply_mutations` admission before becoming a candidate tree.
//! The candidate tree is written to disk for human promotion; nothing is
//! auto-applied to the baseline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use clap::{Args, Subcommand};
use zene_config::ZeneConfig;
use zene_core::{Agent, PromptOptions};
use zene_eval::{
    append_decision_record, apply_mutations, decide_floor, decide_win_margin, parse_mutations,
    run_episodes, run_paired_episodes, DecisionRecord, EpisodeExecutor, EpisodeRun, EpisodeTask,
    ExactAnswerScorer, FailureEvidence, HarnessTree, Mutation, Proposer,
};
use zene_llm::{ChatClient, ChatRequest, Message};

#[derive(Subcommand)]
pub(crate) enum EvalCommands {
    /// Run paired episodes for a task set, decide, and record the decision
    Run(RunArgs),
    /// Discover failures on the current tree, propose mutations, and evaluate the candidate
    Evolve(EvolveArgs),
}

#[derive(Args)]
pub(crate) struct RunArgs {
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
}

#[derive(Args)]
pub(crate) struct EvolveArgs {
    /// JSON task set: [{"id": "...", "prompt": "...", "expected": "..."}]
    #[arg(long)]
    tasks: PathBuf,
    /// Harness tree JSON (baseline; never modified by this command)
    #[arg(long)]
    tree: PathBuf,
    /// Root for episode workdirs (default: <workdir>/.zene/eval-runs)
    #[arg(long)]
    root: Option<PathBuf>,
    /// Selection policy for the candidate: win_margin | floor
    #[arg(long, default_value = "win_margin")]
    policy: String,
    /// win_margin: candidate wins must exceed losses by more than this
    #[arg(long, default_value_t = 0)]
    margin: i64,
    /// floor: minimum acceptable score on every task (policy=floor)
    #[arg(long)]
    floor: Option<f64>,
    /// Tasks scoring below this count as failures (default: 1.0)
    #[arg(long, default_value_t = 1.0)]
    failure_threshold: f64,
    /// Baseline identity recorded with the decision (e.g. git sha)
    #[arg(long, default_value = "unspecified")]
    baseline: String,
    /// Append the decision record (JSONL) to this path
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(serde::Deserialize)]
struct CliTask {
    id: String,
    prompt: String,
    #[serde(default)]
    expected: Option<String>,
}

pub(crate) async fn dispatch(command: EvalCommands, workdir: &Path) -> Result<()> {
    match command {
        EvalCommands::Run(args) => run(workdir, args).await,
        EvalCommands::Evolve(args) => evolve(workdir, args).await,
    }
}

fn load_tasks(path: &Path) -> Result<(Vec<EpisodeTask>, BTreeMap<String, String>)> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read task set {}", path.display()))?;
    let wire: Vec<CliTask> = serde_json::from_str(&raw).context("parse task set")?;
    let mut expected = BTreeMap::new();
    let mut tasks = Vec::new();
    for task in wire {
        if let Some(answer) = task.expected {
            expected.insert(task.id.clone(), answer);
        }
        tasks.push(EpisodeTask {
            id: task.id,
            prompt: task.prompt,
        });
    }
    Ok((tasks, expected))
}

fn decide(
    policy: &str,
    margin: i64,
    floor: Option<f64>,
    outcome: &zene_eval::PairedEpisodeOutcome,
) -> Result<zene_eval::SelectionDecision> {
    match policy {
        "win_margin" => {
            decide_win_margin(&outcome.candidate_scores, &outcome.incumbent_scores, margin)
        }
        "floor" => decide_floor(
            &outcome.candidate_scores,
            floor.context("--floor is required for policy=floor")?,
        ),
        other => bail!("unknown policy `{other}` (win_margin | floor)"),
    }
}

async fn run(workdir: &Path, args: RunArgs) -> Result<()> {
    let config = ZeneConfig::load(workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let (tasks, expected) = load_tasks(&args.tasks)?;
    let run_root = args.root.unwrap_or_else(|| workdir.join(".zene/eval-runs"));
    let executor = CoreEpisodeExecutor { config };
    let scorer = ExactAnswerScorer::new(expected);
    let outcome = run_paired_episodes(
        &tasks,
        &args.incumbent,
        &args.candidate,
        &run_root,
        &executor,
        &scorer,
    )
    .await?;
    let decision = decide(&args.policy, args.margin, args.floor, &outcome)?;
    println!("task_ids: {:?}", outcome.task_ids);
    println!("incumbent_scores: {:?}", outcome.incumbent_scores);
    println!("candidate_scores: {:?}", outcome.candidate_scores);
    println!("{}", serde_json::to_string_pretty(&decision)?);
    if let Some(out) = &args.out {
        let record = DecisionRecord {
            task_ids: outcome.task_ids,
            candidate_tree: args.candidate_tree,
            incumbent_tree: args.incumbent_tree,
            decision,
            baseline_commit: args.baseline,
            ts: chrono::Utc::now(),
        };
        append_decision_record(out, &record)?;
        println!("decision record: {}", out.display());
    }
    Ok(())
}

async fn evolve(workdir: &Path, args: EvolveArgs) -> Result<()> {
    let config = ZeneConfig::load(workdir).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let (tasks, expected) = load_tasks(&args.tasks)?;
    let tree_raw = std::fs::read_to_string(&args.tree)
        .with_context(|| format!("read harness tree {}", args.tree.display()))?;
    let tree: HarnessTree = serde_json::from_str(&tree_raw).context("parse harness tree JSON")?;
    tree.validate().context("validate harness tree")?;

    let run_root = args.root.unwrap_or_else(|| workdir.join(".zene/eval-runs"));
    let incumbent_dir = run_root.join("harness-incumbent");
    tree.render(&incumbent_dir)?;
    let executor = CoreEpisodeExecutor {
        config: config.clone(),
    };
    let scorer = ExactAnswerScorer::new(expected);

    // 1. discover failures on the current tree
    let discovered = run_episodes(
        &tasks,
        &incumbent_dir,
        &run_root.join("discover"),
        &executor,
        &scorer,
    )
    .await?;
    let failures: Vec<FailureEvidence> = discovered
        .iter()
        .filter(|episode| episode.score < args.failure_threshold)
        .map(|episode| FailureEvidence {
            task_id: episode.task_id.clone(),
            score: episode.score,
            final_text: episode.run.final_text.clone(),
            trajectory: episode.run.trajectory.clone(),
        })
        .collect();
    if failures.is_empty() {
        println!(
            "no task scored below {}; nothing to propose",
            args.failure_threshold
        );
        return Ok(());
    }
    let summary: Vec<(&str, f64)> = failures
        .iter()
        .map(|failure| (failure.task_id.as_str(), failure.score))
        .collect();
    println!("failures: {summary:?}");

    // 2. propose mutations (untrusted) -> admission -> candidate tree
    let mutations = LlmProposer { config }.propose(&tree, &failures).await?;
    println!(
        "proposed mutations: {}",
        serde_json::to_string_pretty(&mutations)?
    );
    let candidate = apply_mutations(&tree, &mutations).context("mutation admission")?;

    // 3. paired evaluation: incumbent vs candidate
    let candidate_dir = run_root.join("harness-candidate");
    candidate.render(&candidate_dir)?;
    let outcome = run_paired_episodes(
        &tasks,
        &incumbent_dir,
        &candidate_dir,
        &run_root.join("pairs"),
        &executor,
        &scorer,
    )
    .await?;
    let decision = decide(&args.policy, args.margin, args.floor, &outcome)?;
    println!("task_ids: {:?}", outcome.task_ids);
    println!("incumbent_scores: {:?}", outcome.incumbent_scores);
    println!("candidate_scores: {:?}", outcome.candidate_scores);
    println!("{}", serde_json::to_string_pretty(&decision)?);

    // 4. durable record + candidate tree for human promotion (never auto-applied)
    let candidate_out = args.tree.with_extension("candidate.json");
    std::fs::write(
        &candidate_out,
        serde_json::to_string_pretty(&candidate).context("serialize candidate tree")?,
    )
    .with_context(|| format!("write candidate tree {}", candidate_out.display()))?;
    if let Some(out) = &args.out {
        let record = DecisionRecord {
            task_ids: outcome.task_ids,
            candidate_tree: format!("{} + {} mutations", args.tree.display(), mutations.len()),
            incumbent_tree: args.tree.display().to_string(),
            decision,
            baseline_commit: args.baseline,
            ts: chrono::Utc::now(),
        };
        append_decision_record(out, &record)?;
        println!("decision record: {}", out.display());
    }
    println!(
        "candidate tree: {} — on Select, promote manually (cp over {})",
        candidate_out.display(),
        args.tree.display()
    );
    Ok(())
}

/// Runs one episode in-process: fresh workdir with the harness rendered in,
/// yolo permissions, fixed config. Returns the run's evidence.
struct CoreEpisodeExecutor {
    config: ZeneConfig,
}

#[async_trait]
impl EpisodeExecutor for CoreEpisodeExecutor {
    async fn run(&self, task: &EpisodeTask, workdir: &Path) -> Result<EpisodeRun> {
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

/// LLM-backed proposer: reads failure evidence and proposes strict-JSON
/// mutations. Output is untrusted and re-validated by admission afterwards.
struct LlmProposer {
    config: ZeneConfig,
}

const PROPOSER_SYSTEM: &str = "You improve a coding-agent harness (rules, skills, prompt, config).\n\
Given the current harness tree and failed task runs, propose the smallest mutations that fix the failures.\n\
You may only change textual harness content: rules (AGENTS.md sections), skills (SKILL.md with frontmatter),\n\
prompt (system prompt text), and non-sensitive config values.\n\
You may never set provider, model, api_key, base_url, anthropic_*, permission_mode, permission_rules,\n\
sandbox, or hooks: those stay outside the tree.\n\
Reply with STRICT JSON only, no prose, exactly:\n\
{\"mutations\":[{\"op\":\"create|update|remove\",\"id\":\"...\",\"kind\":\"rules|skill|config|prompt\",\"config\":{...}}]}\n\
kind is required for create and forbidden to differ on update; remove takes no config.";

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}…[truncated]")
}

#[async_trait]
impl Proposer for LlmProposer {
    async fn propose(
        &self,
        tree: &HarnessTree,
        failures: &[FailureEvidence],
    ) -> Result<Vec<Mutation>> {
        let client = ChatClient::from_config(&self.config).await?;
        let mut payload = format!(
            "CURRENT TREE:\n{}\n\nFAILURES:\n",
            serde_json::to_string_pretty(tree)?
        );
        for failure in failures {
            let trajectory = serde_json::to_string(&failure.trajectory)?;
            payload.push_str(&format!(
                "\n- task `{}` score {}\n  final_text: {}\n  trajectory: {}\n",
                failure.task_id,
                failure.score,
                truncate(&failure.final_text, 2_000),
                truncate(&trajectory, 4_000),
            ));
        }
        let response = client
            .chat(ChatRequest {
                model: self.config.model.clone(),
                messages: vec![Message::system(PROPOSER_SYSTEM), Message::user(payload)],
                tools: Vec::new(),
                stream: false,
                context: None,
                reasoning_effort: None,
            })
            .await?;
        let text = response.message.content.unwrap_or_default();
        parse_mutations(&text).context("parse proposer output")
    }
}
