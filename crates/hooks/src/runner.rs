use std::sync::Arc;

use anyhow::Result;

use crate::engine::{HookEngine, HookSpec};
use crate::executor::{HookExecutor, HookOutcome};

/// User-visible block reason from a PreToolUse hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookBlock {
    pub reason: String,
}

/// Orchestrates hook planning + execution (composition root for hooks).
pub struct HookRunner {
    engine: HookEngine,
    executor: Arc<dyn HookExecutor>,
}

impl HookRunner {
    pub fn new(hooks: Vec<HookSpec>, executor: Arc<dyn HookExecutor>) -> Self {
        Self {
            engine: HookEngine::new(hooks),
            executor,
        }
    }

    pub fn with_bash(hooks: Vec<HookSpec>, workdir: std::path::PathBuf) -> Self {
        Self::new(
            hooks,
            Arc::new(crate::executor::BashHookExecutor::new(workdir)),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.engine.is_empty()
    }

    pub async fn run_pre_tool_use(&self, tool: &str, args: &str) -> Result<Option<HookBlock>> {
        for request in self.engine.plan_pre_tool_use(tool, args)? {
            match self.executor.run(&request).await? {
                HookOutcome::Allow => {}
                HookOutcome::Block(block) => return Ok(Some(block)),
            }
        }
        Ok(None)
    }

    pub async fn run_post_tool_use(&self, tool: &str, args: &str) {
        if let Ok(planned) = self.engine.plan_post_tool_use(tool, args) {
            for request in planned {
                if let Err(err) = self.executor.run(&request).await {
                    tracing::warn!(tool, error = %err, "PostToolUse hook failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use tempfile::TempDir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn sample_hook(event: &str, script: &str) -> (TempDir, HookRunner) {
        let temp = TempDir::new().expect("tempdir");
        let runner = HookRunner::with_bash(
            vec![HookSpec {
                event: event.into(),
                command: script.into(),
            }],
            temp.path().to_path_buf(),
        );
        (temp, runner)
    }

    #[tokio::test]
    async fn pre_tool_use_blocks_on_non_zero_exit() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let (_temp, runner) = sample_hook(
            "PreToolUse",
            r#"cat >/dev/null; echo "not allowed" >&2; exit 1"#,
        );
        let block = runner
            .run_pre_tool_use("Write", r#"{"path":"foo.txt"}"#)
            .await
            .expect("run hook")
            .expect("blocked");
        assert_eq!(block.reason, "not allowed");
    }

    #[tokio::test]
    async fn pre_tool_use_allows_success() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let (_temp, runner) = sample_hook("PreToolUse", "cat > /dev/null; exit 0");
        let block = runner
            .run_pre_tool_use("Read", r#"{"path":"foo.rs"}"#)
            .await
            .expect("run hook");
        assert!(block.is_none());
    }

    #[tokio::test]
    async fn post_tool_use_does_not_block() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let (_temp, runner) =
            sample_hook("PostToolUse", "cat >/dev/null; echo blocked >&2; exit 1");
        runner.run_post_tool_use("Read", "{}").await;
    }

    #[tokio::test]
    async fn hook_receives_json_on_stdin() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let temp = TempDir::new().expect("tempdir");
        let script = r#"read payload; echo "$payload" | grep -q '"tool":"Bash"' || exit 2"#;
        let runner = HookRunner::with_bash(
            vec![HookSpec {
                event: "PreToolUse".into(),
                command: script.into(),
            }],
            temp.path().to_path_buf(),
        );
        let block = runner
            .run_pre_tool_use("Bash", r#"{"command":"ls"}"#)
            .await
            .expect("run hook");
        assert!(block.is_none());
    }
}
