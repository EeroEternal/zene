use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::event_journal::EventJournal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    Starting,
    Running,
    Exited,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub agent_id: String,
    pub workspace: PathBuf,
    pub state: AgentState,
    pub pid: Option<u32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub exit_code: Option<i32>,
}

struct AgentRuntime {
    info: AgentInfo,
    stdin: Option<ChildStdin>,
    child: Child,
    journal: EventJournal,
}

#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<RwLock<HashMap<String, Arc<Mutex<AgentRuntime>>>>>,
    command: PathBuf,
    command_args: Vec<String>,
}

impl AgentManager {
    pub fn new(command: PathBuf, command_args: Vec<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            command,
            command_args,
        }
    }

    pub async fn create(&self, workspace: PathBuf) -> Result<AgentInfo> {
        let workspace = canonicalize_workspace(&workspace)?;
        let agent_id = format!("agent_{}", Uuid::new_v4().simple());
        let journal = EventJournal::new();

        let mut child = Command::new(&self.command)
            .args(&self.command_args)
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "failed to spawn ACP process {} {:?}",
                    self.command.display(),
                    self.command_args
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("ACP process stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("ACP process stdout missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("ACP process stderr missing"))?;

        let info = AgentInfo {
            agent_id: agent_id.clone(),
            workspace,
            state: AgentState::Running,
            pid: child.id(),
            created_at: chrono::Utc::now(),
            exit_code: None,
        };

        let runtime = Arc::new(Mutex::new(AgentRuntime {
            info: info.clone(),
            stdin: Some(stdin),
            child,
            journal: journal.clone(),
        }));

        {
            let mut map = self.inner.write().await;
            map.insert(agent_id.clone(), runtime.clone());
        }

        let stdout_runtime = runtime.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let payload = match serde_json::from_str::<Value>(&line) {
                    Ok(value) => value,
                    Err(err) => serde_json::json!({
                        "type": "gateway.system",
                        "kind": "invalid_stdout",
                        "message": format!("invalid ACP JSON: {err}"),
                        "raw": line,
                    }),
                };
                let journal = {
                    let guard = stdout_runtime.lock().await;
                    guard.journal.clone()
                };
                journal.append(payload).await;
            }
        });

        let stderr_agent = agent_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(agent_id = %stderr_agent, "acp stderr: {line}");
            }
        });

        let wait_runtime = runtime.clone();
        let wait_agent = agent_id.clone();
        tokio::spawn(async move {
            let exit = loop {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let mut guard = wait_runtime.lock().await;
                match guard.child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => {}
                    Err(err) => break Err(err),
                }
            };

            let mut guard = wait_runtime.lock().await;
            match exit {
                Ok(status) => {
                    guard.info.state = if status.success() {
                        AgentState::Exited
                    } else {
                        AgentState::Failed
                    };
                    guard.info.exit_code = status.code();
                    guard.stdin = None;
                    let code = status.code();
                    let journal = guard.journal.clone();
                    drop(guard);
                    journal
                        .append_system(
                            "process_exit",
                            format!("ACP process {wait_agent} exited with code {code:?}"),
                        )
                        .await;
                }
                Err(err) => {
                    guard.info.state = AgentState::Failed;
                    guard.stdin = None;
                    let journal = guard.journal.clone();
                    drop(guard);
                    journal
                        .append_system("process_wait_error", format!("failed to wait: {err}"))
                        .await;
                }
            }
        });

        journal
            .append_system(
                "agent_started",
                format!("ACP process started for {}", info.workspace.display()),
            )
            .await;

        Ok(info)
    }

    pub async fn get(&self, agent_id: &str) -> Option<AgentInfo> {
        let map = self.inner.read().await;
        let runtime = map.get(agent_id)?.clone();
        let guard = runtime.lock().await;
        Some(guard.info.clone())
    }

    pub async fn journal(&self, agent_id: &str) -> Option<EventJournal> {
        let map = self.inner.read().await;
        let runtime = map.get(agent_id)?.clone();
        let guard = runtime.lock().await;
        Some(guard.journal.clone())
    }

    pub async fn write_messages(&self, agent_id: &str, messages: &[Value]) -> Result<usize> {
        let map = self.inner.read().await;
        let runtime = map
            .get(agent_id)
            .cloned()
            .ok_or_else(|| anyhow!("agent not found"))?;
        drop(map);

        let mut guard = runtime.lock().await;
        if !matches!(guard.info.state, AgentState::Running | AgentState::Starting) {
            bail!("agent is not running");
        }
        let Some(stdin) = guard.stdin.as_mut() else {
            bail!("agent stdin closed");
        };
        let mut written = 0usize;
        for message in messages {
            let mut line = serde_json::to_string(message)?;
            line.push('\n');
            stdin
                .write_all(line.as_bytes())
                .await
                .context("failed writing to ACP stdin")?;
            written += 1;
        }
        stdin.flush().await.context("failed flushing ACP stdin")?;
        Ok(written)
    }

    pub async fn health(&self, agent_id: &str) -> Option<AgentHealth> {
        let map = self.inner.read().await;
        let runtime = map.get(agent_id)?.clone();
        let guard = runtime.lock().await;
        let (oldest, latest, count) = guard.journal.snapshot_meta().await;
        Some(AgentHealth {
            agent_id: guard.info.agent_id.clone(),
            state: guard.info.state,
            pid: guard.info.pid,
            exit_code: guard.info.exit_code,
            workspace: guard.info.workspace.clone(),
            oldest_cursor: oldest,
            latest_cursor: latest,
            event_count: count,
            uptime_ms: (chrono::Utc::now() - guard.info.created_at).num_milliseconds().max(0)
                as u64,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHealth {
    pub agent_id: String,
    pub state: AgentState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub workspace: PathBuf,
    pub oldest_cursor: Option<u64>,
    pub latest_cursor: u64,
    pub event_count: usize,
    pub uptime_ms: u64,
}

pub fn resolve_zene_bin(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    if let Ok(path) = std::env::var("ZENE_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("zene");
        if sibling.exists() {
            return sibling;
        }
        #[cfg(windows)]
        {
            let sibling = exe.with_file_name("zene.exe");
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from("zene")
}

fn canonicalize_workspace(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        bail!("workspace does not exist: {}", path.display());
    }
    if !path.is_dir() {
        bail!("workspace is not a directory: {}", path.display());
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    Ok(canonical)
}

/// Best-effort wait helper used by tests.
pub async fn wait_until<F, Fut>(timeout: std::time::Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    false
}
