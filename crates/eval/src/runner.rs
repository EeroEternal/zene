//! Paired episode runner (docs/harness-evolution.md §3.4).
//!
//! The only variable between the two runs of a task is the harness: both runs
//! use fresh workdirs rendered from their harness snapshot and the same
//! executor with fixed model settings. Runs are sequential in fixed order
//! (incumbent first) — part of the determinism contract. Paired score sets
//! feed [`crate::decide_win_margin`] / [`crate::decide_floor`].

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{EpisodeRun, EpisodeScorer};

/// One evaluation task: `id` keys the scorer's fixtures, `prompt` is the run
/// input. Method-side data (expected answers, rubrics) stays in the scorer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeTask {
    pub id: String,
    pub prompt: String,
}

/// Runs one episode (task under one rendered harness) to completion and
/// returns the run's evidence.
#[async_trait]
pub trait EpisodeExecutor: Send + Sync {
    async fn run(&self, task: &EpisodeTask, workdir: &Path) -> Result<EpisodeRun>;
}

/// Paired score sets aligned with [`PairedEpisodeOutcome::task_ids`].
#[derive(Debug, Clone, Serialize)]
pub struct PairedEpisodeOutcome {
    pub task_ids: Vec<String>,
    pub candidate_scores: Vec<f64>,
    pub incumbent_scores: Vec<f64>,
}

/// Run every task twice — incumbent harness first, then candidate — score both
/// runs and return paired score sets.
pub async fn run_paired_episodes(
    tasks: &[EpisodeTask],
    incumbent_harness: &Path,
    candidate_harness: &Path,
    run_root: &Path,
    executor: &dyn EpisodeExecutor,
    scorer: &dyn EpisodeScorer,
) -> Result<PairedEpisodeOutcome> {
    let mut outcome = PairedEpisodeOutcome {
        task_ids: Vec::new(),
        candidate_scores: Vec::new(),
        incumbent_scores: Vec::new(),
    };
    for task in tasks {
        let incumbent_dir = run_root.join(&task.id).join("incumbent");
        let candidate_dir = run_root.join(&task.id).join("candidate");
        prepare_workdir(incumbent_harness, &incumbent_dir)?;
        prepare_workdir(candidate_harness, &candidate_dir)?;

        let incumbent_run = executor
            .run(task, &incumbent_dir)
            .await
            .with_context(|| format!("run task `{}` under incumbent harness", task.id))?;
        let candidate_run = executor
            .run(task, &candidate_dir)
            .await
            .with_context(|| format!("run task `{}` under candidate harness", task.id))?;

        outcome.task_ids.push(task.id.clone());
        outcome
            .incumbent_scores
            .push(scorer.score(&task.id, &incumbent_run)?);
        outcome
            .candidate_scores
            .push(scorer.score(&task.id, &candidate_run)?);
    }
    Ok(outcome)
}

/// Fresh episode workdir with the harness rendered in. Loading is filesystem
/// convention (`AGENTS.md`, `.agents/skills/`, `.zene/config.toml`), so
/// rendering is a tree copy ("关键懒点", docs/harness-evolution.md §3.1).
fn prepare_workdir(harness_dir: &Path, workdir: &Path) -> Result<()> {
    if !harness_dir.is_dir() {
        bail!("harness dir not found: {}", harness_dir.display());
    }
    if workdir.exists() {
        fs::remove_dir_all(workdir)
            .with_context(|| format!("clean stale workdir {}", workdir.display()))?;
    }
    fs::create_dir_all(workdir).with_context(|| format!("create workdir {}", workdir.display()))?;
    copy_tree(harness_dir, workdir)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let entries =
        fs::read_dir(from).with_context(|| format!("read harness dir {}", from.display()))?;
    for entry in entries {
        let entry = entry.context("read harness entry")?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type().context("stat harness entry")?;
        if kind.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("create harness dir {}", target.display()))?;
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .with_context(|| format!("copy harness file {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Built-in scorer: exact final-answer comparison (tutorial-grade, see
/// docs/harness-evolution.md §3.3). Scores `1.0` when the run's final text
/// trims to the fixture's expected answer, else `0.0`.
#[derive(Debug, Default, Clone)]
pub struct ExactAnswerScorer {
    expected: BTreeMap<String, String>,
}

impl ExactAnswerScorer {
    pub fn new(expected: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            expected: expected.into_iter().collect(),
        }
    }
}

impl EpisodeScorer for ExactAnswerScorer {
    fn score(&self, task_id: &str, run: &EpisodeRun) -> Result<f64> {
        let expected = self
            .expected
            .get(task_id)
            .with_context(|| format!("no expected-answer fixture for task `{task_id}`"))?;
        Ok(if run.final_text.trim() == expected.trim() {
            1.0
        } else {
            0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use zene_session::RecordEntry;

    /// Scripted executor: answers per (task, harness side) and call log.
    struct FixedExecutor {
        answers: BTreeMap<String, (String, String)>,
        calls: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl EpisodeExecutor for FixedExecutor {
        async fn run(&self, task: &EpisodeTask, workdir: &Path) -> Result<EpisodeRun> {
            let side = workdir
                .file_name()
                .expect("workdir leaf")
                .to_string_lossy()
                .into_owned();
            self.calls
                .lock()
                .unwrap()
                .push((task.id.clone(), side.clone()));
            assert!(
                workdir.join("AGENTS.md").is_file(),
                "harness must be rendered into the workdir"
            );
            let pair = self.answers.get(&task.id).expect("fixture for task");
            let final_text = if side == "candidate" {
                pair.1.clone()
            } else {
                pair.0.clone()
            };
            Ok(EpisodeRun {
                final_text,
                trajectory: vec![RecordEntry::TurnPrompt {
                    turn_id: task.id.clone(),
                    prompt: task.prompt.clone(),
                    ts: chrono::Utc::now(),
                }],
            })
        }
    }

    fn harness(base: &Path, name: &str, rules: &str) -> std::path::PathBuf {
        let dir = base.join(name);
        fs::create_dir_all(dir.join(".agents/skills/demo")).unwrap();
        fs::write(dir.join("AGENTS.md"), rules).unwrap();
        fs::write(dir.join(".agents/skills/demo/SKILL.md"), "demo").unwrap();
        dir
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paired_runs_score_aligned_and_incumbent_first() {
        let base = tempfile::tempdir().unwrap();
        let incumbent = harness(base.path(), "harness-a", "rules A");
        let candidate = harness(base.path(), "harness-b", "rules B");

        let executor = FixedExecutor {
            answers: [("t1", ("wrong", "right")), ("t2", ("right", "right"))]
                .into_iter()
                .map(|(k, (a, b))| (k.to_string(), (a.to_string(), b.to_string())))
                .collect(),
            calls: Mutex::new(Vec::new()),
        };
        let scorer = ExactAnswerScorer::new([
            ("t1".to_string(), "right".to_string()),
            ("t2".to_string(), "right".to_string()),
        ]);
        let tasks = vec![
            EpisodeTask {
                id: "t1".into(),
                prompt: "p1".into(),
            },
            EpisodeTask {
                id: "t2".into(),
                prompt: "p2".into(),
            },
        ];
        let run_root = base.path().join("runs");
        let outcome = run_paired_episodes(
            &tasks, &incumbent, &candidate, &run_root, &executor, &scorer,
        )
        .await
        .unwrap();

        assert_eq!(outcome.task_ids, vec!["t1", "t2"]);
        assert_eq!(outcome.incumbent_scores, vec![0.0, 1.0]);
        assert_eq!(outcome.candidate_scores, vec![1.0, 1.0]);
        let calls = executor.calls.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            [
                ("t1".to_string(), "incumbent".to_string()),
                ("t1".to_string(), "candidate".to_string()),
                ("t2".to_string(), "incumbent".to_string()),
                ("t2".to_string(), "candidate".to_string()),
            ]
        );
        // workdirs persist for inspection, harness fully rendered
        assert!(run_root.join("t1/incumbent/AGENTS.md").is_file());
        assert!(run_root
            .join("t2/candidate/.agents/skills/demo/SKILL.md")
            .is_file());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_workdirs_are_rebuilt() {
        let base = tempfile::tempdir().unwrap();
        let incumbent = harness(base.path(), "harness-a", "rules A");
        let candidate = harness(base.path(), "harness-b", "rules B");
        let run_root = base.path().join("runs");
        let stale = run_root.join("t1/candidate/leftover.txt");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, "from a previous eval").unwrap();

        let executor = FixedExecutor {
            answers: [("t1", ("a", "a"))]
                .into_iter()
                .map(|(k, (x, y))| (k.to_string(), (x.to_string(), y.to_string())))
                .collect(),
            calls: Mutex::new(Vec::new()),
        };
        let scorer = ExactAnswerScorer::new([("t1".to_string(), "a".to_string())]);
        let tasks = vec![EpisodeTask {
            id: "t1".into(),
            prompt: "p".into(),
        }];
        run_paired_episodes(
            &tasks, &incumbent, &candidate, &run_root, &executor, &scorer,
        )
        .await
        .unwrap();
        assert!(!stale.exists(), "stale files must not leak across evals");
    }

    #[test]
    fn exact_answer_scorer_trims_and_compares() {
        let scorer = ExactAnswerScorer::new([("t".to_string(), "42".to_string())]);
        let run = |text: &str| EpisodeRun {
            final_text: text.to_string(),
            trajectory: Vec::new(),
        };
        assert_eq!(scorer.score("t", &run(" 42 ")).unwrap(), 1.0);
        assert_eq!(scorer.score("t", &run("41")).unwrap(), 0.0);
        assert!(scorer.score("missing", &run("42")).is_err());
    }
}
