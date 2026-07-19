use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use zene_config::ZeneConfig;
use zene_core::{
    Agent, AgentEvent, EventHandler, PermissionGate, PermissionMode, PromptChoice, PromptOptions,
};
use zene_sandbox::LocalSandbox;
use zene_session::{list_sessions_for_workdir, SessionRecord};

use crate::sandbox_opts;
use super::protocol::{
    err_response, error_codes, is_notification, is_request, is_response, ok_response,
    prompt_text_from_params, RpcId,
};
use super::updates::{
    agent_message_chunk, plan_from_todo_arguments, replay_updates_from_messages,
    tool_call_result_update, tool_call_update, tool_kind, tool_title,
};

struct PendingResponse {
    tx: oneshot::Sender<Result<Value, Value>>,
}

struct SharedState {
    next_id: u64,
    pending: HashMap<String, PendingResponse>,
}

/// Tracks the tool call currently awaiting permission so ACP can reuse its id.
#[derive(Default)]
struct PendingToolCall {
    id: Option<String>,
}

#[derive(Clone)]
struct AcpWriter {
    tx: mpsc::UnboundedSender<String>,
    shared: Arc<Mutex<SharedState>>,
}

impl AcpWriter {
    fn send_raw(&self, line: String) -> Result<()> {
        self.tx
            .send(line)
            .map_err(|_| anyhow!("ACP stdout writer closed"))
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send_raw(
            json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            })
            .to_string(),
        )
    }

    fn session_update(&self, session_id: &str, update: Value) -> Result<()> {
        self.notify(
            "session/update",
            json!({
                "sessionId": session_id,
                "update": update,
            }),
        )
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut g = self.shared.lock().unwrap();
            g.next_id += 1;
            g.next_id
        };
        let id_key = id.to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut g = self.shared.lock().unwrap();
            g.pending.insert(id_key.clone(), PendingResponse { tx });
        }
        self.send_raw(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            })
            .to_string(),
        )?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(anyhow!("ACP client error: {e}")),
            Err(_) => Err(anyhow!("ACP client response channel closed")),
        }
    }
}

struct AcpSession {
    agent: Agent,
    cancel: Option<CancellationToken>,
    permission_mode: PermissionMode,
    /// Last tool call id observed via AgentEvent (used by permission prompts).
    pending_tool: Arc<Mutex<PendingToolCall>>,
}

pub struct AcpServer {
    workdir: PathBuf,
    yolo: bool,
    sessions: HashMap<String, AcpSession>,
    writer: AcpWriter,
}

/// Run the ACP stdio agent until stdin closes.
pub async fn run_acp(workdir: PathBuf, yolo: bool) -> Result<()> {
    AcpServer::run(workdir, yolo).await
}

impl AcpServer {
    async fn run(workdir: PathBuf, yolo: bool) -> Result<()> {
        let shared = Arc::new(Mutex::new(SharedState {
            next_id: 1,
            pending: HashMap::new(),
        }));
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let writer = AcpWriter {
            tx: out_tx,
            shared: Arc::clone(&shared),
        };

        let stdout_task = tokio::task::spawn_blocking(move || {
            let mut stdout = std::io::stdout().lock();
            while let Some(line) = out_rx.blocking_recv() {
                if writeln!(stdout, "{line}").is_err() {
                    break;
                }
                let _ = stdout.flush();
            }
        });

        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Value>();
        let shared_reader = Arc::clone(&shared);
        let stdin_task = tokio::task::spawn_blocking(move || {
            let stdin = std::io::stdin();
            let reader = BufReader::new(stdin.lock());
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(msg) => {
                        if is_response(&msg) {
                            let id = match &msg["id"] {
                                Value::Number(n) => n.to_string(),
                                Value::String(s) => s.clone(),
                                _ => continue,
                            };
                            let pending = {
                                let mut g = shared_reader.lock().unwrap();
                                g.pending.remove(&id)
                            };
                            if let Some(p) = pending {
                                if let Some(err) = msg.get("error") {
                                    let _ = p.tx.send(Err(err.clone()));
                                } else {
                                    let _ = p
                                        .tx
                                        .send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
                                }
                            }
                        } else if in_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("ACP: invalid JSON line: {e}");
                    }
                }
            }
        });

        let mut server = Self {
            workdir,
            yolo,
            sessions: HashMap::new(),
            writer,
        };

        while let Some(msg) = in_rx.recv().await {
            if is_notification(&msg) {
                let method = msg["method"].as_str().unwrap_or("");
                if method == "session/cancel" {
                    if let Some(sid) = msg["params"]["sessionId"].as_str() {
                        if let Some(s) = server.sessions.get(sid) {
                            if let Some(token) = &s.cancel {
                                token.cancel();
                            }
                        }
                    }
                }
                continue;
            }
            if !is_request(&msg) {
                continue;
            }
            let id = RpcId::from_value(&msg["id"]);
            let method = msg["method"].as_str().unwrap_or("").to_string();
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let reply = match server.dispatch(&method, params).await {
                Ok(result) => ok_response(id, result),
                Err(e) => {
                    warn!("ACP {method}: {e:#}");
                    err_response(id, dispatch_error_code(&method, &e), &format!("{e:#}"))
                }
            };
            if let Err(e) = server.writer.send_raw(reply.to_string()) {
                warn!("ACP write failed: {e}");
                break;
            }
        }

        for (_, mut sess) in server.sessions.drain() {
            let _ = sess.agent.shutdown().await;
        }
        drop(server.writer);
        let _ = stdin_task.await;
        let _ = stdout_task.await;
        Ok(())
    }

    async fn dispatch(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "initialize" => self.handle_initialize(params),
            "session/new" => self.handle_session_new(params).await,
            "session/load" => self.handle_session_load(params).await,
            "session/prompt" => self.handle_session_prompt(params).await,
            "session/list" => self.handle_session_list(params),
            "authenticate" => Ok(json!({})),
            other => Err(MethodNotFound(other.to_string()).into()),
        }
    }

    fn handle_initialize(&self, params: Value) -> Result<Value> {
        let client_version = params
            .get("protocolVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        if client_version != 1 {
            bail!("unsupported protocolVersion {client_version}; zene acp speaks 1");
        }
        Ok(json!({
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {
                    "image": false,
                    "audio": false,
                    "embeddedContext": true
                },
                "sessionCapabilities": {
                    "list": {}
                }
            },
            "agentInfo": {
                "name": "zene",
                "title": "Zene",
                "version": env!("CARGO_PKG_VERSION")
            },
            "authMethods": []
        }))
    }

    async fn handle_session_new(&mut self, params: Value) -> Result<Value> {
        let cwd = resolve_cwd(&params, &self.workdir)?;
        let session = SessionRecord::new(&cwd);
        let id = session.meta.id.clone();
        let acp_session = self.build_session(session, &cwd).await?;
        self.sessions.insert(id.clone(), acp_session);
        Ok(json!({ "sessionId": id }))
    }

    async fn handle_session_load(&mut self, params: Value) -> Result<Value> {
        let sid = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("sessionId required"))?
            .to_string();
        let cwd = resolve_cwd(&params, &self.workdir)?;
        let session = SessionRecord::load(&sid).context("load session")?;
        let updates = replay_updates_from_messages(&session.messages);
        let acp_session = self.build_session(session, &cwd).await?;
        self.sessions.insert(sid.clone(), acp_session);

        // ACP requires replaying history via session/update before responding.
        for update in updates {
            let mut update = update;
            if let Some(obj) = update.as_object_mut() {
                obj.insert(
                    "_meta".into(),
                    json!({ "isReplay": true }),
                );
            }
            self.writer.session_update(&sid, update)?;
        }

        Ok(json!({ "sessionId": sid }))
    }

    fn handle_session_list(&self, params: Value) -> Result<Value> {
        let cwd = params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.workdir.clone());
        let cwd = cwd
            .canonicalize()
            .unwrap_or(cwd);
        let sessions = list_sessions_for_workdir(&cwd).context("list sessions")?;
        let sessions: Vec<Value> = sessions
            .into_iter()
            .map(|meta| {
                json!({
                    "sessionId": meta.id,
                    "cwd": meta.workdir,
                    "title": meta.title,
                    "updatedAt": meta.updated_at.to_rfc3339(),
                })
            })
            .collect();
        Ok(json!({ "sessions": sessions }))
    }

    async fn build_session(&self, session: SessionRecord, cwd: &Path) -> Result<AcpSession> {
        let config = ZeneConfig::load(cwd).map_err(|err| anyhow!(err.to_string()))?;
        let permission_mode = if self.yolo {
            PermissionMode::BypassPermissions
        } else {
            PermissionMode::parse(&config.permission_mode)
        };
        let sandbox_opts = sandbox_opts::build_sandbox_options(&config, None);
        let sandbox = LocalSandbox::with_options(cwd, sandbox_opts)
            .await
            .context("initialize Keel execution layer")?;
        let agent = Agent::new(config, sandbox, session, permission_mode).await?;
        Ok(AcpSession {
            agent,
            cancel: None,
            permission_mode,
            pending_tool: Arc::new(Mutex::new(PendingToolCall::default())),
        })
    }

    async fn handle_session_prompt(&mut self, params: Value) -> Result<Value> {
        let sid = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("sessionId required"))?
            .to_string();
        let text = prompt_text_from_params(&params);
        if text.trim().is_empty() {
            bail!("empty prompt");
        }

        let writer = self.writer.clone();
        let yolo = self.yolo;
        let sess = self
            .sessions
            .get_mut(&sid)
            .ok_or_else(|| anyhow!("unknown sessionId: {sid}"))?;

        if sess.cancel.is_some() {
            bail!("session already has an active prompt; cancel it first");
        }

        let cancel = CancellationToken::new();
        sess.cancel = Some(cancel.clone());
        let pending_tool = Arc::clone(&sess.pending_tool);

        let permission_mode = sess.permission_mode;
        if !yolo {
            let writer_perm = writer.clone();
            let session_id = sid.clone();
            let mode_str = permission_mode.as_str().to_string();
            let pending_for_perm = Arc::clone(&pending_tool);
            let gate = PermissionGate::with_prompter(
                permission_mode,
                Box::new(move |tool_name, preview| {
                    let tool_call_id = pending_for_perm
                        .lock()
                        .unwrap()
                        .id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    acp_permission_prompt(
                        &writer_perm,
                        &session_id,
                        &mode_str,
                        tool_name,
                        preview,
                        &tool_call_id,
                    )
                }),
            );
            sess.agent.set_permission_gate(gate);
        }

        let prompt_id = uuid::Uuid::new_v4().to_string();
        let event_counter = Arc::new(Mutex::new(0u64));
        let on_event: EventHandler = {
            let writer = writer.clone();
            let session_id = sid.clone();
            let pending_tool = Arc::clone(&pending_tool);
            let prompt_id = prompt_id.clone();
            let event_counter = Arc::clone(&event_counter);
            Arc::new(move |event: AgentEvent| {
                let event_id = {
                    let mut n = event_counter.lock().unwrap();
                    *n += 1;
                    format!("{prompt_id}-{}", *n)
                };
                let meta = json!({
                    "promptId": prompt_id,
                    "eventId": event_id,
                    "isReplay": false,
                });
                match event {
                    AgentEvent::TextDelta { delta } => {
                        let mut update = agent_message_chunk(&delta);
                        attach_meta(&mut update, meta);
                        let _ = writer.session_update(&session_id, update);
                    }
                    AgentEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        pending_tool.lock().unwrap().id = Some(id.clone());
                        let mut update = tool_call_update(&id, &name, &arguments, "pending");
                        attach_meta(&mut update, meta);
                        let _ = writer.session_update(&session_id, update);
                        if name == "TodoWrite" {
                            if let Some(mut plan) = plan_from_todo_arguments(&arguments) {
                                attach_meta(
                                    &mut plan,
                                    json!({
                                        "promptId": prompt_id,
                                        "isReplay": false,
                                    }),
                                );
                                let _ = writer.session_update(&session_id, plan);
                            }
                        }
                    }
                    AgentEvent::ToolResult {
                        id,
                        name: _,
                        content,
                        is_error,
                        duration_ms,
                    } => {
                        let mut update = tool_call_result_update(&id, &content, is_error);
                        let mut meta = json!({
                            "promptId": prompt_id,
                            "eventId": event_id,
                            "isReplay": false,
                        });
                        if let Some(ms) = duration_ms {
                            meta.as_object_mut()
                                .unwrap()
                                .insert("durationMs".into(), json!(ms));
                        }
                        attach_meta(&mut update, meta);
                        let _ = writer.session_update(&session_id, update);
                        pending_tool.lock().unwrap().id = None;
                    }
                    AgentEvent::Error { message } => {
                        let mut update = agent_message_chunk(&format!("\n[error] {message}\n"));
                        attach_meta(&mut update, meta);
                        let _ = writer.session_update(&session_id, update);
                    }
                    _ => {}
                }
            })
        };

        let result = sess
            .agent
            .prompt(
                &text,
                PromptOptions {
                    stream: true,
                    cancel: Some(cancel.clone()),
                    event_handler: Some(on_event),
                    quiet: true,
                },
            )
            .await;

        sess.cancel = None;
        sess.pending_tool.lock().unwrap().id = None;
        let _ = sess.agent.session().save();

        match result {
            Ok(_) => {
                debug!(session = %sid, "ACP prompt completed");
                if cancel.is_cancelled() {
                    Ok(json!({ "stopReason": "cancelled" }))
                } else {
                    Ok(json!({ "stopReason": "end_turn" }))
                }
            }
            Err(err) => {
                if cancel.is_cancelled() || err.to_string().contains("aborted") {
                    Ok(json!({ "stopReason": "cancelled" }))
                } else {
                    Err(err)
                }
            }
        }
    }
}

fn attach_meta(update: &mut Value, meta: Value) {
    if let Some(obj) = update.as_object_mut() {
        obj.insert("_meta".into(), meta);
    }
}

fn resolve_cwd(params: &Value, fallback: &Path) -> Result<PathBuf> {
    let cwd = params
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.to_path_buf());
    cwd.canonicalize()
        .with_context(|| format!("invalid cwd: {}", cwd.display()))
}

fn acp_permission_prompt(
    writer: &AcpWriter,
    session_id: &str,
    permission_mode: &str,
    tool_name: &str,
    preview: &str,
    tool_call_id: &str,
) -> io::Result<PromptChoice> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Value, String>>();
    let writer = writer.clone();
    let session_id = session_id.to_string();
    let permission_mode = permission_mode.to_string();
    let tool = tool_name.to_string();
    let preview = preview.to_string();
    let tool_call_id = tool_call_id.to_string();
    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        let raw_input = serde_json::from_str::<Value>(&preview)
            .unwrap_or_else(|_| json!({ "preview": preview }));
        let result = writer
            .request(
                "session/request_permission",
                json!({
                    "sessionId": session_id,
                    "toolCall": {
                        "toolCallId": tool_call_id,
                        "title": tool_title(&tool, &preview),
                        "kind": tool_kind(&tool),
                        "status": "pending",
                        "rawInput": raw_input,
                    },
                    "options": [
                        {
                            "optionId": "allow-once",
                            "name": "Allow once",
                            "kind": "allow_once"
                        },
                        {
                            "optionId": "allow-always",
                            "name": "Allow always",
                            "kind": "allow_always"
                        },
                        {
                            "optionId": "reject-once",
                            "name": "Reject",
                            "kind": "reject_once"
                        }
                    ],
                    "permissionMode": permission_mode,
                }),
            )
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
    });

    // Avoid blocking a tokio worker while waiting for the client reply.
    let result = std::thread::spawn(move || rx.recv())
        .join()
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "permission thread panicked"))?
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;

    let option_id = result
        .pointer("/outcome/optionId")
        .or_else(|| result.get("optionId"))
        .and_then(|v| v.as_str())
        .unwrap_or("reject-once");

    Ok(match option_id {
        "allow-always" => PromptChoice::AllowSession,
        "allow-once" => PromptChoice::AllowOnce,
        _ => PromptChoice::Deny,
    })
}

#[derive(Debug)]
struct MethodNotFound(String);

impl std::fmt::Display for MethodNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "method not found: {}", self.0)
    }
}

impl std::error::Error for MethodNotFound {}

fn dispatch_error_code(method: &str, err: &anyhow::Error) -> i64 {
    if err.downcast_ref::<MethodNotFound>().is_some()
        || err.to_string().starts_with("method not found:")
    {
        return error_codes::METHOD_NOT_FOUND;
    }
    let msg = err.to_string();
    if msg.contains("required")
        || msg.contains("empty prompt")
        || msg.contains("invalid cwd")
        || msg.contains("unsupported protocolVersion")
        || msg.contains("unknown sessionId")
        || msg.contains("already has an active prompt")
    {
        return error_codes::INVALID_PARAMS;
    }
    let _ = method;
    error_codes::APPLICATION_ERROR
}
