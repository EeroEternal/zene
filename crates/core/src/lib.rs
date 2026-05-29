use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zene_config::ZeneConfig;
use zene_llm::{ChatClient, ChatRequest, Message, StreamEvent, TokenUsage, ToolCall};
use zene_sandbox::LocalSandbox;
use zene_session::{AgentRecordWriter, RecordEntry, SessionRecord};
use zene_tools::{
    shared_plan_mode, PlanModeState, SharedPlanMode, SharedToolPermission, SubagentEnv,
    ToolContext, ToolRegistry, DEFAULT_SUBAGENT_MAX_DEPTH,
};
use zene_mcp::McpManager;

mod compaction;
mod events;
mod hooks;
mod permission;
mod plan_mode;
mod skills;
mod subagent;
mod tokens;
mod tool_dedup;
mod turn;

use compaction::{compact_session, is_context_overflow_error, should_compact, CompactionResult};
mod workspace;

pub use events::{emit_event, AgentEvent, EventHandler};
pub use hooks::{HookBlock, HookRunner};
pub use permission::{approve_tool_call, PermissionGate, PermissionMode, PermissionPrompter, PromptChoice};
pub use plan_mode::PlanApprovalPrompter;
pub use subagent::{run_subagent, ChatBackend, CoreSubagentRunner};
pub use turn::{StepId, TurnId, TurnState};
use plan_mode::{
    build_effective_system_prompt, default_plan_approval_prompter, handle_enter_plan_mode,
    handle_exit_plan_mode, tool_visible_in_definitions,
};
use tool_dedup::{append_reminder, ToolDedup};

pub struct Agent {
    config: ZeneConfig,
    client: ChatClient,
    tools: ToolRegistry,
    sandbox: Arc<LocalSandbox>,
    session: SessionRecord,
    turn_usage: TokenUsage,
    active_turn: Option<TurnState>,
    system_prompt: String,
    permission: SharedToolPermission,
    plan_mode: SharedPlanMode,
    plan_approval: PlanApprovalPrompter,
    tool_dedup: ToolDedup,
    hooks: HookRunner,
    record_writer: AgentRecordWriter,
    mcp: Option<McpManager>,
}

pub struct PromptOptions {
    pub stream: bool,
    pub cancel: Option<CancellationToken>,
    pub event_handler: Option<EventHandler>,
    /// When true, suppress stdout/stderr tool and stream printing (for TUI).
    pub quiet: bool,
}

impl Default for PromptOptions {
    fn default() -> Self {
        Self {
            stream: true,
            cancel: None,
            event_handler: None,
            quiet: false,
        }
    }
}

impl Agent {
    pub async fn new(
        config: ZeneConfig,
        sandbox: LocalSandbox,
        mut session: SessionRecord,
        permission_mode: PermissionMode,
    ) -> Result<Self> {
        let workdir = sandbox.workdir().to_path_buf();
        let system_prompt =
            workspace::build_system_prompt(&config.system_prompt, &workdir, config.include_workspace_context);
        session.ensure_system_message(&system_prompt);
        let client = ChatClient::from_config(&config).await?;
        let record_writer = AgentRecordWriter::for_session(&session.meta.id)?;

        let mut tools = zene_tools::builtin_tools();
        let (mcp, mcp_tools) = McpManager::connect(&workdir).await?;
        if !mcp_tools.definitions().is_empty() {
            info!(
                tool_count = mcp_tools.definitions().len(),
                "registered MCP tools"
            );
            tools.extend(mcp_tools);
        }
        let mcp = if mcp.is_empty() { None } else { Some(mcp) };
        let hook_entries = config.load_hooks().unwrap_or_else(|err| {
            warn!(error = %err, "failed to load hooks; continuing without hooks");
            Vec::new()
        });
        let hooks = HookRunner::new(hook_entries, workdir.clone());

        Ok(Self {
            config,
            client,
            tools,
            sandbox: Arc::new(sandbox),
            session,
            turn_usage: TokenUsage::default(),
            active_turn: None,
            system_prompt,
            permission: shared_permission(permission_mode),
            plan_mode: shared_plan_mode(),
            plan_approval: Arc::new(default_plan_approval_prompter),
            tool_dedup: ToolDedup::new(),
            hooks,
            record_writer,
            mcp,
        })
    }

    pub fn is_plan_mode_active(&self) -> bool {
        self.plan_mode
            .lock()
            .map(|s| s.is_active())
            .unwrap_or(false)
    }

    pub fn enter_plan_mode(&mut self) {
        let should_sync = {
            if let Ok(mut state) = self.plan_mode.lock() {
                if !state.is_active() {
                    state.enter();
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if should_sync {
            self.sync_plan_mode_system();
        }
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        let active = self.is_plan_mode_active();
        if !active {
            return true;
        }
        self.plan_mode
            .lock()
            .map(|s| s.is_tool_allowed(tool_name))
            .unwrap_or(true)
    }

    fn sync_plan_mode_system(&mut self) {
        let active = self.is_plan_mode_active();
        let effective = build_effective_system_prompt(&self.system_prompt, active);
        self.session.set_system_message(&effective);
    }

    fn tool_definitions_for_llm(&self) -> Vec<zene_llm::ToolDefinition> {
        let active = self.is_plan_mode_active();
        self.tools
            .filter_definitions(|name| tool_visible_in_definitions(name, active))
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(mcp) = self.mcp.as_mut() {
            mcp.disconnect().await?;
        }
        Ok(())
    }

    pub fn session(&self) -> &SessionRecord {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut SessionRecord {
        &mut self.session
    }

    pub fn turn_usage(&self) -> &TokenUsage {
        &self.turn_usage
    }

    pub fn is_turn_active(&self) -> bool {
        self.active_turn.is_some()
    }

    /// Replace the permission gate (e.g. TUI custom prompter).
    pub fn set_permission_gate(&mut self, gate: PermissionGate) {
        self.permission = Arc::new(Mutex::new(gate));
    }

    pub async fn prompt(&mut self, user_input: &str, options: PromptOptions) -> Result<String> {
        turn::begin_turn(&mut self.active_turn)?;
        let cancel = options.cancel.clone();
        let turn_id = self
            .active_turn
            .as_ref()
            .map(|t| t.turn_id)
            .expect("turn just started");

        self.record_writer.append(&RecordEntry::TurnPrompt {
            turn_id: turn_id.to_string(),
            prompt: user_input.to_string(),
            ts: chrono::Utc::now(),
        })?;

        let event_handler = merge_event_handler(&self.record_writer, options.event_handler.clone());

        info!(turn_id = %turn_id, "turn_start");
        emit_event(
            &event_handler,
            AgentEvent::TurnStart { turn_id },
        );

        let run_options = PromptOptions {
            stream: options.stream,
            cancel: options.cancel,
            event_handler,
            quiet: options.quiet,
        };
        let result = self.run_turn(user_input, &run_options, cancel.as_ref()).await;

        let steps = self.active_turn.as_ref().map(|t| t.step).unwrap_or(0);
        match &result {
            Ok(_) => {
                info!(turn_id = %turn_id, steps, "turn_end");
                emit_event(
                    &run_options.event_handler,
                    AgentEvent::TurnEnd { turn_id, steps },
                );
            }
            Err(err) => {
                if err.to_string().contains("aborted") {
                    info!(turn_id = %turn_id, steps, "turn_end");
                } else {
                    info!(turn_id = %turn_id, steps, error = %err, "turn_end");
                    emit_event(
                        &run_options.event_handler,
                        AgentEvent::Error {
                            message: err.to_string(),
                        },
                    );
                }
                emit_event(
                    &run_options.event_handler,
                    AgentEvent::TurnEnd { turn_id, steps },
                );
            }
        }

        turn::end_turn(&mut self.active_turn);
        result
    }

    async fn run_turn(
        &mut self,
        user_input: &str,
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<String> {
        self.turn_usage = TokenUsage::default();
        self.tool_dedup.reset();
        self.session.ensure_system_message(&self.system_prompt);
        self.session.set_title_from_prompt(user_input);
        self.session.push_message(Message::user(user_input));

        let mut final_text = String::new();
        let max_steps = self.config.max_turns;
        let mut completed = false;

        for _ in 0..max_steps {
            if Self::check_cancelled(cancel)? {
                return Err(turn::aborted_error());
            }

            let (turn_id, step_id, step_num) = {
                let turn = self
                    .active_turn
                    .as_mut()
                    .expect("active turn during run_turn");
                let step_id = turn.next_step_id();
                (turn.turn_id, step_id, turn.step)
            };
            debug!(
                turn_id = %turn_id,
                step_id = %step_id,
                step = step_num,
                "step_begin"
            );
            emit_event(
                &options.event_handler,
                AgentEvent::StepBegin {
                    turn_id,
                    step_id,
                    step: step_num,
                },
            );

            let step_result = self.run_step(options, cancel).await;
            debug!(
                turn_id = %turn_id,
                step_id = %step_id,
                step = step_num,
                ok = step_result.is_ok(),
                "step_end"
            );
            let (assistant_message, usage, had_tool_calls) = step_result?;

            if let Some(usage) = usage {
                debug!(
                    prompt_tokens = usage.prompt_tokens,
                    completion_tokens = usage.completion_tokens,
                    total_tokens = usage.total_tokens,
                    "llm response usage"
                );
                self.turn_usage.accumulate(&usage);
            }

            if had_tool_calls {
                if let Some(tool_calls) = assistant_message.tool_calls.clone() {
                    self.session.push_message(assistant_message);
                    self.run_tools(&tool_calls, options, cancel).await?;
                    continue;
                }
            }

            final_text = assistant_message.content.unwrap_or_default();
            self.session.push_message(Message::assistant(&final_text));
            completed = true;
            break;
        }

        if !completed {
            return Err(turn::max_steps_error(max_steps));
        }

        self.session.save()?;
        Ok(final_text)
    }

    async fn run_step(
        &mut self,
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>, bool)> {
        if Self::check_cancelled(cancel)? {
            return Err(turn::aborted_error());
        }

        self.maybe_compact_before_llm().await?;

        let tools = self.tool_definitions_for_llm();
        let (assistant_message, usage) = self
            .run_llm_step(&tools, options, cancel)
            .await
            .context("llm step")?;

        let had_tool_calls = assistant_message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());

        Ok((assistant_message, usage, had_tool_calls))
    }

    async fn maybe_compact_before_llm(&mut self) -> Result<()> {
        let messages = self.build_messages();
        let tools = self.tool_definitions_for_llm();
        let estimated_tokens = tokens::estimate_request_tokens(&messages, &tools);
        debug!(
            estimated_context_tokens = estimated_tokens,
            message_count = messages.len(),
            tool_count = tools.len(),
            "llm request context estimate"
        );

        if should_compact(estimated_tokens, &self.config.compaction) {
            if let Some(result) = compact_session(
                &mut self.session,
                &self.client,
                &self.config.model,
                &self.config.compaction,
                "token_threshold",
            )
            .await?
            {
                self.record_compaction(&result)?;
            }
            self.session
                .ensure_system_message(&self.system_prompt);
        }
        Ok(())
    }

    async fn run_llm_step(
        &mut self,
        tools: &[zene_llm::ToolDefinition],
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>)> {
        let mut overflow_retried = false;

        loop {
            if Self::check_cancelled(cancel)? {
                return Err(turn::aborted_error());
            }

            let messages = self.build_messages();
            let estimated_tokens = tokens::estimate_request_tokens(messages.as_slice(), tools);
            debug!(
                estimated_context_tokens = estimated_tokens,
                message_count = messages.len(),
                "llm step context estimate"
            );

            let request = ChatRequest {
                model: self.config.model.clone(),
                messages,
                tools: tools.to_vec(),
                stream: options.stream,
            };

            let result = if options.stream {
                self.run_streaming_step(request, options, cancel).await
            } else {
                self.client
                    .chat(request)
                    .await
                    .map(|response| (response.message, response.usage))
            };

            match result {
                Ok(value) => return Ok(value),
                Err(err) if !overflow_retried && is_context_overflow_error(&err) => {
                    overflow_retried = true;
                    if let Some(result) = compact_session(
                        &mut self.session,
                        &self.client,
                        &self.config.model,
                        &self.config.compaction,
                        "context_overflow",
                    )
                    .await?
                    {
                        self.record_compaction(&result)?;
                    }
                    self.session
                        .ensure_system_message(&self.system_prompt);
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn build_messages(&self) -> Vec<Message> {
        self.session.messages.clone()
    }

    fn check_cancelled(cancel: Option<&CancellationToken>) -> Result<bool> {
        Ok(cancel.is_some_and(CancellationToken::is_cancelled))
    }

    async fn run_streaming_step(
        &self,
        request: ChatRequest,
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>)> {
        if Self::check_cancelled(cancel)? {
            return Err(turn::aborted_error());
        }

        let mut stream = self.client.chat_stream(request).await?;
        let mut text = String::new();
        let mut tool_calls: Vec<ToolCallBuilder> = Vec::new();
        let mut usage = None;

        while let Some(event) = stream.next().await {
            if Self::check_cancelled(cancel)? {
                return Err(turn::aborted_error());
            }
            match event.context("stream event")? {
                StreamEvent::TextDelta(delta) => {
                    emit_event(
                        &options.event_handler,
                        AgentEvent::TextDelta {
                            delta: delta.clone(),
                        },
                    );
                    if !options.quiet {
                        print!("{delta}");
                        let _ = io::stdout().flush();
                    }
                    text.push_str(&delta);
                }
                StreamEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments,
                } => {
                    while tool_calls.len() <= index {
                        tool_calls.push(ToolCallBuilder::default());
                    }
                    let entry = &mut tool_calls[index];
                    if let Some(id) = id {
                        entry.id = id;
                    }
                    if let Some(name) = name {
                        entry.name = name;
                    }
                    if let Some(arguments) = arguments {
                        entry.arguments.push_str(&arguments);
                    }
                }
                StreamEvent::Done { usage: step_usage } => {
                    usage = step_usage;
                    break;
                }
            }
        }

        if !text.is_empty() && !options.quiet {
            println!();
        }

        let built_calls = tool_calls
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .map(|call| ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect::<Vec<_>>();

        let message = if built_calls.is_empty() {
            Message::assistant(text)
        } else {
            Message::assistant_with_tools(
                if text.is_empty() { None } else { Some(text) },
                built_calls,
            )
        };

        Ok((message, usage))
    }

    async fn run_tools(
        &mut self,
        tool_calls: &[ToolCall],
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let subagent_runner = Arc::new(CoreSubagentRunner::new(self.config.clone()));
        let permission: SharedToolPermission = Arc::clone(&self.permission);
        let plan_mode: SharedPlanMode = Arc::clone(&self.plan_mode);
        let plan_approval: PlanApprovalPrompter = Arc::clone(&self.plan_approval);
        let session_id = self.session.meta.id.clone();
        let workdir = self.sandbox.workdir().to_path_buf();
        let ctx = ToolContext {
            sandbox: Arc::clone(&self.sandbox),
            cancel: cancel.cloned(),
            subagent: Some(SubagentEnv {
                depth: 0,
                max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
                runner: subagent_runner,
            }),
            permission: Some(permission),
            plan_mode: Some(Arc::clone(&plan_mode)),
        };

        for call in tool_calls {
            if Self::check_cancelled(cancel)? {
                return Err(turn::aborted_error());
            }

            if !options.quiet {
                eprintln!("\n[tool] {}({})", call.name, truncate(&call.arguments, 120));
            }
            emit_event(
                &options.event_handler,
                AgentEvent::ToolCall {
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            );

            let result = if call.name == "EnterPlanMode" {
                let mut result = zene_tools::ToolResult {
                    content: "plan mode lock poisoned".to_string(),
                    is_error: true,
                };
                let mut should_sync = false;
                if let Ok(mut state) = plan_mode.lock() {
                    result = handle_enter_plan_mode(&mut state, &call.arguments);
                    should_sync = !result.is_error;
                }
                if should_sync {
                    self.sync_plan_mode_system();
                }
                result
            } else if call.name == "ExitPlanMode" {
                let mut result = zene_tools::ToolResult {
                    content: "plan mode lock poisoned".to_string(),
                    is_error: true,
                };
                let mut should_sync = false;
                if let Ok(mut state) = plan_mode.lock() {
                    result = handle_exit_plan_mode(
                        &mut state,
                        &call.arguments,
                        &workdir,
                        &session_id,
                        &plan_approval,
                    )
                    .unwrap_or_else(|err| zene_tools::ToolResult {
                        content: err.to_string(),
                        is_error: true,
                    });
                    should_sync = !result.is_error;
                }
                if should_sync {
                    self.sync_plan_mode_system();
                }
                result
            } else if let Some(block) = self
                .hooks
                .run_pre_tool_use(&call.name, &call.arguments)
                .await?
            {
                zene_tools::ToolResult {
                    content: format!("Hook blocked tool: {}", block.reason),
                    is_error: true,
                }
            } else if self.is_plan_mode_active() {
                let allowed_in_plan = self
                    .plan_mode
                    .lock()
                    .map(|s| s.is_tool_allowed(&call.name))
                    .unwrap_or(true);
                if !allowed_in_plan {
                    zene_tools::ToolResult {
                        content: PlanModeState::blocked_message(&call.name),
                        is_error: true,
                    }
                } else {
                    self.tools
                        .execute(&call.name, &call.arguments, &ctx)
                        .await
                        .unwrap_or_else(|err| zene_tools::ToolResult {
                            content: err.to_string(),
                            is_error: true,
                        })
                }
            } else {
                let allowed = match self.permission.lock() {
                    Ok(mut gate) => match gate.approve_tool_call(&call.name, &call.arguments) {
                        Ok(v) => v,
                        Err(err) => {
                            if !options.quiet {
                                eprintln!("permission prompt error: {err}");
                            }
                            false
                        }
                    },
                    Err(_) => {
                        if !options.quiet {
                            eprintln!("permission lock poisoned");
                        }
                        false
                    }
                };
                if !allowed {
                    zene_tools::ToolResult {
                        content: PermissionGate::denied_message(&call.name),
                        is_error: true,
                    }
                } else {
                    self.tools
                        .execute(&call.name, &call.arguments, &ctx)
                        .await
                        .unwrap_or_else(|err| zene_tools::ToolResult {
                            content: err.to_string(),
                            is_error: true,
                        })
                }
            };

            if !result.is_error {
                self.hooks
                    .run_post_tool_use(&call.name, &call.arguments)
                    .await;
            }

            if result.is_error && !options.quiet {
                eprintln!("[tool error] {}", truncate(&result.content, 200));
            }

            let mut content = if result.content.is_empty() {
                if result.is_error {
                    "(tool returned empty error output)".to_string()
                } else {
                    "(tool returned no output)".to_string()
                }
            } else {
                result.content
            };

            if let Some(reminder) = self.tool_dedup.on_call(&call.name, &call.arguments) {
                content = append_reminder(&content, reminder);
            }

            emit_event(
                &options.event_handler,
                AgentEvent::ToolResult {
                    name: call.name.clone(),
                    content: content.clone(),
                    is_error: result.is_error,
                },
            );

            self.session.push_message(Message::tool_result_with_error(
                &call.id,
                &call.name,
                content,
                result.is_error,
            ));
        }

        Ok(())
    }

    fn record_compaction(&self, result: &CompactionResult) -> Result<()> {
        self.record_writer.append(&RecordEntry::Compaction {
            reason: result.reason.clone(),
            compacted_count: result.compacted_count,
            ts: chrono::Utc::now(),
        })
    }
}

#[derive(Default)]
struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}...", input.chars().take(max).collect::<String>())
    }
}

fn shared_permission(mode: PermissionMode) -> SharedToolPermission {
    Arc::new(Mutex::new(PermissionGate::new(mode)))
}

fn merge_event_handler(
    record_writer: &AgentRecordWriter,
    user_handler: Option<EventHandler>,
) -> Option<EventHandler> {
    let record_writer = record_writer.clone();
    Some(Arc::new(move |event| {
        if let Some(entry) = record_entry_from_agent_event(&event) {
            let _ = record_writer.append(&entry);
        }
        if let Some(handler) = &user_handler {
            handler(event);
        }
    }))
}

fn record_entry_from_agent_event(event: &AgentEvent) -> Option<RecordEntry> {
    let ts = chrono::Utc::now();
    match event {
        AgentEvent::StepBegin {
            turn_id,
            step_id,
            step,
        } => Some(RecordEntry::StepBegin {
            turn_id: turn_id.to_string(),
            step_id: step_id.to_string(),
            step: *step,
            ts,
        }),
        AgentEvent::ToolCall { name, arguments } => Some(RecordEntry::ToolCall {
            name: name.clone(),
            arguments: arguments.clone(),
            ts,
        }),
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
        } => Some(RecordEntry::ToolResult {
            name: name.clone(),
            content: content.clone(),
            is_error: *is_error,
            ts,
        }),
        AgentEvent::TurnEnd { turn_id, steps } => Some(RecordEntry::TurnEnd {
            turn_id: turn_id.to_string(),
            steps: *steps,
            ts,
        }),
        AgentEvent::Error { message } => Some(RecordEntry::Error {
            message: message.clone(),
            ts,
        }),
        AgentEvent::TurnStart { .. } | AgentEvent::TextDelta { .. } => None,
    }
}
