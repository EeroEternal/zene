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
use crate::store::{AgentMeta, DataStore};
use crate::terminal::{is_terminal_request, TerminalHost, TerminalInfo};

const DEFAULT_MAX_MESSAGE_BYTES: usize = 1_048_576;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_path: Option<PathBuf>,
}

struct AgentRuntime {
    info: AgentInfo,
    stdin: Option<ChildStdin>,
    child: Child,
    journal: EventJournal,
    terminals: TerminalHost,
}

#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<RwLock<HashMap<String, Arc<Mutex<AgentRuntime>>>>>,
    command: PathBuf,
    command_args: Vec<String>,
    env: Vec<(String, String)>,
    store: Option<DataStore>,
    max_message_bytes: usize,
}

impl AgentManager {
    pub fn new(command: PathBuf, command_args: Vec<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            command,
            command_args,
            env: Vec::new(),
            store: None,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }

    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.env = env;
        self
    }

    pub fn with_store(mut self, store: DataStore) -> Self {
        self.store = Some(store);
        self
    }

    pub fn store(&self) -> Option<&DataStore> {
        self.store.as_ref()
    }

    pub async fn create(&self, workspace: PathBuf) -> Result<AgentInfo> {
        let workspace = canonicalize_workspace(&workspace)?;
        let agent_id = format!("agent_{}", Uuid::new_v4().simple());
        let journal = if let Some(store) = &self.store {
            let now = chrono::Utc::now();
            store.write_meta(&AgentMeta {
                agent_id: agent_id.clone(),
                workspace: workspace.clone(),
                created_at: now,
                updated_at: now,
            })?;
            EventJournal::with_persist(store.journal_path(&agent_id))
        } else {
            EventJournal::new()
        };
        self.spawn_agent(agent_id, workspace, journal, "agent_started")
            .await
    }

    pub async fn restart(&self, agent_id: &str) -> Result<AgentInfo> {
        let (workspace, journal) = {
            let mut map = self.inner.write().await;
            let runtime = map
                .remove(agent_id)
                .ok_or_else(|| anyhow!("agent not found"))?;
            let mut guard = runtime.lock().await;
            let workspace = guard.info.workspace.clone();
            let journal = guard.journal.clone();
            let _ = guard.child.start_kill();
            guard.stdin = None;
            drop(guard);
            (workspace, journal)
        };
        journal
            .append_system("agent_restarting", "respawning ACP child; journal retained")
            .await;
        if let Some(store) = &self.store {
            if let Ok(mut meta) = store.read_meta(agent_id) {
                meta.updated_at = chrono::Utc::now();
                let _ = store.write_meta(&meta);
            }
        }
        self.spawn_agent(agent_id.to_string(), workspace, journal, "agent_restarted")
            .await
    }

    pub async fn attach(&self, agent_id: &str) -> Result<AgentInfo> {
        if self.get(agent_id).await.is_some() {
            bail!("agent already attached in this gateway process");
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| anyhow!("persistence is disabled"))?;
        let meta = store.read_meta(agent_id)?;
        let workspace = canonicalize_workspace(&meta.workspace)?;
        let journal = EventJournal::load_from_file(&store.journal_path(agent_id))?;
        journal
            .append_system(
                "agent_attach",
                "reattaching persisted agent journal after gateway restart",
            )
            .await;
        self.spawn_agent(agent_id.to_string(), workspace, journal, "agent_started")
            .await
    }

    pub async fn list_persisted(&self) -> Result<Vec<AgentMeta>> {
        let Some(store) = &self.store else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for id in store.list_agent_ids()? {
            if let Ok(meta) = store.read_meta(&id) {
                out.push(meta);
            }
        }
        Ok(out)
    }

    pub async fn list_live(&self) -> Vec<AgentInfo> {
        let map = self.inner.read().await;
        let mut out = Vec::new();
        for runtime in map.values() {
            let guard = runtime.lock().await;
            out.push(guard.info.clone());
        }
        out.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        out
    }

    async fn spawn_agent(
        &self,
        agent_id: String,
        workspace: PathBuf,
        journal: EventJournal,
        start_kind: &str,
    ) -> Result<AgentInfo> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.command_args)
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let mut child = command.spawn().with_context(|| {
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
            workspace: workspace.clone(),
            state: AgentState::Running,
            pid: child.id(),
            created_at: chrono::Utc::now(),
            exit_code: None,
            journal_path: journal.persist_path().await,
        };

        let terminals = TerminalHost::new();
        let runtime = Arc::new(Mutex::new(AgentRuntime {
            info: info.clone(),
            stdin: Some(stdin),
            child,
            journal: journal.clone(),
            terminals,
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
                    Err(err) => {
                        let journal = {
                            let guard = stdout_runtime.lock().await;
                            guard.journal.clone()
                        };
                        journal
                            .append(serde_json::json!({
                                "type": "gateway.system",
                                "kind": "invalid_stdout",
                                "message": format!("invalid ACP JSON: {err}"),
                                "raw": line,
                            }))
                            .await;
                        continue;
                    }
                };

                if is_terminal_request(&payload) {
                    let runtime = stdout_runtime.clone();
                    tokio::spawn(async move {
                        let (journal, terminals, workspace) = {
                            let guard = runtime.lock().await;
                            (
                                guard.journal.clone(),
                                guard.terminals.clone(),
                                guard.info.workspace.clone(),
                            )
                        };
                        journal.append(payload.clone()).await;
                        match terminals
                            .handle_request(&payload, &journal, &workspace)
                            .await
                        {
                            Ok(response) => {
                                if let Err(err) = write_stdin_locked(&runtime, &[response]).await {
                                    journal
                                        .append_system(
                                            "terminal_reply_failed",
                                            format!("failed writing terminal response: {err}"),
                                        )
                                        .await;
                                }
                            }
                            Err(err) => {
                                let id = payload.get("id").cloned();
                                let response = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": {
                                        "code": -32000,
                                        "message": format!("terminal host error: {err}")
                                    }
                                });
                                let _ = write_stdin_locked(&runtime, &[response]).await;
                                journal
                                    .append_system("terminal_error", err.to_string())
                                    .await;
                            }
                        }
                    });
                    continue;
                }

                let journal = {
                    let guard = stdout_runtime.lock().await;
                    guard.journal.clone()
                };
                journal.append(payload).await;
            }
        });

        spawn_stderr_pump(agent_id.clone(), stderr);
        spawn_wait_pump(runtime.clone(), agent_id.clone());

        journal
            .append_system(
                start_kind,
                format!("ACP process ready for {}", workspace.display()),
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
        for message in messages {
            let raw = serde_json::to_vec(message)?;
            if raw.len() > self.max_message_bytes {
                bail!(
                    "message exceeds max size {} bytes (got {})",
                    self.max_message_bytes,
                    raw.len()
                );
            }
        }

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
        let (oldest, latest, count, bytes) = guard.journal.snapshot_meta().await;
        Some(AgentHealth {
            agent_id: guard.info.agent_id.clone(),
            state: guard.info.state,
            pid: guard.info.pid,
            exit_code: guard.info.exit_code,
            workspace: guard.info.workspace.clone(),
            oldest_cursor: oldest,
            latest_cursor: latest,
            event_count: count,
            journal_bytes: bytes,
            uptime_ms: (chrono::Utc::now() - guard.info.created_at)
                .num_milliseconds()
                .max(0) as u64,
            restartable: matches!(guard.info.state, AgentState::Exited | AgentState::Failed),
        })
    }

    pub async fn list_terminals(&self, agent_id: &str) -> Option<Vec<TerminalInfo>> {
        let map = self.inner.read().await;
        let runtime = map.get(agent_id)?.clone();
        let guard = runtime.lock().await;
        Some(guard.terminals.list(None).await)
    }

    pub async fn terminal_output(
        &self,
        agent_id: &str,
        terminal_id: &str,
        offset: usize,
    ) -> Result<(String, usize, Option<i32>)> {
        let map = self.inner.read().await;
        let runtime = map
            .get(agent_id)
            .cloned()
            .ok_or_else(|| anyhow!("agent not found"))?;
        let terminals = {
            let guard = runtime.lock().await;
            guard.terminals.clone()
        };
        let (chunk, len, code, _) = terminals.output_since(terminal_id, offset).await?;
        Ok((chunk, len, code))
    }

    pub async fn kill_terminal(&self, agent_id: &str, terminal_id: &str) -> Result<()> {
        let map = self.inner.read().await;
        let runtime = map
            .get(agent_id)
            .cloned()
            .ok_or_else(|| anyhow!("agent not found"))?;
        let terminals = {
            let guard = runtime.lock().await;
            guard.terminals.clone()
        };
        terminals.kill(terminal_id).await
    }
}

fn spawn_stderr_pump(agent_id: String, stderr: impl tokio::io::AsyncRead + Unpin + Send + 'static) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(agent_id = %agent_id, "acp stderr: {line}");
        }
    });
}

fn spawn_wait_pump(runtime: Arc<Mutex<AgentRuntime>>, agent_id: String) {
    tokio::spawn(async move {
        let exit = loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut guard = runtime.lock().await;
            match guard.child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(err) => break Err(err),
            }
        };

        let mut guard = runtime.lock().await;
        // Ignore wait updates after restart replaced this runtime.
        if guard.info.agent_id != agent_id {
            return;
        }
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
                        format!(
                            "ACP process {agent_id} exited with code {code:?}; POST .../restart to respawn"
                        ),
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
}

async fn write_stdin_locked(
    runtime: &Arc<Mutex<AgentRuntime>>,
    messages: &[Value],
) -> Result<()> {
    let mut guard = runtime.lock().await;
    let Some(stdin) = guard.stdin.as_mut() else {
        bail!("agent stdin closed");
    };
    for message in messages {
        let mut line = serde_json::to_string(message)?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
    }
    stdin.flush().await?;
    Ok(())
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
    pub journal_bytes: usize,
    pub uptime_ms: u64,
    pub restartable: bool,
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
