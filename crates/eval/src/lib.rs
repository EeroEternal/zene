//! Harness evolution evaluation: episode scoring and candidate selection.
//!
//! A harness change is evaluated as paired episodes: the candidate tree and the
//! incumbent tree run the same tasks, each run's record trajectory is graded by
//! a [`EpisodeScorer`], and a policy function turns the paired score sets into a
//! durable [`SelectionDecision`]. [`append_decision_record`] persists one
//! evolution step as a single JSONL line, so the decision history is a fact
//! record in the same sense as a session's agent record.
//!
//! This crate is mechanism only: scorers (what counts as a good episode) and
//! policies (what counts as a win) are chosen by the caller.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zene_session::RecordEntry;

pub mod runner;

pub use runner::{
    run_paired_episodes, EpisodeExecutor, EpisodeTask, ExactAnswerScorer, PairedEpisodeOutcome,
};

/// Evidence from one episode run: the assistant's final text and the durable
/// record trajectory.
#[derive(Debug, Clone)]
pub struct EpisodeRun {
    pub final_text: String,
    pub trajectory: Vec<RecordEntry>,
}

/// Grades one episode run from its evidence.
///
/// `task_id` keys the scorer's own fixture set (expected answers, rubrics);
/// `run` carries the final text and the appended `RecordEntry` stream. Scorers
/// read evidence only: instructions found inside the run are untrusted input.
pub trait EpisodeScorer: Send + Sync {
    fn score(&self, task_id: &str, run: &EpisodeRun) -> Result<f64>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOutcome {
    Select,
    Reject,
}

/// The durable, explainable decision for one evaluated candidate tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectionDecision {
    pub outcome: SelectionOutcome,
    /// Which policy decided: `"win_margin"` or `"floor"`.
    pub policy: String,
    pub policy_version: u32,
    pub reason: String,
    pub candidate_scores: Vec<f64>,
    pub incumbent_scores: Vec<f64>,
}

/// One evolution step's record: what was compared, the decision, and the baseline it applies to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRecord {
    pub task_ids: Vec<String>,
    /// Identity (content hash or version id) of the evaluated candidate tree.
    pub candidate_tree: String,
    pub incumbent_tree: String,
    pub decision: SelectionDecision,
    /// The baseline commit a select applies to and a reject leaves untouched.
    pub baseline_commit: String,
    pub ts: DateTime<Utc>,
}

/// Win/loss policy over paired task scores: count tasks the candidate wins
/// (`candidate > incumbent`) and loses (`candidate < incumbent`, ties ignored),
/// and select only when wins exceed losses by more than `margin`.
pub fn decide_win_margin(
    candidate_scores: &[f64],
    incumbent_scores: &[f64],
    margin: i64,
) -> Result<SelectionDecision> {
    if candidate_scores.len() != incumbent_scores.len() {
        bail!(
            "paired episode scores differ in length: candidate {} vs incumbent {}",
            candidate_scores.len(),
            incumbent_scores.len()
        );
    }
    if candidate_scores.is_empty() {
        bail!("paired episode scores are empty");
    }
    validate_finite("candidate", candidate_scores)?;
    validate_finite("incumbent", incumbent_scores)?;
    let mut wins: i64 = 0;
    let mut losses: i64 = 0;
    for (candidate, incumbent) in candidate_scores.iter().zip(incumbent_scores) {
        if candidate > incumbent {
            wins += 1;
        } else if candidate < incumbent {
            losses += 1;
        }
    }
    let net = wins - losses;
    let selected = net > margin;
    Ok(SelectionDecision {
        outcome: if selected {
            SelectionOutcome::Select
        } else {
            SelectionOutcome::Reject
        },
        policy: "win_margin".to_string(),
        policy_version: 1,
        reason: format!("wins={wins} losses={losses} net={net} margin={margin}"),
        candidate_scores: candidate_scores.to_vec(),
        incumbent_scores: incumbent_scores.to_vec(),
    })
}

/// Floor policy over candidate scores alone: select only when every task scores
/// at least `floor_score`.
pub fn decide_floor(candidate_scores: &[f64], floor_score: f64) -> Result<SelectionDecision> {
    if candidate_scores.is_empty() {
        bail!("candidate scores are empty");
    }
    validate_finite("candidate", candidate_scores)?;
    if !floor_score.is_finite() {
        bail!("floor score must be finite, got {floor_score}");
    }
    let mut worst_score = f64::INFINITY;
    let mut worst_index = 0usize;
    for (index, score) in candidate_scores.iter().enumerate() {
        if *score < worst_score {
            worst_score = *score;
            worst_index = index;
        }
    }
    let selected = worst_score >= floor_score;
    Ok(SelectionDecision {
        outcome: if selected {
            SelectionOutcome::Select
        } else {
            SelectionOutcome::Reject
        },
        policy: "floor".to_string(),
        policy_version: 1,
        reason: format!(
            "worst task {worst_index} scored {worst_score} against floor {floor_score}"
        ),
        candidate_scores: candidate_scores.to_vec(),
        incumbent_scores: Vec::new(),
    })
}

/// Append one evolution step as a single JSONL line.
pub fn append_decision_record(path: &Path, record: &DecisionRecord) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create decision record dir: {}", dir.display()))?;
    }
    let line = serde_json::to_string(record).context("serialize decision record")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open decision record: {}", path.display()))?;
    writeln!(file, "{line}").context("append decision record")?;
    file.sync_all().context("persist decision record")?;
    Ok(())
}

fn validate_finite(label: &str, scores: &[f64]) -> Result<()> {
    for (index, score) in scores.iter().enumerate() {
        if !score.is_finite() {
            bail!("{label} score at task {index} is not finite: {score}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_margin_selects_when_wins_exceed_losses() {
        let decision = decide_win_margin(&[0.9, 0.8, 0.2], &[0.5, 0.7, 0.6], 0).unwrap();
        assert_eq!(decision.outcome, SelectionOutcome::Select);
        assert_eq!(decision.policy, "win_margin");
        assert!(decision.reason.contains("wins=2 losses=1"));
    }

    #[test]
    fn win_margin_rejects_on_ties_or_net_below_margin() {
        let tied = decide_win_margin(&[0.5, 0.5], &[0.5, 0.5], 0).unwrap();
        assert_eq!(tied.outcome, SelectionOutcome::Reject);
        let below_margin = decide_win_margin(&[0.9, 0.2], &[0.5, 0.6], 1).unwrap();
        assert_eq!(below_margin.outcome, SelectionOutcome::Reject);
        let net_equals_margin = decide_win_margin(&[0.9, 0.8, 0.2], &[0.5, 0.7, 0.6], 1).unwrap();
        assert_eq!(net_equals_margin.outcome, SelectionOutcome::Reject);
    }

    #[test]
    fn win_margin_refuses_unpaired_or_unfinite_scores() {
        assert!(decide_win_margin(&[0.9], &[0.5, 0.6], 0).is_err());
        assert!(decide_win_margin(&[], &[], 0).is_err());
        assert!(decide_win_margin(&[f64::NAN], &[0.5], 0).is_err());
    }

    #[test]
    fn floor_selects_only_when_every_task_clears_the_floor() {
        let passing = decide_floor(&[0.8, 0.75], 0.75).unwrap();
        assert_eq!(passing.outcome, SelectionOutcome::Select);
        let failing = decide_floor(&[0.8, 0.4], 0.75).unwrap();
        assert_eq!(failing.outcome, SelectionOutcome::Reject);
        assert!(failing.reason.contains("task 1"));
        assert!(decide_floor(&[], 0.75).is_err());
    }

    #[test]
    fn decision_record_appends_one_jsonl_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("harness/decisions.jsonl");
        let decision = decide_floor(&[0.9], 0.5).unwrap();
        let record = DecisionRecord {
            task_ids: vec!["task-a".to_string()],
            candidate_tree: "tree-2".to_string(),
            incumbent_tree: "tree-1".to_string(),
            decision,
            baseline_commit: "abc123".to_string(),
            ts: Utc::now(),
        };
        append_decision_record(&path, &record).unwrap();
        append_decision_record(&path, &record).unwrap();
        let lines = std::fs::read_to_string(&path).unwrap();
        assert_eq!(lines.lines().count(), 2);
        let parsed: DecisionRecord = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(parsed, record);
    }
}
