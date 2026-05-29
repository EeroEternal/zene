use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::warn;
use zene_config::HookEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
}

impl HookEvent {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
struct HookInput<'a> {
    tool: &'a str,
    args: &'a str,
}

#[derive(Debug, Clone)]
pub struct HookBlock {
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct HookRunner {
    hooks: Vec<HookEntry>,
    workdir: PathBuf,
}

impl HookRunner {
    pub fn new(hooks: Vec<HookEntry>, workdir: PathBuf) -> Self {
        Self { hooks, workdir }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub async fn run_pre_tool_use(
        &self,
        tool: &str,
        args: &str,
    ) -> Result<Option<HookBlock>> {
        for hook in self.hooks_for(HookEvent::PreToolUse) {
            if let Some(block) = self.run_hook(hook, tool, args, true).await? {
                return Ok(Some(block));
            }
        }
        Ok(None)
    }

    pub async fn run_post_tool_use(&self, tool: &str, args: &str) {
        for hook in self.hooks_for(HookEvent::PostToolUse) {
            if let Err(err) = self.run_hook(hook, tool, args, false).await {
                warn!(tool, error = %err, "PostToolUse hook failed");
            }
        }
    }

    fn hooks_for(&self, event: HookEvent) -> impl Iterator<Item = &HookEntry> {
        self.hooks
            .iter()
            .filter(move |hook| HookEvent::parse(&hook.event) == Some(event))
    }

    async fn run_hook(
        &self,
        hook: &HookEntry,
        tool: &str,
        args: &str,
        blocking: bool,
    ) -> Result<Option<HookBlock>> {
        let payload = HookInput { tool, args };
        let input = serde_json::to_string(&payload).context("serialize hook input")?;

        let mut child = Command::new("bash")
            .arg("-c")
            .arg(&hook.command)
            .current_dir(&self.workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn hook command: {}", hook.command))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .await
                .context("write hook stdin")?;
            stdin.shutdown().await.context("close hook stdin")?;
        }

        let output = child
            .wait_with_output()
            .await
            .context("wait for hook command")?;

        if blocking && !output.status.success() {
            let reason = hook_failure_reason(&output.stderr, &output.stdout);
            return Ok(Some(HookBlock { reason }));
        }

        if !blocking && !output.status.success() {
            let reason = hook_failure_reason(&output.stderr, &output.stdout);
            warn!(
                event = hook.event,
                command = %hook.command,
                reason = %reason,
                "PostToolUse hook exited with non-zero status"
            );
        }

        Ok(None)
    }
}

fn hook_failure_reason(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    "hook exited with non-zero status".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn sample_hook(event: &str, script: &str) -> (TempDir, HookRunner) {
        let temp = TempDir::new().expect("tempdir");
        let runner = HookRunner::new(
            vec![HookEntry {
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
        let (_temp, runner) = sample_hook("PreToolUse", r#"echo "not allowed" >&2; exit 1"#);
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
        let (_temp, runner) = sample_hook("PostToolUse", "echo blocked >&2; exit 1");
        runner.run_post_tool_use("Read", "{}").await;
    }

    #[tokio::test]
    async fn hook_receives_json_on_stdin() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let temp = TempDir::new().expect("tempdir");
        let script = r#"read payload; echo "$payload" | grep -q '"tool":"Bash"' || exit 2"#;
        let runner = HookRunner::new(
            vec![HookEntry {
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

    #[test]
    fn hook_event_parsing() {
        assert_eq!(HookEvent::parse("PreToolUse"), Some(HookEvent::PreToolUse));
        assert_eq!(HookEvent::parse("PostToolUse"), Some(HookEvent::PostToolUse));
        assert_eq!(HookEvent::parse("Unknown"), None);
    }
}
