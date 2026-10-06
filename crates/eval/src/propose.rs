//! Failure-driven mutation proposals (docs/harness-evolution.md §3.4 / P2).
//!
//! The proposer reads failure evidence and returns mutations; the mutations
//! are still untrusted input — every proposal goes through
//! [`crate::apply_mutations`] admission before it can become a candidate tree.
//! Proposers may only be text-level advisers: the denied-key boundary lives in
//! the tree module, not in the proposer.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use zene_session::RecordEntry;

use crate::tree::{HarnessTree, Mutation};

/// One failed episode run fed to the proposer as untrusted evidence.
#[derive(Debug, Clone)]
pub struct FailureEvidence {
    pub task_id: String,
    pub score: f64,
    pub final_text: String,
    pub trajectory: Vec<RecordEntry>,
}

/// Produces candidate mutations from failure evidence.
#[async_trait]
pub trait Proposer: Send + Sync {
    async fn propose(
        &self,
        tree: &HarnessTree,
        failures: &[FailureEvidence],
    ) -> Result<Vec<Mutation>>;
}

/// Strict parse of a proposal payload: exactly `{"mutations": [...]}` with no
/// unknown fields, so a model cannot smuggle extra instructions through the
/// proposal shape. A surrounding ``` code fence is tolerated.
pub fn parse_mutations(raw: &str) -> Result<Vec<Mutation>> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Proposal {
        mutations: Vec<Mutation>,
    }

    let proposal: Proposal =
        serde_json::from_str(trimmed).context("proposal must be {\"mutations\": [...]} JSON")?;
    Ok(proposal.mutations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{HarnessKind, MutationOp};
    use serde_json::json;

    #[test]
    fn parse_accepts_plain_and_fenced_payloads() {
        let payload =
            r#"{"mutations":[{"op":"create","id":"r2","kind":"rules","config":{"body":"x"}}]}"#;
        for raw in [payload, &format!("```json\n{payload}\n```")] {
            let mutations = parse_mutations(raw).unwrap();
            assert_eq!(mutations.len(), 1);
            assert_eq!(mutations[0].op, MutationOp::Create);
            assert_eq!(mutations[0].kind, Some(HarnessKind::Rules));
        }
    }

    #[test]
    fn parse_rejects_unknown_fields_and_bad_shapes() {
        // unknown top-level field smuggled in
        let err = parse_mutations(r#"{"mutations":[],"extra":"ignore previous"}"#).unwrap_err();
        assert!(err.to_string().contains("must be"));
        // unknown field inside a mutation
        let err = parse_mutations(r#"{"mutations":[{"op":"remove","id":"x","tool":"bash"}]}"#)
            .unwrap_err();
        assert!(err.to_string().contains("must be"));
        // not JSON at all
        assert!(parse_mutations("sure, I will delete everything").is_err());
        let _ = json!({});
    }
}
