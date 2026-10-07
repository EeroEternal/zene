//! `zene analysis` — read-only digest over harness-evolution results.
//!
//! Renders what background `zene eval` runs accumulated (decision records,
//! pending candidate trees, repeated task failures) into one page for human
//! judgment. This command never modifies anything: applying a decision stays a
//! separate explicit step (docs/harness-evolution.md §5).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use zene_eval::{DecisionRecord, SelectionOutcome};

#[derive(Args)]
pub(crate) struct AnalysisArgs {
    /// Decision record JSONL appended by `zene eval run/evolve --out`
    #[arg(long)]
    records: Option<PathBuf>,
    /// Harness tree JSON; a differing `<tree>.candidate.json` counts as pending
    #[arg(long)]
    tree: Option<PathBuf>,
    /// How many recent decisions to list
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

pub(crate) fn run(workdir: &Path, args: AnalysisArgs) -> Result<()> {
    let records_path = args
        .records
        .unwrap_or_else(|| workdir.join(".zene/eval-runs/decisions.jsonl"));
    let tree_path = args.tree.unwrap_or_else(|| workdir.join("tree.json"));
    let records = load_records(&records_path)?;
    let pending = pending_candidate(&tree_path);
    print!(
        "{}",
        render_digest(&records, &records_path, pending.as_deref(), args.limit)
    );
    Ok(())
}

fn load_records(path: &Path) -> Result<Vec<DecisionRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read decision records {}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str(line)
                .with_context(|| format!("parse {} line {}", path.display(), index + 1))?,
        );
    }
    Ok(records)
}

/// A candidate tree that differs from the baseline tree is a pending judgment.
fn pending_candidate(tree: &Path) -> Option<String> {
    let candidate = tree.with_extension("candidate.json");
    let candidate_raw = std::fs::read_to_string(&candidate).ok()?;
    let tree_raw = std::fs::read_to_string(tree).ok()?;
    if candidate_raw == tree_raw {
        return None;
    }
    Some(candidate.display().to_string())
}

/// Per-task failure stats across every recorded decision (both sides of each
/// pair count; "failed" means the scorer gave less than 1.0).
fn task_stats(records: &[DecisionRecord]) -> BTreeMap<&str, TaskStats> {
    let mut stats: BTreeMap<&str, TaskStats> = BTreeMap::new();
    for record in records {
        for (index, task_id) in record.task_ids.iter().enumerate() {
            let entry = stats.entry(task_id.as_str()).or_default();
            let scores = record
                .decision
                .incumbent_scores
                .get(index)
                .into_iter()
                .chain(record.decision.candidate_scores.get(index));
            for score in scores {
                entry.seen += 1;
                if *score < 1.0 {
                    entry.failed += 1;
                }
                entry.worst = if entry.seen == 1 {
                    *score
                } else {
                    entry.worst.min(*score)
                };
            }
        }
    }
    stats
}

#[derive(Default)]
struct TaskStats {
    seen: usize,
    failed: usize,
    worst: f64,
}

fn render_digest(
    records: &[DecisionRecord],
    records_path: &Path,
    pending: Option<&str>,
    limit: usize,
) -> String {
    let mut out = String::from("# Harness analysis (read-only)\n\n");

    match pending {
        Some(candidate) => out.push_str(&format!(
            "## Pending judgment (1)\n- `{candidate}` differs from the baseline tree — promote it manually if the decision below is Select, or delete it\n\n"
        )),
        None => out.push_str("## Pending judgment (0)\n\n"),
    }

    if records.is_empty() {
        out.push_str(&format!(
            "No decision records at `{}`.\nProduce some with `zene eval run|evolve --out {}`.\n",
            records_path.display(),
            records_path.display()
        ));
        return out;
    }

    let stats = task_stats(records);
    out.push_str("## Repeated failures (optimization candidates)\n");
    let mut failures: Vec<_> = stats.iter().filter(|(_, stat)| stat.failed > 0).collect();
    failures.sort_by_key(|a| std::cmp::Reverse(a.1.failed));
    if failures.is_empty() {
        out.push_str("- none: every recorded task scored 1.0\n");
    }
    for (task_id, stat) in failures {
        out.push_str(&format!(
            "- task `{task_id}` failed {}/{} runs (worst {:.2})\n",
            stat.failed, stat.seen, stat.worst
        ));
    }

    out.push_str(&format!("\n## Recent decisions (last {limit})\n"));
    for record in records.iter().rev().take(limit) {
        let outcome = match record.decision.outcome {
            SelectionOutcome::Select => "Select",
            SelectionOutcome::Reject => "Reject",
        };
        out.push_str(&format!(
            "- {}  {}  [{}] {}  tasks={:?}  baseline={}\n",
            record.ts.format("%Y-%m-%d %H:%M"),
            outcome,
            record.decision.policy,
            record.decision.reason,
            record.task_ids,
            record.baseline_commit,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use zene_eval::SelectionDecision;

    fn record(
        outcome: SelectionOutcome,
        task_ids: &[&str],
        candidate_scores: &[f64],
        incumbent_scores: &[f64],
    ) -> DecisionRecord {
        DecisionRecord {
            task_ids: task_ids.iter().map(|id| id.to_string()).collect(),
            candidate_tree: "candidate".into(),
            incumbent_tree: "incumbent".into(),
            decision: SelectionDecision {
                outcome,
                policy: "win_margin".into(),
                policy_version: 1,
                reason: "wins=1 losses=0 net=1 margin=0".into(),
                candidate_scores: candidate_scores.to_vec(),
                incumbent_scores: incumbent_scores.to_vec(),
            },
            baseline_commit: "abc123".into(),
            ts: Utc::now(),
        }
    }

    #[test]
    fn digest_counts_repeated_failures_and_lists_recent_decisions() {
        let records = vec![
            record(
                SelectionOutcome::Reject,
                &["t1", "t2"],
                &[0.0, 1.0],
                &[1.0, 1.0],
            ),
            record(SelectionOutcome::Select, &["t1"], &[0.5], &[0.0]),
        ];
        let digest = render_digest(
            &records,
            Path::new("decisions.jsonl"),
            Some("tree.candidate.json"),
            10,
        );
        assert!(digest.contains("Pending judgment (1)"));
        assert!(digest.contains("`t1` failed 3/4 runs (worst 0.00)"));
        assert!(digest.contains("Select"));
        assert!(digest.contains("Reject"));
    }

    #[test]
    fn digest_reports_worst_score_without_zero_floor() {
        let records = vec![record(SelectionOutcome::Reject, &["t1"], &[0.5], &[0.6])];
        let digest = render_digest(&records, Path::new("d.jsonl"), None, 10);
        assert!(digest.contains("`t1` failed 2/2 runs (worst 0.50)"));
    }

    #[test]
    fn digest_handles_empty_records() {
        let digest = render_digest(&[], Path::new("none.jsonl"), None, 10);
        assert!(digest.contains("No decision records"));
    }
}
