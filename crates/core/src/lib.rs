use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zene_config::ZeneConfig;
use zene_context::{ContextDeps, ContextEngine, ContextEvent, PrefireClientFactory, StepContext};
use zene_llm::{ChatClient, ChatRequest, Message, StreamEvent, TokenUsage, ToolCall};
use zene_sandbox::{LocalSandbox, Sandbox};
use zene_session::{
    fork_session, latest_checkpoint_id, load_checkpoint, restore_checkpoint, save_checkpoint,
    AgentRecordWriter, RecordEntry, SessionRecord,
};
use zene_tool_runtime::{
    apply_tool_bound_plan, plan_tool_output_bound, FsToolOutputStore,
};
use zene_tools::{
    shared_todo_store_from,
    SharedAskUserPrompter, SharedBackgroundTasks, SharedTodoStore, PlanModeState,
    SharedPlanMode, SubagentEnv, ToolContext, ToolRegistry,
    DEFAULT_SUBAGENT_MAX_DEPTH,
};
pub use zene_tools::AskUserOption;
use zene_mcp::McpManager;

mod agent_builder;
mod context_config;
mod context_hooks;
mod events;
mod plan_mode;
mod skills;
mod subagent;
mod tool_dedup;
pub mod tool_scheduler;
mod turn;
mod worktree;

pub use zene_context::{
    compact_session, compact_session_forced, ensure_memory_in_system, memory_enabled,
    memory_root, CompactionResult, ContextWaterLevel, EstimateMode, EstimateProvider,
    InputLadderStage, TiktokenEncoding, TokenEstimator, estimate_context,
};
mod workspace;

pub use agent_builder::AgentBuilder;
pub use events::{emit_event, AgentEvent, EventHandler};
pub use zene_hooks::{HookBlock, HookRunner, HookSpec};
pub use zene_permission::{
    approve_tool_call, policy_denied, PermissionGate, PermissionMode, PermissionPrompter,
    PermissionRule, PromptChoice, RuleAction, SharedToolPermission, ToolPermission,
};
pub use plan_mode::PlanApprovalPrompter;
pub use subagent::{run_subagent, ChatBackend, CoreSubagentRunner};
pub use turn::{SteerBuffer, StepId, TurnId, TurnState};
use plan_mode::{
    build_effective_system_prompt, handle_enter_plan_mode,
    handle_exit_plan_mode, tool_visible_in_definitions,
};
pub use tool_dedup::{append_reminder, ToolDedup};
pub use tool_scheduler::{classify_tool_accesses, ToolScheduler};
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
        let prefire_factory = self.prefire_client_factory();
        let mut deps = ContextDeps {
            session: &mut self.session,
            compaction_config: &compaction_config,
            model: &self.config.model,
            workdir: self.sandbox.workdir(),
            client: &self.client,
            hooks: Some(&hooks),
            system_prompt: &self.system_prompt,
            estimator: &estimator,
            prefire_client_factory: prefire_factory,
        };
        let result = self
            .context
            .compact_forced(&mut deps, &tools, user_hint)
            .await;
        match &result {
            Ok(forced) => {
                self.dispatch_context_events(&forced.events)?;
                if let Some(compact_result) = &forced.compaction {
                    self.record_compaction(compact_result)?;
                }
                self.session.save()?;
                Ok(forced.compaction.clone())
            }
            Err(_) => result.map(|forced| forced.compaction),
        }
    }

    fn dispatch_context_events(&self, events: &[ContextEvent]) -> Result<()> {
        for event in events {
            if let ContextEvent::Checkpoint { reason } = event {
                let _ = save_checkpoint(&self.session, reason)?;
            }
        }
        Ok(())
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
            return Err(turn::steer_requires_active_turn());
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
        // 0 = unlimited (do not use `for _ in 0..0`).
        let max_steps = self.config.max_turns;
        let mut completed = false;
        let mut steps_done = 0u32;

        loop {
            if max_steps > 0 && steps_done >= max_steps {
                break;
            }
            steps_done = steps_done.saturating_add(1);

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
                let tools = self.tool_definitions_for_llm();
                let estimator = self.token_estimator();
                let compaction_config =
                    context_config::context_compaction_config(&self.config.compaction);
                self.context.record_step_usage(
                    &usage,
                    &mut self.session,
                    &tools,
                    &estimator,
                    &compaction_config,
                );
                emit_event(
                    &options.event_handler,
                    AgentEvent::UsageUpdate {
                        usage: self.turn_usage,
                        context_tokens: self.context.water().effective_tokens(),
                        context_window: self.config.compaction.context_window_tokens,
                        context_percent: self.context.water().usage_percent(),
                        context_epoch: self.context.epoch(),
                    },
                );
            }

            if had_tool_calls {
                if let Some(tool_calls) = assistant_message.tool_calls.clone() {
                    self.session.push_message(assistant_message);
                    self.run_tools(&tool_calls, options, cancel).await?;
                    if self.inject_pending_steer(options)? {
                        continue;
                    }
                    continue;
                }
            }

            self.session.push_message(assistant_message.clone());
            if self.inject_pending_steer(options)? {
                continue;
            }

            final_text = assistant_message.content.unwrap_or_default();
            completed = true;
            break;
        }

        if !completed {
            // Soft-stop: leave the session usable for a follow-up instead of failing the run.
            let notice = turn::max_turns_notice(max_steps);
            let delta = if final_text.trim().is_empty() {
                format!("\n{notice}\n")
            } else {
                format!("\n\n{notice}")
            };
            final_text = if final_text.trim().is_empty() {
                notice
            } else {
                format!("{final_text}\n\n{notice}")
            };
            self.session.push_message(Message::assistant(&final_text));
            emit_event(
                &options.event_handler,
                AgentEvent::TextDelta { delta },
            );
        }

        self.sync_todos_to_session();
        self.session.save()?;
        Ok(final_text)
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
            return Err(turn::aborted_error());
        }

        self.sync_todos_to_session();
        let tools = self.tool_definitions_for_llm();
        let estimator = self.token_estimator();
        let background_tasks = self.background.lock().list();
        let hooks = context_hooks::ZeneContextHooks::new(&self.session, &background_tasks);
        let compaction_config = context_config::context_compaction_config(&self.config.compaction);
        let prefire_factory = self.prefire_client_factory();
        let mut deps = ContextDeps {
            session: &mut self.session,
            compaction_config: &compaction_config,
            model: &self.config.model,
            workdir: self.sandbox.workdir(),
            client: &self.client,
            hooks: Some(&hooks),
            system_prompt: &self.system_prompt,
            estimator: &estimator,
            prefire_client_factory: prefire_factory,
        };
        let prepared = self.context.prepare_step(&mut deps, &tools).await?;
        self.dispatch_context_events(&prepared.events)?;
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
                return Err(turn::aborted_error());
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
                    let prefire_factory = self.prefire_client_factory();
                    let overflow = {
                        let mut deps = ContextDeps {
                            session: &mut self.session,
                            compaction_config: &compaction_config,
                            model: &self.config.model,
                            workdir: self.sandbox.workdir(),
                            client: &self.client,
                            hooks: Some(&hooks),
                            system_prompt: &self.system_prompt,
                            estimator: &estimator,
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
                    self.dispatch_context_events(&overflow.events)?;
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
            todos: Some(Arc::clone(&self.todos)),
            ask_user: Some(Arc::clone(&self.ask_user)),
            background: Some(Arc::clone(&self.background)),
        };

        struct PreparedTool {
            call: ToolCall,
            immediate: Option<(zene_tools::ToolResult, Option<u64>)>,
            schedule: Option<(tool_scheduler::ToolAccesses, String, String)>,
        }

        let mut prepared = Vec::with_capacity(tool_calls.len());

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
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            );

            let immediate = if call.name == "EnterPlanMode" {
                let mut state = plan_mode.lock();
                let result = handle_enter_plan_mode(&mut state, &call.arguments);
                drop(state);
                if !result.is_error {
                    self.sync_plan_mode_system();
                    emit_event(
                        &options.event_handler,
                        AgentEvent::ModeChanged {
                            mode_id: "plan".into(),
                        },
                    );
                }
                Some((result, None))
            } else if call.name == "ExitPlanMode" {
                let mut state = plan_mode.lock();
                let result = handle_exit_plan_mode(
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
                drop(state);
                if !result.is_error {
                    self.sync_plan_mode_system();
                    emit_event(
                        &options.event_handler,
                        AgentEvent::ModeChanged {
                            mode_id: "default".into(),
                        },
                    );
                }
                Some((result, None))
            } else if let Some(block) = self
                .hooks
                .run_pre_tool_use(&call.name, &call.arguments)
                .await?
            {
                Some((
                    zene_tools::ToolResult {
                        content: format!("Hook blocked tool: {}", block.reason),
                        is_error: true,
                    },
                    None,
                ))
            } else if !self.tools.contains(&call.name) {
                Some((
                    zene_tools::ToolResult {
                        content: format!("unknown tool: {}", call.name),
                        is_error: true,
                    },
                    None,
                ))
            } else if self.is_plan_mode_active() {
                let allowed_in_plan = self
                    .plan_mode
                    .lock()
                    .is_tool_allowed(&call.name);
                if !allowed_in_plan {
                    Some((
                        zene_tools::ToolResult {
                            content: PlanModeState::blocked_message(&call.name),
                            is_error: true,
                        },
                        None,
                    ))
                } else {
                    None
                }
            } else {
                let allowed = match self.permission.lock().approve_tool_call(&call.name, &call.arguments) {
                    Ok(v) => v,
                    Err(err) => {
                        if !options.quiet {
                            eprintln!("permission prompt error: {err}");
                        }
                        false
                    }
                };
                if !allowed {
                    Some((
                        zene_tools::ToolResult {
                            content: PermissionGate::permission_denied_message(
                                &call.name,
                                &call.arguments,
                            ),
                            is_error: true,
                        },
                        None,
                    ))
                } else {
                    None
                }
            };

            let schedule = if immediate.is_some() {
                None
            } else {
                Some((
                    classify_tool_accesses(&call.name, &call.arguments),
                    call.name.clone(),
                    call.arguments.clone(),
                ))
            };

            prepared.push(PreparedTool {
                call: call.clone(),
                immediate,
                schedule,
            });
        }

        let tools = Arc::clone(&self.tools);
        let mut scheduled = Vec::new();
        for item in &prepared {
            if let Some((accesses, name, arguments)) = item.schedule.as_ref() {
                let ctx = ToolContext {
                    sandbox: Arc::clone(&ctx.sandbox),
                    cancel: ctx.cancel.clone(),
                    subagent: ctx.subagent.clone(),
                    permission: ctx.permission.clone(),
                    plan_mode: ctx.plan_mode.clone(),
                    todos: ctx.todos.clone(),
                    ask_user: ctx.ask_user.clone(),
                    background: ctx.background.clone(),
                };
                let tools = Arc::clone(&tools);
                let name = name.clone();
                let arguments = arguments.clone();
                let future: std::pin::Pin<
                    Box<dyn std::future::Future<Output = (zene_tools::ToolResult, Option<u64>)> + Send>,
                > = Box::pin(async move {
                    let started = Instant::now();
                    let result = tools
                        .execute(&name, &arguments, &ctx)
                        .await
                        .unwrap_or_else(|err| zene_tools::ToolResult {
                            content: err.to_string(),
                            is_error: true,
                        });
                    (result, Some(started.elapsed().as_millis() as u64))
                });
                scheduled.push((accesses.clone(), future));
            }
        }

        let scheduled_results = ToolScheduler::run_ordered(scheduled).await;
        let mut scheduled_iter = scheduled_results.into_iter();

        for item in prepared {
            let (result, duration_ms) = if let Some(immediate) = item.immediate {
                immediate
            } else {
                scheduled_iter
                    .next()
                    .expect("missing scheduled tool result")
            };

            let call = item.call;

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
                bound_tool_output(&workdir, &call.name, result.content)
            };

            if let Some(reminder) = self.tool_dedup.on_call(&call.name, &call.arguments) {
                content = append_reminder(&content, reminder);
            }

            emit_event(
                &options.event_handler,
                AgentEvent::ToolResult {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    content: content.clone(),
                    is_error: result.is_error,
                    duration_ms,
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

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}...", input.chars().take(max).collect::<String>())
    }
}

/// Plan output bounds (pure) then spill via filesystem store (runtime IO).
fn bound_tool_output(workdir: &std::path::Path, tool_name: &str, content: String) -> String {
    let plan = plan_tool_output_bound(content, tool_name);
    let store = FsToolOutputStore::new(workdir);
    apply_tool_bound_plan(plan, &store)
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
