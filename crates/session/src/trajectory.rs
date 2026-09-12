//! Trajectory mining and distillation for continuous agent evolution.
//!
//! Analyzes append-only session event logs to extract repeatable patterns,
//! tool failures (tool churn), and human correction turns.
//! Adheres strictly to the invariant: Self-Evolution != Self-Authorization.
//! Patterns require >= 2 independent occurrences before being eligible for promotion.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{SessionEvent, SessionRecord};

/// Pattern of a repeated tool failure or struggle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolChurnPattern {
    pub tool_name: String,
    pub occurrences: usize,
    pub sample_arguments: Vec<String>,
    pub sample_errors: Vec<String>,
}

/// A detected human correction turn in conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionPattern {
    pub turn_id: String,
    pub user_prompt: String,
    pub prior_tool_call: Option<String>,
}

/// Distilled candidate findings from a single session trajectory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTrajectoryMiningReport {
    pub session_id: String,
    pub tool_churns: Vec<ToolChurnPattern>,
    pub corrections: Vec<CorrectionPattern>,
    pub attempt_failures: usize,
    pub compaction_count: usize,
}

/// Multi-session aggregator enforcing the Anti-Anecdote threshold (>= 2 sessions).
#[derive(Debug, Default)]
pub struct TrajectoryCorpusMiner {
    reports: Vec<SessionTrajectoryMiningReport>,
}

/// A verified lesson candidate ready for human review / PR promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonCandidate {
    pub pattern_key: String,
    pub session_count: usize,
    pub description: String,
    pub recommendation: String,
}

impl SessionTrajectoryMiningReport {
    /// Mine patterns from a single session record's event stream.
    pub fn mine_session(session: &SessionRecord) -> Self {
        let mut report = Self {
            session_id: session.meta.id.clone(),
            ..Default::default()
        };

        let mut current_tool_streak_name: Option<String> = None;
        let mut current_tool_streak_args: Vec<String> = Vec::new();
        let mut current_tool_streak_errors: Vec<String> = Vec::new();
        let mut last_tool_call_name: Option<String> = None;

        for event in &session.events {
            match event {
                SessionEvent::CompactionApplied { .. } => {
                    report.compaction_count += 1;
                }
                SessionEvent::AssistantAttemptFailed { .. } => {
                    report.attempt_failures += 1;
                }
                SessionEvent::ToolCall {
                    name, arguments, ..
                } => {
                    last_tool_call_name = Some(name.clone());
                    if let Some(streak_name) = &current_tool_streak_name {
                        if streak_name == name {
                            current_tool_streak_args.push(arguments.clone());
                        } else {
                            Self::flush_streak(
                                &mut report.tool_churns,
                                streak_name,
                                &current_tool_streak_args,
                                &current_tool_streak_errors,
                            );
                            current_tool_streak_name = Some(name.clone());
                            current_tool_streak_args = vec![arguments.clone()];
                            current_tool_streak_errors.clear();
                        }
                    } else {
                        current_tool_streak_name = Some(name.clone());
                        current_tool_streak_args = vec![arguments.clone()];
                        current_tool_streak_errors.clear();
                    }
                }
                SessionEvent::ToolResult {
                    is_error, content, ..
                } => {
                    if *is_error {
                        current_tool_streak_errors.push(content.clone());
                    }
                }
                SessionEvent::TurnStarted {
                    prompt, turn_id, ..
                } => {
                    // Check if this turn looks like a user correction
                    if is_correction_prompt(prompt) {
                        report.corrections.push(CorrectionPattern {
                            turn_id: turn_id.clone(),
                            user_prompt: prompt.clone(),
                            prior_tool_call: last_tool_call_name.clone(),
                        });
                    }
                }
                _ => {}
            }
        }

        if let Some(streak_name) = &current_tool_streak_name {
            Self::flush_streak(
                &mut report.tool_churns,
                streak_name,
                &current_tool_streak_args,
                &current_tool_streak_errors,
            );
        }

        report
    }

    fn flush_streak(
        churns: &mut Vec<ToolChurnPattern>,
        name: &str,
        args: &[String],
        errors: &[String],
    ) {
        // >= 3 consecutive calls with at least 1 failure marks a tool struggle/churn
        if args.len() >= 3 || !errors.is_empty() && args.len() >= 2 {
            churns.push(ToolChurnPattern {
                tool_name: name.to_string(),
                occurrences: args.len(),
                sample_arguments: args.iter().take(3).cloned().collect(),
                sample_errors: errors.iter().take(3).cloned().collect(),
            });
        }
    }
}

impl TrajectoryCorpusMiner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_session_report(&mut self, report: SessionTrajectoryMiningReport) {
        self.reports.push(report);
    }

    pub fn add_session(&mut self, session: &SessionRecord) {
        self.add_session_report(SessionTrajectoryMiningReport::mine_session(session));
    }

    /// Distill candidate lessons that satisfy the Anti-Anecdote threshold (>= 2 independent sessions).
    pub fn distill_candidates(&self) -> Vec<LessonCandidate> {
        let mut candidates = Vec::new();
        let mut tool_sessions: HashMap<String, usize> = HashMap::new();

        for report in &self.reports {
            let mut seen_tools_in_session = std::collections::HashSet::new();
            for churn in &report.tool_churns {
                if seen_tools_in_session.insert(churn.tool_name.clone()) {
                    *tool_sessions.entry(churn.tool_name.clone()).or_insert(0) += 1;
                }
            }
        }

        for (tool_name, count) in tool_sessions {
            if count >= 2 {
                candidates.push(LessonCandidate {
                    pattern_key: format!("tool_churn:{tool_name}"),
                    session_count: count,
                    description: format!(
                        "Tool `{tool_name}` caused repeated retry churn in {count} independent sessions."
                    ),
                    recommendation: format!(
                        "Consider creating a specialized skill or negative guardrail in `.agents/skills/` for `{tool_name}`."
                    ),
                });
            }
        }

        candidates
    }
}

/// Heuristic keywords indicating human correction of agent drift
fn is_correction_prompt(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let keywords = [
        "not what i asked",
        "don't",
        "do not",
        "stop",
        "revert",
        "wrong",
        "别动",
        "不对",
        "撤销",
        "不要改",
    ];
    keywords.iter().any(|kw| lower.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_mine_tool_churn_and_correction() {
        let mut session = SessionRecord::new(Path::new("."));
        session.record_turn_started("turn-1", "test the project");
        session.record_tool_call(Some("turn-1"), Some("step-1"), "t1", "Bash", "cargo test");
        session.record_tool_result(
            Some("turn-1"),
            Some("step-1"),
            "t1",
            "Bash",
            "error: failed",
            true,
            Some(10),
        );
        session.record_tool_call(
            Some("turn-1"),
            Some("step-2"),
            "t2",
            "Bash",
            "cargo test --fix",
        );
        session.record_tool_result(
            Some("turn-1"),
            Some("step-2"),
            "t2",
            "Bash",
            "error: failed",
            true,
            Some(10),
        );

        session.record_turn_started("turn-2", "不要改代码，只跑测试");

        let report = SessionTrajectoryMiningReport::mine_session(&session);
        assert_eq!(report.tool_churns.len(), 1);
        assert_eq!(report.tool_churns[0].tool_name, "Bash");
        assert_eq!(report.corrections.len(), 1);
        assert_eq!(
            report.corrections[0].prior_tool_call.as_deref(),
            Some("Bash")
        );
    }

    #[test]
    fn test_anti_anecdote_threshold_requires_two_sessions() {
        let mut miner = TrajectoryCorpusMiner::new();

        let mut session1 = SessionRecord::new(Path::new("."));
        session1.record_tool_call(None, None, "1", "Grep", "pattern1");
        session1.record_tool_result(None, None, "1", "Grep", "err", true, None);
        session1.record_tool_call(None, None, "2", "Grep", "pattern2");
        session1.record_tool_result(None, None, "2", "Grep", "err", true, None);

        miner.add_session(&session1);
        // Single session should NOT pass threshold
        assert!(miner.distill_candidates().is_empty());

        let mut session2 = SessionRecord::new(Path::new("."));
        session2.record_tool_call(None, None, "3", "Grep", "pattern3");
        session2.record_tool_result(None, None, "3", "Grep", "err", true, None);
        session2.record_tool_call(None, None, "4", "Grep", "pattern4");
        session2.record_tool_result(None, None, "4", "Grep", "err", true, None);

        miner.add_session(&session2);
        // Two independent sessions -> eligible candidate!
        let candidates = miner.distill_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pattern_key, "tool_churn:Grep");
        assert_eq!(candidates[0].session_count, 2);
    }
}
