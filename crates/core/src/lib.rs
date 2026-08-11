use std::io::{self, Write};
use std::sync::Arc;
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zene_config::ZeneConfig;
use zene_context::{
    ContextDeps, ContextEngine, PrefireClientFactory, StepContext,
};
use zene_llm::{ChatClient, ChatRequest, Message, StreamEvent, TokenUsage, ToolCall};
use zene_sandbox::{LocalSandbox, Sandbox};
use zene_session::{
    fork_session, latest_checkpoint_id, load_checkpoint, restore_checkpoint, save_checkpoint,
    AgentRecordWriter, RecordEntry, SessionRecord,
};
use zene_tools::{
    shared_todo_store_from,
    SharedAskUserPrompter, SharedBackgroundTasks, SharedTodoStore, SharedPlanMode,
    SubagentEnv, ToolRegistry, DEFAULT_SUBAGENT_MAX_DEPTH,
};
pub use zene_tools::AskUserOption;
use zene_mcp::McpManager;

mod agent_builder;
mod agent_turn;
mod context_config;
mod context_events;
mod context_hooks;
mod events;
mod plan_mode;
mod subagent;
mod tool_dedup;
mod tool_executor;
pub mod tool_scheduler;
mod worktree;

pub use zene_context::{
    compact_session, compact_session_forced, ensure_memory_in_system, memory_enabled,
    memory_root, CompactionResult, ContextWaterLevel, EstimateMode, EstimateProvider,
    InputLadderStage, TiktokenEncoding, TokenEstimator, estimate_context,
};

pub use agent_builder::AgentBuilder;
pub use events::{emit_event, runtime_event_handler, AgentEvent, EventHandler};
pub use zene_hooks::{HookBlock, HookRunner, HookSpec};
pub use zene_permission::{
    approve_tool_call, policy_denied, PermissionGate, PermissionMode, PermissionPrompter,
    PermissionRule, PromptChoice, RuleAction, SharedToolPermission, ToolPermission,
};
pub use plan_mode::PlanApprovalPrompter;
pub use subagent::{run_subagent, ChatBackend, CoreSubagentRunner};
pub use zene_turn::{
    aborted_error, begin_turn, end_turn, max_turns_notice, steer_requires_active_turn,
    EventSequence, RuntimeEvent, RuntimeEventHandler, RuntimeEventKind, SessionId, SteerBuffer,
    StepId, ToolCallId, TurnId, TurnState,
};
use plan_mode::{build_effective_system_prompt, tool_visible_in_definitions};
pub use tool_dedup::{append_reminder, ToolDedup};
pub use tool_scheduler::{classify_tool_accesses, ToolScheduler};
use crate::tool_executor::{DefaultToolExecutor, ToolExecutorDeps};
pub use zene_workspace::{build_system_prompt, FsWorkspaceProvider, WorkspaceProvider};
pub use worktree::ensure_session_worktree;

pub struct Agent {
    config: ZeneConfig,
    client: ChatClient,
    tools: Arc<ToolRegistry>,
    sandbox: Arc<dyn Sandbox>,
    session: SessionRecord,
    turn_usage: TokenUsage,
    context: ContextEngine,
    active_turn: Option<TurnState>,
    steer_buffer: Arc<Mutex<SteerBuffer>>,
    system_prompt: String,
    permission: SharedToolPermission,
    plan_mode: SharedPlanMode,
    plan_approval: PlanApprovalPrompter,
    todos: SharedTodoStore,
    ask_user: SharedAskUserPrompter,
    tool_dedup: ToolDedup,
    hooks: HookRunner,
    record_writer: AgentRecordWriter,
    mcp: Option<McpManager>,
    background: SharedBackgroundTasks,
}

pub struct PromptOptions {
    pub stream: bool,
    pub cancel: Option<CancellationToken>,
    pub event_handler: Option<EventHandler>,
    /// Shared runtime event sink; legacy event_handler remains supported.
    pub runtime_event_handler: Option<RuntimeEventHandler>,
    /// When true, suppress stdout/stderr tool and stream printing (for TUI).
    pub quiet: bool,
}

impl Default for PromptOptions {
    fn default() -> Self {
        Self {
            stream: true,
            cancel: None,
            event_handler: None,
            runtime_event_handler: None,
            quiet: false,
        }
    }
}

impl Agent {
    pub async fn new(
        config: ZeneConfig,
        sandbox: LocalSandbox,
        session: SessionRecord,
        permission_mode: PermissionMode,
    ) -> Result<Self> {
        AgentBuilder::new(config, sandbox, session, permission_mode)
            .build()
            .await
    }

    /// Override inference session id (e.g. Cloud run_id for gateway linkage).
    pub fn set_external_session_id(&mut self, id: Option<String>) {
        self.context.set_external_session_id(id);
    }

    pub fn is_plan_mode_active(&self) -> bool {
        self.plan_mode
            .lock()
            .is_active()
    }

    pub fn enter_plan_mode(&mut self) {
        self.enter_plan_mode_with_handler(&None);
    }

    pub fn leave_plan_mode(&mut self) {
        self.leave_plan_mode_with_handler(&None);
    }

    /// Set ACP/session mode. Returns the active mode id (`default` or `plan`).
    pub fn set_session_mode(&mut self, mode_id: &str) -> Result<String> {
        match mode_id {
            "plan" | "architect" => {
                self.enter_plan_mode();
                Ok("plan".into())
            }
            "default" | "agent" | "code" | "ask" => {
                self.leave_plan_mode();
                Ok("default".into())
            }
            other => bail!("unknown session mode: {other}"),
        }
    }

    pub fn current_session_mode(&self) -> &'static str {
        if self.is_plan_mode_active() {
            "plan"
        } else {
            "default"
        }
    }

    fn enter_plan_mode_with_handler(&mut self, handler: &Option<EventHandler>) {
        let should_sync = {
            let mut state = self.plan_mode.lock();
            if state.is_active() {
                false
            } else {
                state.enter();
                true
            }
        };
        if should_sync {
            self.sync_plan_mode_system();
            emit_event(
                handler,
                AgentEvent::ModeChanged {
                    mode_id: "plan".into(),
                },
            );
        }
    }

    fn leave_plan_mode_with_handler(&mut self, handler: &Option<EventHandler>) {
        let should_sync = {
            let mut state = self.plan_mode.lock();
            if !state.is_active() {
                false
            } else {
                state.exit();
                true
            }
        };
        if should_sync {
            self.sync_plan_mode_system();
            emit_event(
                handler,
                AgentEvent::ModeChanged {
                    mode_id: "default".into(),
                },
            );
        }
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        let active = self.is_plan_mode_active();
        if !active {
            return true;
        }
        self.plan_mode
            .lock()
            .is_tool_allowed(tool_name)
    }

    fn sync_plan_mode_system(&mut self) {
        let active = self.is_plan_mode_active();
        let effective = build_effective_system_prompt(&self.system_prompt, active);
        self.session.set_system_message(&effective);
        let _event = self
            .context
            .on_system_prefix_changed("plan_mode");
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
        let session_id = self.context.metadata(&self.session).session_id;
        zene_context::close_session(&session_id).await;
        self.sandbox.shutdown().await?;
        Ok(())
    }

    pub fn config(&self) -> &ZeneConfig {
        &self.config
    }

    pub async fn switch_model(
        &mut self,
        model: &str,
        provider: Option<String>,
        base_url: Option<String>,
        api_key: Option<String>,
    ) -> Result<()> {
        if let Some(p) = provider {
            self.config.provider = p;
        }
        self.config.model = model.to_string();
        if let Some(url) = base_url {
            if self.config.provider.trim().to_lowercase() == "anthropic" {
                self.config.anthropic_base_url = Some(url);
            } else {
                self.config.base_url = url;
            }
        }
        if let Some(key) = api_key {
            if self.config.provider.trim().to_lowercase() == "anthropic" {
                self.config.anthropic_api_key = Some(key);
            } else {
                self.config.api_key = Some(key);
            }
        }

        self.config.refresh_model_context_window();
        self.context
            .set_window(self.config.compaction.context_window_tokens);

        // Recreate the client
        self.client = zene_llm::ChatClient::from_config(&self.config).await?;
        self.config
            .persist_connection_settings()
            .context("save model settings to ~/.zene/config.toml")?;
        Ok(())
    }

    pub fn context_water(&self) -> &ContextWaterLevel {
        &self.context.water()
    }

    /// Manually compact the conversation (`/compact [hint]`).
    pub async fn compact_now(&mut self, user_hint: Option<&str>) -> Result<Option<CompactionResult>> {
        self.sync_todos_to_session();
        let tools = self.tool_definitions_for_llm();
        let estimator = self.token_estimator();
        let background_tasks = self.background.lock().list();
        let hooks = context_hooks::ZeneContextHooks::new(&self.session, &background_tasks);
        let compaction_config = context_config::context_compaction_config(&self.config.compaction);
        let mut handler = context_events::AgentContextHandler::new(
            &self.client,
            &self.config.model,
            self.sandbox.workdir(),
        );
        let prefire_factory = self.prefire_client_factory();
        let mut deps = ContextDeps {
            session: &mut self.session,
            compaction_config: &compaction_config,
            model: &self.config.model,
            client: &self.client,
            hooks: Some(&hooks),
            system_prompt: &self.system_prompt,
            estimator: &estimator,
            handler: &mut handler,
            prefire_client_factory: prefire_factory,
        };
        let result = self
            .context
            .compact_forced(&mut deps, &tools, user_hint)
            .await;
        match &result {
            Ok(forced) => {
                if let Some(compact_result) = &forced.compaction {
                    self.record_compaction(compact_result)?;
                }
                self.session.save()?;
                Ok(forced.compaction.clone())
            }
            Err(_) => result.map(|forced| forced.compaction),
        }
    }

    /// Human-readable context report for `/context` (grok-aligned).
    pub fn context_report(&self) -> String {
        let water = self.context.water();
        let cfg = context_config::context_compaction_config(&self.config.compaction);
        let threshold_pct = ContextWaterLevel::auto_compact_threshold_percent(&cfg);
        let threshold_tokens = ContextWaterLevel::threshold_tokens(&cfg);
        let used = water.effective_tokens();
        let window = water.context_window_tokens.max(1);
        let mut lines = vec![
            format!(
                "context: {}% ({} / {} tokens)",
                water.usage_percent(),
                used,
                window
            ),
            format!(
                "auto-compact at {}% ({} tokens)",
                threshold_pct, threshold_tokens
            ),
            format!(
                "sources: usage={} estimate={}",
                water
                    .last_prompt_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                water
                    .last_estimate_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!("messages: {}", self.session.messages.len()),
            format!("model: {}", self.config.model),
            format!("context_epoch: {}", self.context.epoch()),
        ];
        let delivery = zene_context::delivery_mode_from_env();
        lines.push(format!(
            "delivery: {} (gateway_prefix_len={})",
            delivery.as_str(),
            self.context.gateway_prefix_len()
        ));
        if water.auto_compact_suppressed {
            lines.push(
                "auto-compact: suppressed (last summarize failed; use /compact to retry)"
                    .to_string(),
            );
        }
        if self.context.prefire_has_cache() {
            lines.push("prefire: NOTE₁ cached (two-pass ready)".to_string());
        } else if self.context.prefire_in_flight() {
            lines.push("prefire: pass1 in flight".to_string());
        }
        if memory_enabled() {
            let root = memory_root(self.sandbox.workdir());
            lines.push(format!("memory: {} (ZENE_MEMORY)", root.display()));
        } else {
            lines.push("memory: disabled".to_string());
        }
        lines.join("\n")
    }

    /// Rewind to the latest compaction checkpoint (or a specific id).
    pub fn rewind_to_checkpoint(&mut self, checkpoint_id: Option<&str>) -> Result<String> {
        let id = match checkpoint_id {
            Some(id) => id.to_string(),
            None => latest_checkpoint_id(&self.session.meta.id)?
                .ok_or_else(|| anyhow::anyhow!("no compaction checkpoint available to rewind"))?,
        };
        let checkpoint = load_checkpoint(&self.session.meta.id, &id)?;
        restore_checkpoint(&mut self.session, &checkpoint);
        self.session.ensure_system_message(&self.system_prompt);
        if let Some(tokens) = self.session.context_tokens_used {
            self.context.restore_water_from_session(tokens);
        }
        self.context.clear_prefire();
        self.session.save()?;
        Ok(id)
    }

    /// Fork the current session into a new saved session and switch to it.
    pub fn fork_session(&mut self) -> Result<String> {
        let workdir = self.sandbox.workdir().to_path_buf();
        let forked = fork_session(&self.session, &workdir);
        let id = forked.meta.id.clone();
        forked.save()?;
        let _ = save_checkpoint(&forked, "fork");
        self.session = forked;
        self.record_writer = AgentRecordWriter::for_session(&id)?;
        self.todos = shared_todo_store_from(self.session.todos.clone());
        self.context.clear_prefire();
        Ok(id)
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

    pub fn pending_steer_count(&self) -> usize {
        self.steer_buffer
            .lock()
            .len()
    }

    pub fn steer_buffer(&self) -> Arc<Mutex<SteerBuffer>> {
        Arc::clone(&self.steer_buffer)
    }

    /// Queue follow-up user guidance for the active turn (injected between steps).
    pub fn queue_steer(&self, text: &str) -> Result<()> {
        if !self.is_turn_active() {
            return Err(zene_turn::steer_requires_active_turn());
        }
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("steer message cannot be empty");
        }
        self.steer_buffer
            .lock()
            .push(text.to_string());
        Ok(())
    }

    /// Queue follow-up user guidance for the active turn (injected between steps).
    pub fn steer(&mut self, text: &str) -> Result<()> {
        self.queue_steer(text)
    }

    fn token_estimator(&self) -> TokenEstimator {
        TokenEstimator::for_provider(
            EstimateProvider::from_name(&self.config.provider),
            &self.config.model,
            self.config.chars_per_token_for_model(),
        )
    }

    fn prefire_client_factory(&self) -> Option<PrefireClientFactory> {
        let config = self.config.clone();
        Some(std::sync::Arc::new(move || {
            let config = config.clone();
            Box::pin(async move { ChatClient::from_config(&config).await })
        }))
    }

    fn warn_if_near_context_limit(&self, estimated_tokens: usize) {
        let window = self.config.compaction.context_window_tokens as f32;
        if window <= 0.0 {
            return;
        }
        let ratio = estimated_tokens as f32 / window;
        if ratio >= 0.9 {
            warn!(
                estimated_context_tokens = estimated_tokens,
                context_window_tokens = self.config.compaction.context_window_tokens,
                usage_ratio = ratio,
                "context estimate exceeds 90% of model window"
            );
        }
    }

    /// Replace the permission gate (e.g. TUI custom prompter).
    ///
    /// Re-applies sandbox `auto_allow_bash` and config permission rules so a TUI
    /// prompter swap does not drop sandbox-linked policy.
    pub fn set_permission_gate(&mut self, mut gate: PermissionGate) {
        gate.set_auto_allow_bash(self.config.sandbox.auto_allow_bash && self.sandbox.is_enforced());
        gate.set_rules(agent_builder::permission_rules_from_config(&self.config));
        self.permission = Arc::new(Mutex::new(gate));
    }

    /// Replace the AskUserQuestion prompter (e.g. TUI modal).
    pub fn set_ask_user_prompter(&mut self, prompter: SharedAskUserPrompter) {
        self.ask_user = prompter;
    }

    pub async fn prompt(&mut self, user_input: &str, options: PromptOptions) -> Result<String> {
        zene_turn::begin_turn(&mut self.active_turn)?;
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

        let event_handler = merge_event_handler(
            &self.record_writer,
            self.session.meta.id.clone(),
            options.event_handler.clone(),
            options.runtime_event_handler.clone(),
        );

        info!(turn_id = %turn_id, "turn_start");
        emit_event(
            &event_handler,
            AgentEvent::TurnStart { turn_id },
        );

        let run_options = PromptOptions {
            stream: options.stream,
            cancel: options.cancel,
            event_handler,
            runtime_event_handler: None,
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

        zene_turn::end_turn(&mut self.active_turn);
        result
    }

    async fn run_turn(
        &mut self,
        user_input: &str,
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<String> {
        let mut ports = zene_turn::LegacyTurnPorts::new(self);
        zene_turn::TurnEngine::new(&mut ports)
            .run(zene_turn::TurnRequest::new(user_input, options, cancel))
            .await
            .map(|outcome| outcome.final_text)
    }

    fn sync_todos_to_session(&mut self) {
        let store = self.todos.lock();
        self.session.todos = store.to_items();
    }

    fn inject_pending_steer(&mut self, options: &PromptOptions) -> Result<bool> {
        let messages = self
            .steer_buffer
            .lock()
            .take_all();
        if messages.is_empty() {
            return Ok(false);
        }
        for text in messages {
            info!(steer_chars = text.len(), "steer_injected");
            emit_event(
                &options.event_handler,
                AgentEvent::SteerInput {
                    text: text.clone(),
                },
            );
            self.session.push_message(Message::user(text));
        }
        Ok(true)
    }

    async fn run_step(
        &mut self,
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>, bool)> {
        if Self::check_cancelled(cancel)? {
            return Err(zene_turn::aborted_error());
        }

        self.sync_todos_to_session();
        let tools = self.tool_definitions_for_llm();
        let estimator = self.token_estimator();
        let background_tasks = self.background.lock().list();
        let hooks = context_hooks::ZeneContextHooks::new(&self.session, &background_tasks);
        let compaction_config = context_config::context_compaction_config(&self.config.compaction);
        let mut handler = context_events::AgentContextHandler::new(
            &self.client,
            &self.config.model,
            self.sandbox.workdir(),
        );
        let prefire_factory = self.prefire_client_factory();
        let mut deps = ContextDeps {
            session: &mut self.session,
            compaction_config: &compaction_config,
            model: &self.config.model,
            client: &self.client,
            hooks: Some(&hooks),
            system_prompt: &self.system_prompt,
            estimator: &estimator,
            handler: &mut handler,
            prefire_client_factory: prefire_factory,
        };
        let prepared = self.context.prepare_step(&mut deps, &tools).await?;
        if let Some(result) = &prepared.compaction {
            self.record_compaction(result)?;
        }
        let step = prepared.step;
        debug!(
            estimated_context_tokens = step.estimate_tokens,
            effective_tokens = self.context.water().effective_tokens(),
            usage_percent = self.context.water().usage_percent(),
            message_count = step.messages.len(),
            tool_count = tools.len(),
            context_epoch = step.metadata.context_epoch,
            "llm request context water level"
        );
        self.warn_if_near_context_limit(step.estimate_tokens as usize);

        let (assistant_message, usage) = self
            .run_llm_step(&step, &tools, options, cancel)
            .await
            .context("llm step")?;

        let had_tool_calls = assistant_message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());

        Ok((assistant_message, usage, had_tool_calls))
    }

    async fn run_llm_step(
        &mut self,
        step: &StepContext,
        tools: &[zene_llm::ToolDefinition],
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>)> {
        let mut overflow_truncated = false;
        let mut overflow_summarized = false;
        let mut messages = step.messages.clone();
        let mut metadata = step.metadata.clone();

        loop {
            if Self::check_cancelled(cancel)? {
                return Err(zene_turn::aborted_error());
            }

            debug!(
                estimated_context_tokens = step.estimate_tokens,
                message_count = messages.len(),
                "llm step context estimate"
            );

            let request = ChatRequest {
                model: self.config.model.clone(),
                messages: messages.clone(),
                tools: tools.to_vec(),
                stream: options.stream,
                context: Some(metadata.clone()),
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
                Err(err) if ContextEngine::is_context_overflow_error(&err) => {
                    self.sync_todos_to_session();
                    let estimator = self.token_estimator();
                    let background_tasks = self.background.lock().list();
                    let hooks = context_hooks::ZeneContextHooks::new(&self.session, &background_tasks);
                    let compaction_config =
                        context_config::context_compaction_config(&self.config.compaction);
                    let mut handler = context_events::AgentContextHandler::new(
                        &self.client,
                        &self.config.model,
                        self.sandbox.workdir(),
                    );
                    let prefire_factory = self.prefire_client_factory();
                    let overflow = {
                        let mut deps = ContextDeps {
                            session: &mut self.session,
                            compaction_config: &compaction_config,
                            model: &self.config.model,
                            client: &self.client,
                            hooks: Some(&hooks),
                            system_prompt: &self.system_prompt,
                            estimator: &estimator,
                            handler: &mut handler,
                            prefire_client_factory: prefire_factory,
                        };
                        self.context
                            .handle_overflow(
                                &mut deps,
                                tools,
                                &mut overflow_truncated,
                                &mut overflow_summarized,
                            )
                            .await?
                    };
                    if let Some(result) = &overflow.compaction {
                        self.record_compaction(result)?;
                    }
                    if overflow.retry {
                        let refreshed = self.context.assemble_step(
                            &self.session,
                            tools,
                            &estimator,
                        );
                        messages = refreshed.messages;
                        metadata = refreshed.metadata;
                        continue;
                    }
                    return Err(err);
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn check_cancelled(cancel: Option<&CancellationToken>) -> Result<bool> {
        Ok(zene_turn::is_cancelled(cancel))
    }

    async fn run_streaming_step(
        &self,
        request: ChatRequest,
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>)> {
        if Self::check_cancelled(cancel)? {
            return Err(zene_turn::aborted_error());
        }

        let mut stream = self.client.chat_stream(request).await?;
        let mut text = String::new();
        let mut tool_calls: Vec<ToolCallBuilder> = Vec::new();
        let mut usage = None;

        while let Some(event) = stream.next().await {
            if Self::check_cancelled(cancel)? {
                return Err(zene_turn::aborted_error());
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
                StreamEvent::ThoughtDelta(delta) => {
                    emit_event(
                        &options.event_handler,
                        AgentEvent::ThoughtDelta {
                            delta: delta.clone(),
                        },
                    );
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

        let built_calls = normalize_tool_calls(
            tool_calls
                .into_iter()
                .filter(|call| !call.name.is_empty())
                .map(|call| ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                })
                .collect(),
        );

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
    ) -> Result<zene_turn::ToolBatchOutcome> {
        let subagent_runner = Arc::new(CoreSubagentRunner::new(self.config.clone()));
        let result = {
            let executor = DefaultToolExecutor::new(ToolExecutorDeps {
                tools: Arc::clone(&self.tools),
                sandbox: Arc::clone(&self.sandbox),
                permission: Arc::clone(&self.permission),
                plan_mode: Arc::clone(&self.plan_mode),
                plan_approval: &self.plan_approval,
                todos: Arc::clone(&self.todos),
                ask_user: Arc::clone(&self.ask_user),
                background: Arc::clone(&self.background),
                subagent: Some(SubagentEnv {
                    depth: 0,
                    max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
                    runner: subagent_runner,
                }),
                hooks: &self.hooks,
            });
            executor
                .execute(
                    tool_calls,
                    options,
                    cancel,
                    &self.session.meta.id,
                    self.sandbox.workdir(),
                    &mut self.tool_dedup,
                )
                .await?
        };

        if !result.mode_changes.is_empty() {
            self.sync_plan_mode_system();
        }
        for message in result.messages {
            self.session.push_message(Message::tool_result_with_error(
                &message.call.id,
                &message.call.name,
                message.content,
                message.is_error,
            ));
        }
        Ok(result.outcome)
    }

    fn record_compaction(&self, result: &CompactionResult) -> Result<()> {
        self.record_writer.append(&RecordEntry::Compaction {
            reason: result.reason.clone(),
            compacted_count: result.compacted_count,
            tokens_before: Some(result.stats.tokens_before),
            tokens_after: Some(result.stats.tokens_after),
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

/// Streaming providers sometimes omit tool call ids; API follow-up turns require stable unique ids.
fn normalize_tool_calls(calls: Vec<ToolCall>) -> Vec<ToolCall> {
    let mut used_ids = std::collections::HashSet::new();
    calls
        .into_iter()
        .enumerate()
        .map(|(index, mut call)| {
            if call.id.trim().is_empty() {
                call.id = format!("call_{index}");
            }
            let base = call.id.clone();
            let mut unique = base.clone();
            let mut suffix = 0u32;
            while !used_ids.insert(unique.clone()) {
                suffix += 1;
                unique = format!("{base}_{suffix}");
            }
            call.id = unique;
            call
        })
        .collect()
}

fn merge_event_handler(
    record_writer: &AgentRecordWriter,
    session_id: String,
    user_handler: Option<EventHandler>,
    runtime_handler: Option<RuntimeEventHandler>,
) -> Option<EventHandler> {
    let record_writer = record_writer.clone();
    let shared = runtime_event_handler(
        SessionId::from_string(session_id),
        user_handler,
        runtime_handler,
    );
    Some(Arc::new(move |event| {
        if let Some(entry) = record_entry_from_agent_event(&event) {
            let _ = record_writer.append(&entry);
        }
        shared(event);
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
        AgentEvent::ToolCall { name, arguments, .. } => Some(RecordEntry::ToolCall {
            name: name.clone(),
            arguments: arguments.clone(),
            ts,
        }),
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
            duration_ms: _,
            ..
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
        AgentEvent::TurnStart { .. }
        | AgentEvent::TextDelta { .. }
        | AgentEvent::ThoughtDelta { .. }
        | AgentEvent::SteerInput { .. }
        | AgentEvent::ModeChanged { .. }
        | AgentEvent::UsageUpdate { .. } => None,
    }
}
