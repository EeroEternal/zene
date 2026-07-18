use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zene_config::ZeneConfig;
use zene_llm::{ChatClient, ChatRequest, Message, StreamEvent, TokenUsage, ToolCall};
use zene_sandbox::LocalSandbox;
use zene_session::{
    fork_session, latest_checkpoint_id, load_checkpoint, restore_checkpoint, save_checkpoint,
    AgentRecordWriter, RecordEntry, SessionRecord,
};
use zene_tools::{
    default_ask_user_prompter, shared_background_tasks, shared_todo_store_from,
    SharedAskUserPrompter, SharedBackgroundTasks, SharedTodoStore, shared_plan_mode, PlanModeState,
    SharedPlanMode, SharedToolPermission, SubagentEnv, ToolContext, ToolRegistry,
    DEFAULT_SUBAGENT_MAX_DEPTH,
};
use zene_mcp::McpManager;

mod compaction;
mod context_water;
mod events;
mod hooks;
mod input_ladder;
mod permission;
mod plan_mode;
mod skills;
mod subagent;
mod tokens;
mod tool_dedup;
pub mod tool_scheduler;
mod turn;
mod worktree;

use compaction::{
    apply_overflow_truncate_pass, compact_session, compact_session_forced, is_context_overflow_error,
};
pub use compaction::CompactionResult;
mod workspace;

pub use context_water::ContextWaterLevel;
pub use events::{emit_event, AgentEvent, EventHandler};
pub use hooks::{HookBlock, HookRunner};
pub use input_ladder::InputLadderStage;
pub use permission::{
    approve_tool_call, policy_denied, PermissionGate, PermissionMode, PermissionPrompter,
    PermissionRule, PromptChoice, RuleAction,
};
pub use plan_mode::PlanApprovalPrompter;
pub use subagent::{run_subagent, ChatBackend, CoreSubagentRunner};
pub use tokens::{estimate_context, TokenEstimator};
pub use turn::{SteerBuffer, StepId, TurnId, TurnState};
use plan_mode::{
    build_effective_system_prompt, default_plan_approval_prompter, handle_enter_plan_mode,
    handle_exit_plan_mode, tool_visible_in_definitions,
};
pub use tool_dedup::{append_reminder, ToolDedup};
pub use tool_scheduler::{classify_tool_accesses, ToolScheduler};
pub use worktree::ensure_session_worktree;

pub struct Agent {
    config: ZeneConfig,
    client: ChatClient,
    tools: Arc<ToolRegistry>,
    sandbox: Arc<LocalSandbox>,
    session: SessionRecord,
    turn_usage: TokenUsage,
    context_water: ContextWaterLevel,
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
        mut session: SessionRecord,
        permission_mode: PermissionMode,
    ) -> Result<Self> {
        let workdir = sandbox.workdir().to_path_buf();
        let system_prompt =
            workspace::build_system_prompt(&config.system_prompt, &workdir, config.include_workspace_context);
        session.ensure_system_message(&system_prompt);
        let client = ChatClient::from_config(&config).await?;
        let record_writer = AgentRecordWriter::for_session(&session.meta.id)?;

        let mut tools = zene_tools::agent_tools(config.agent_profile, config.web_search.clone());
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
        let todos = shared_todo_store_from(session.todos.clone());
        let context_water =
            ContextWaterLevel::new(config.compaction.context_window_tokens);
        let permission = shared_permission_with_rules(
            permission_mode,
            permission_rules_from_config(&config),
        );

        Ok(Self {
            config,
            client,
            tools: Arc::new(tools),
            sandbox: Arc::new(sandbox),
            session,
            turn_usage: TokenUsage::default(),
            context_water,
            active_turn: None,
            steer_buffer: Arc::new(Mutex::new(SteerBuffer::default())),
            system_prompt,
            permission,
            plan_mode: shared_plan_mode(),
            plan_approval: Arc::new(default_plan_approval_prompter),
            todos,
            ask_user: default_ask_user_prompter(),
            tool_dedup: ToolDedup::new(),
            hooks,
            record_writer,
            mcp,
            background: shared_background_tasks(),
        })
    }

    pub fn is_plan_mode_active(&self) -> bool {
        self.plan_mode
            .lock()
            .is_active()
    }

    pub fn enter_plan_mode(&mut self) {
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
        self.context_water
            .set_window(self.config.compaction.context_window_tokens);

        // Recreate the client
        self.client = zene_llm::ChatClient::from_config(&self.config).await?;
        self.config
            .persist_connection_settings()
            .context("save model settings to ~/.zene/config.toml")?;
        Ok(())
    }

    pub fn context_water(&self) -> &ContextWaterLevel {
        &self.context_water
    }

    /// Manually compact the conversation (`/compact [hint]`).
    pub async fn compact_now(&mut self, user_hint: Option<&str>) -> Result<Option<CompactionResult>> {
        let tools = self.tool_definitions_for_llm();
        let estimator = self.token_estimator();
        let _ = save_checkpoint(&self.session, "pre_manual_compact");
        let result = compact_session_forced(
            &mut self.session,
            &self.client,
            &self.config.model,
            &self.config.compaction,
            "manual",
            &tools,
            &estimator,
            true,
            user_hint,
        )
        .await?;
        if let Some(result) = &result {
            self.record_compaction(result)?;
            let _ = save_checkpoint(&self.session, "post_manual_compact");
            self.sync_context_water_from_estimate();
        }
        self.session.ensure_system_message(&self.system_prompt);
        self.session.save()?;
        Ok(result)
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
            self.context_water.last_prompt_tokens = Some(tokens);
            self.context_water.last_estimate_tokens = Some(tokens);
        }
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
        TokenEstimator::new(self.config.chars_per_token_for_model())
    }

    fn estimated_context_tokens(&self, messages: &[Message], tools: &[zene_llm::ToolDefinition]) -> usize {
        estimate_context(messages, tools, &self.token_estimator())
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
    pub fn set_permission_gate(&mut self, gate: PermissionGate) {
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
                self.context_water.record_usage(&usage);
                self.session.update_context_usage(
                    self.context_water.effective_tokens(),
                    self.config.compaction.context_window_tokens,
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
            return Err(turn::max_steps_error(max_steps));
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
        let estimated_tokens = self.estimated_context_tokens(&messages, &tools);
        self.context_water.record_estimate(estimated_tokens as u32);
        self.context_water
            .set_window(self.config.compaction.context_window_tokens);
        debug!(
            estimated_context_tokens = estimated_tokens,
            effective_tokens = self.context_water.effective_tokens(),
            usage_percent = self.context_water.usage_percent(),
            message_count = messages.len(),
            tool_count = tools.len(),
            chars_per_token = self.config.chars_per_token_for_model(),
            "llm request context water level"
        );
        self.warn_if_near_context_limit(self.context_water.effective_tokens() as usize);

        if self.context_water.should_compact(&self.config.compaction) {
            let _ = save_checkpoint(&self.session, "pre_auto_compact");
            let estimator = self.token_estimator();
            if let Some(result) = compact_session(
                &mut self.session,
                &self.client,
                &self.config.model,
                &self.config.compaction,
                "token_threshold",
                &tools,
                &estimator,
            )
            .await?
            {
                self.record_compaction(&result)?;
                let _ = save_checkpoint(&self.session, "post_auto_compact");
                self.sync_context_water_from_estimate();
            }
            self.session
                .ensure_system_message(&self.system_prompt);
        }
        Ok(())
    }

    fn sync_context_water_from_estimate(&mut self) {
        let messages = self.build_messages();
        let tools = self.tool_definitions_for_llm();
        let estimated = self.estimated_context_tokens(&messages, &tools) as u32;
        self.context_water.record_estimate(estimated);
        // After compaction, prefer the fresh estimate over stale provider usage.
        self.context_water.last_prompt_tokens = Some(estimated);
        self.session.update_context_usage(
            estimated,
            self.config.compaction.context_window_tokens,
        );
    }

    async fn run_llm_step(
        &mut self,
        tools: &[zene_llm::ToolDefinition],
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Message, Option<TokenUsage>)> {
        let mut overflow_truncated = false;
        let mut overflow_summarized = false;

        loop {
            if Self::check_cancelled(cancel)? {
                return Err(turn::aborted_error());
            }

            let messages = self.build_messages();
            let estimated_tokens = self.estimated_context_tokens(&messages, tools);
            debug!(
                estimated_context_tokens = estimated_tokens,
                message_count = messages.len(),
                "llm step context estimate"
            );
            self.warn_if_near_context_limit(estimated_tokens);

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
                Err(err) if is_context_overflow_error(&err) => {
                    if !overflow_truncated {
                        overflow_truncated = true;
                        let estimator = self.token_estimator();
                        if apply_overflow_truncate_pass(
                            &mut self.session,
                            &self.config.compaction,
                            &estimator,
                        ) {
                            info!("context overflow: applied truncate pass before retry");
                            self.session
                                .ensure_system_message(&self.system_prompt);
                            continue;
                        }
                    }
                    if !overflow_summarized {
                        overflow_summarized = true;
                        let _ = save_checkpoint(&self.session, "pre_overflow_compact");
                        let estimator = self.token_estimator();
                        if let Some(result) = compact_session(
                            &mut self.session,
                            &self.client,
                            &self.config.model,
                            &self.config.compaction,
                            "context_overflow",
                            tools,
                            &estimator,
                        )
                        .await?
                        {
                            self.record_compaction(&result)?;
                            let _ = save_checkpoint(&self.session, "post_overflow_compact");
                            self.sync_context_water_from_estimate();
                        }
                        self.session
                            .ensure_system_message(&self.system_prompt);
                        continue;
                    }
                    return Err(err);
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
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            );

            let immediate = if call.name == "EnterPlanMode" {
                let mut state = plan_mode.lock();
                let result = handle_enter_plan_mode(&mut state, &call.arguments);
                if !result.is_error {
                    self.sync_plan_mode_system();
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
                if !result.is_error {
                    self.sync_plan_mode_system();
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

fn shared_permission_with_rules(
    mode: PermissionMode,
    rules: Vec<PermissionRule>,
) -> SharedToolPermission {
    Arc::new(Mutex::new(PermissionGate::new(mode).with_rules(rules)))
}

fn permission_rules_from_config(config: &ZeneConfig) -> Vec<PermissionRule> {
    config
        .permission_rules
        .to_flat_rules()
        .into_iter()
        .filter_map(|rule| {
            let action = match rule.action.trim().to_lowercase().as_str() {
                "allow" => RuleAction::Allow,
                "deny" => RuleAction::Deny,
                "ask" => RuleAction::Ask,
                _ => return None,
            };
            Some(PermissionRule {
                pattern: rule.pattern,
                action,
            })
        })
        .collect()
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
            duration_ms: _,
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
        AgentEvent::TurnStart { .. } | AgentEvent::TextDelta { .. } | AgentEvent::SteerInput { .. } => None,
    }
}
