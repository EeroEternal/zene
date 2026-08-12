//! Context orchestration: estimate, compact, memory, prefire, epoch.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};
use zene_llm::{ChatClient, ContextMetadata, Message, TokenUsage, ToolDefinition};

use crate::assemble::{assemble_outbound, delivery_mode_from_env, stable_system_boundary, DeliveryMode};
use crate::compaction::{
    apply_overflow_truncate_pass, apply_steps_truncate_pass, compact_session,
    compact_session_forced, is_context_overflow_error, CompactionResult,
};
use crate::config::CompactionConfig;
use crate::context_water::ContextWaterLevel;
use crate::event_handler::{ContextEventHandler, EventOutcome};
use crate::events::ContextEvent;
use crate::hooks::ContextHooks;
#[cfg(feature = "memory")]
use crate::memory;
#[cfg(not(feature = "memory"))]
use crate::memory_stub as memory;
#[cfg(feature = "prefire")]
use crate::prefire::{self, PrefireCache, PrefireState};
#[cfg(not(feature = "prefire"))]
use crate::prefire_stub::{self, PrefireCache, PrefireState};
use crate::session::ContextSession;
use crate::tokens::{self, TokenEstimator};
#[cfg(feature = "gateway")]
use crate::gateway::gateway_configured;
#[cfg(not(feature = "gateway"))]
use crate::gateway_stub::gateway_configured;
use crate::two_pass;

/// Builds a dedicated client for prefire pass1 (runtime-provided; avoids `ZeneConfig` in this crate).
pub type PrefireClientFactory = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<ChatClient>> + Send>> + Send + Sync,
>;

/// Outbound view for one LLM step after context preparation.
#[derive(Debug, Clone)]
pub struct StepContext {
    pub messages: Vec<Message>,
    pub metadata: ContextMetadata,
    pub estimate_tokens: u32,
}

/// Read-only decision before context mutations are committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextObservation {
    pub estimated_tokens: u32,
    pub should_compact: bool,
    pub preflight_overflow: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionExplain {
    pub source_message_count: usize,
    pub projected_message_count: usize,
    pub source_event_count: usize,
    pub used_materialized_fallback: bool,
    pub estimate_tokens: u32,
    pub context_epoch: u64,
}

/// Result of [`ContextEngine::prepare_step`].
#[derive(Debug, Clone)]
pub struct PrepareStepResult {
    pub step: StepContext,
    pub compaction: Option<CompactionResult>,
    pub events: Vec<ContextEvent>,
    pub explain: ProjectionExplain,
}

#[derive(Debug, Clone)]
struct CommitResult {
    compaction: Option<CompactionResult>,
    events: Vec<ContextEvent>,
}

/// Result of [`ContextEngine::compact_forced`].
#[derive(Debug, Clone)]
pub struct ForcedCompactResult {
    pub compaction: Option<CompactionResult>,
    pub events: Vec<ContextEvent>,
}

/// Result of [`ContextEngine::handle_overflow`].
#[derive(Debug, Clone)]
pub struct OverflowHandleResult {
    pub retry: bool,
    pub compaction: Option<CompactionResult>,
    pub events: Vec<ContextEvent>,
}

/// Dependencies passed into context operations (session + runtime config).
pub struct ContextDeps<'a> {
    pub session: &'a mut dyn ContextSession,
    pub compaction_config: &'a CompactionConfig,
    pub model: &'a str,
    pub client: &'a ChatClient,
    pub hooks: Option<&'a dyn ContextHooks>,
    pub system_prompt: &'a str,
    pub estimator: &'a TokenEstimator,
    pub handler: &'a mut dyn ContextEventHandler,
    #[cfg(feature = "prefire")]
    pub prefire_client_factory: Option<PrefireClientFactory>,
}

/// Semantic context authority for one agent session.
pub struct ContextEngine {
    water: ContextWaterLevel,
    prefire: PrefireState,
    epoch: u64,
    last_memory_flush_compaction: u64,
    external_session_id: Option<String>,
    pending_publish: bool,
    gateway_prefix_len: usize,
    initial_publish_done: bool,
}

impl ContextEngine {
    pub fn new(context_window_tokens: u32) -> Self {
        Self {
            water: ContextWaterLevel::new(context_window_tokens),
            prefire: PrefireState::new(),
            epoch: 0,
            last_memory_flush_compaction: 0,
            external_session_id: None,
            pending_publish: false,
            gateway_prefix_len: 0,
            initial_publish_done: false,
        }
    }

    /// Override session id for inference linkage (e.g. Cloud run_id).
    pub fn set_external_session_id(&mut self, id: Option<String>) {
        self.external_session_id = id;
    }

    pub fn water(&self) -> &ContextWaterLevel {
        &self.water
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn gateway_prefix_len(&self) -> usize {
        self.gateway_prefix_len
    }

    pub fn set_window(&mut self, context_window_tokens: u32) {
        self.water.set_window(context_window_tokens);
    }

    pub fn clear_prefire(&self) {
        self.prefire.clear();
    }

    pub fn prefire_has_cache(&self) -> bool {
        self.prefire.has_cache()
    }

    pub fn prefire_in_flight(&self) -> bool {
        self.prefire.is_in_flight()
    }

    pub fn restore_water_from_session(&mut self, tokens: u32) {
        self.water.last_prompt_tokens = Some(tokens);
        self.water.last_estimate_tokens = Some(tokens);
    }

    pub fn metadata(&self, session: &dyn ContextSession) -> ContextMetadata {
        let session_id = self.session_id_for(session);
        ContextMetadata::new(session_id, self.epoch)
    }

    fn metadata_for_outbound(
        &self,
        session: &dyn ContextSession,
        assembled: &crate::assemble::AssembledOutbound,
    ) -> ContextMetadata {
        let mut meta = self.metadata(session);
        meta.prefix_hash = assembled.prefix_hash.clone();
        meta.delivery = match assembled.mode {
            DeliveryMode::Full => zene_llm::ContextDelivery::Full,
            DeliveryMode::Delta => zene_llm::ContextDelivery::Delta,
        };
        meta.tail_start = assembled.tail_start;
        meta
    }

    pub fn on_system_prefix_changed(&mut self, reason: &'static str) -> ContextEvent {
        let old = self.epoch;
        self.epoch = self.epoch.saturating_add(1);
        self.pending_publish = true;
        ContextEvent::EpochBumped {
            old,
            new: self.epoch,
            reason,
        }
    }

    /// Re-assemble outbound view after session mutation (e.g. overflow compact).
    pub fn assemble_step(
        &self,
        session: &dyn ContextSession,
        tools: &[ToolDefinition],
        estimator: &TokenEstimator,
    ) -> StepContext {
        self.project(session, tools, estimator)
    }

    /// Observe session pressure without mutating the session.
    pub fn observe(
        &mut self,
        session: &dyn ContextSession,
        tools: &[ToolDefinition],
        estimator: &TokenEstimator,
        config: &CompactionConfig,
    ) -> ContextObservation {
        let view = session.view();
        let estimated_tokens = tokens::estimate_context(&view.messages, tools, estimator) as u32;
        self.water.record_estimate(estimated_tokens);
        self.water.set_window(config.context_window_tokens);
        let preflight_overflow = self.water.exceeds_window() && !self.water.auto_compact_suppressed;
        ContextObservation {
            estimated_tokens,
            should_compact: self.water.should_compact(config) || preflight_overflow,
            preflight_overflow,
        }
    }

    /// Prepare messages before an LLM step through observe → commit → project.
    pub async fn prepare_step(
        &mut self,
        deps: &mut ContextDeps<'_>,
        tools: &[ToolDefinition],
    ) -> Result<PrepareStepResult> {
        let observation = self.observe(
            deps.session,
            tools,
            deps.estimator,
            deps.compaction_config,
        );
        let commit = self.commit(deps, tools, &observation).await?;
        let step = self.project(deps.session, tools, deps.estimator);
        let view = deps.session.view();
        let explain = ProjectionExplain {
            source_message_count: view.messages.len(),
            projected_message_count: step.messages.len(),
            source_event_count: view.source_event_count,
            used_materialized_fallback: view.used_materialized_fallback,
            estimate_tokens: step.estimate_tokens,
            context_epoch: step.metadata.context_epoch,
        };
        Ok(PrepareStepResult {
            step,
            compaction: commit.compaction,
            events: commit.events,
            explain,
        })
    }

    async fn commit(
        &mut self,
        deps: &mut ContextDeps<'_>,
        tools: &[ToolDefinition],
        observation: &ContextObservation,
    ) -> Result<CommitResult> {
        let mut events = Vec::new();
        self.flush_pending_publish(deps, &mut events).await?;
        self.ensure_initial_publish(deps, &mut events).await?;
        self.maybe_start_prefire(deps, tools);

        if !observation.should_compact {
            return Ok(CommitResult { compaction: None, events });
        }
        if apply_steps_truncate_pass(deps.session, deps.compaction_config) {
            self.sync_water_from_estimate(
                deps.session,
                tools,
                deps.estimator,
                deps.compaction_config,
            );
            if !self.water.should_compact(deps.compaction_config)
                && !self.water.exceeds_window()
            {
                return Ok(CommitResult { compaction: None, events });
            }
        }

        self.prefire.await_in_flight().await;
        let prefire_cache = self.prefire_cache_for_session(deps.session);
        self.maybe_flush_memory(deps, &mut events).await?;
        let memory_block = deps.handler.memory_reminder();
        Self::emit(
            deps,
            &mut events,
            ContextEvent::Checkpoint { reason: "pre_auto_compact" },
        )
        .await?;
        let reason = if observation.preflight_overflow {
            "preflight_overflow"
        } else {
            "token_threshold"
        };
        let compaction = match compact_session(
            deps.session,
            deps.client,
            deps.model,
            deps.compaction_config,
            reason,
            tools,
            deps.estimator,
            deps.hooks,
            prefire_cache.as_ref(),
            memory_block.as_deref(),
        )
        .await
        {
            Ok(Some(result)) => {
                Self::emit_compaction_segments(deps, &mut events, &result).await?;
                self.prefire.clear();
                self.water.clear_auto_compact_suppression();
                self.bump_epoch_and_publish("compaction", deps, &mut events)
                    .await?;
                self.sync_water_from_estimate(
                    deps.session,
                    tools,
                    deps.estimator,
                    deps.compaction_config,
                );
                Self::emit(
                    deps,
                    &mut events,
                    ContextEvent::Checkpoint { reason: "post_auto_compact" },
                )
                .await?;
                Some(result)
            }
            Ok(None) => None,
            Err(err) => {
                warn!(error = %err, "auto-compact failed; suppressing until /compact");
                self.water.suppress_auto_compact();
                None
            }
        };
        deps.session.ensure_system_message(deps.system_prompt);
        Ok(CommitResult { compaction, events })
    }

    pub fn record_step_usage(
        &mut self,
        usage: &TokenUsage,
        session: &mut dyn ContextSession,
        tools: &[ToolDefinition],
        estimator: &TokenEstimator,
        compaction_config: &CompactionConfig,
    ) {
        self.water.record_usage(usage);
        if let Some(cached) = usage.cached_tokens {
            let effective = self.water.effective_tokens();
            if effective > 0 {
                tracing::info!(
                    cached_tokens = cached,
                    prompt_tokens = usage.prompt_tokens,
                    effective_tokens = effective,
                    cache_pct = (cached * 100 / u64::from(effective.max(1))),
                    "provider prompt cache usage"
                );
            } else {
                tracing::debug!(cached_tokens = cached, "provider cache usage");
            }
        }
        let view = session.view();
        let estimated = tokens::estimate_context(&view.messages, tools, estimator) as u32;
        session.update_context_usage(
            self.water.effective_tokens().max(estimated),
            compaction_config.context_window_tokens,
        );
    }

    pub async fn compact_forced(
        &mut self,
        deps: &mut ContextDeps<'_>,
        tools: &[ToolDefinition],
        user_hint: Option<&str>,
    ) -> Result<ForcedCompactResult> {
        let mut events = Vec::new();
        self.prefire.await_in_flight().await;
        let prefire_cache = self.prefire_cache_for_session(deps.session);
        self.maybe_flush_memory(deps, &mut events).await?;
        let memory_block = deps.handler.memory_reminder();
        Self::emit(
            deps,
            &mut events,
            ContextEvent::Checkpoint {
                reason: "pre_manual_compact",
            },
        )
        .await?;
        let result = compact_session_forced(
            deps.session,
            deps.client,
            deps.model,
            deps.compaction_config,
            "manual",
            tools,
            deps.estimator,
            true,
            user_hint,
            deps.hooks,
            prefire_cache.as_ref(),
            memory_block.as_deref(),
        )
        .await;
        self.prefire.clear();
        match result {
            Ok(compaction) => {
                self.water.clear_auto_compact_suppression();
                if compaction.is_some() {
                    if let Some(ref result) = compaction {
                        Self::emit_compaction_segments(deps, &mut events, result).await?;
                    }
                    self.bump_epoch_and_publish("manual_compaction", deps, &mut events)
                        .await?;
                    self.sync_water_from_estimate(
                        deps.session,
                        tools,
                        deps.estimator,
                        deps.compaction_config,
                    );
                    Self::emit(
                        deps,
                        &mut events,
                        ContextEvent::Checkpoint {
                            reason: "post_manual_compact",
                        },
                    )
                    .await?;
                }
                deps.session.ensure_system_message(deps.system_prompt);
                Ok(ForcedCompactResult {
                    compaction,
                    events,
                })
            }
            Err(err) => {
                self.water.suppress_auto_compact();
                Err(err)
            }
        }
    }

    /// Handle provider context-overflow: truncate then full compact. Returns true if retry.
    pub async fn handle_overflow(
        &mut self,
        deps: &mut ContextDeps<'_>,
        tools: &[ToolDefinition],
        overflow_truncated: &mut bool,
        overflow_summarized: &mut bool,
    ) -> Result<OverflowHandleResult> {
        let mut events = Vec::new();
        if !*overflow_truncated {
            *overflow_truncated = true;
            if apply_overflow_truncate_pass(deps.session, deps.compaction_config, deps.estimator) {
                info!("context overflow: applied truncate pass before retry");
                deps.session.ensure_system_message(deps.system_prompt);
                return Ok(OverflowHandleResult {
                    retry: true,
                    compaction: None,
                    events,
                });
            }
        }
        if !*overflow_summarized {
            *overflow_summarized = true;
            self.prefire.await_in_flight().await;
            let prefire_cache = self.prefire_cache_for_session(deps.session);
            self.maybe_flush_memory(deps, &mut events).await?;
            let memory_block = deps.handler.memory_reminder();
            Self::emit(
                deps,
                &mut events,
                ContextEvent::Checkpoint {
                    reason: "pre_overflow_compact",
                },
            )
            .await?;
            match compact_session(
                deps.session,
                deps.client,
                deps.model,
                deps.compaction_config,
                "context_overflow",
                tools,
                deps.estimator,
                deps.hooks,
                prefire_cache.as_ref(),
                memory_block.as_deref(),
            )
            .await
            {
                Ok(Some(result)) => {
                    Self::emit_compaction_segments(deps, &mut events, &result).await?;
                    self.prefire.clear();
                    self.water.clear_auto_compact_suppression();
                    self.bump_epoch_and_publish("overflow_compaction", deps, &mut events)
                        .await?;
                    self.sync_water_from_estimate(
                        deps.session,
                        tools,
                        deps.estimator,
                        deps.compaction_config,
                    );
                    Self::emit(
                        deps,
                        &mut events,
                        ContextEvent::Checkpoint {
                            reason: "post_overflow_compact",
                        },
                    )
                    .await?;
                    deps.session.ensure_system_message(deps.system_prompt);
                    return Ok(OverflowHandleResult {
                        retry: true,
                        compaction: Some(result),
                        events,
                    });
                }
                Ok(None) => {
                    deps.session.ensure_system_message(deps.system_prompt);
                    return Ok(OverflowHandleResult {
                        retry: true,
                        compaction: None,
                        events,
                    });
                }
                Err(err) => {
                    warn!(error = %err, "overflow compact failed");
                    self.water.suppress_auto_compact();
                    return Err(err);
                }
            }
        }
        Ok(OverflowHandleResult {
            retry: false,
            compaction: None,
            events,
        })
    }

    pub fn is_context_overflow_error(err: &anyhow::Error) -> bool {
        is_context_overflow_error(err)
    }

    fn project(
        &self,
        session: &dyn ContextSession,
        tools: &[ToolDefinition],
        estimator: &TokenEstimator,
    ) -> StepContext {
        let mode = delivery_mode_from_env();
        let view = session.view();
        let assembled = assemble_outbound(&view.messages, self.gateway_prefix_len, mode);
        let metadata = self.metadata_for_outbound(session, &assembled);
        let estimate_tokens =
            tokens::estimate_context(&assembled.messages, tools, estimator) as u32;
        StepContext {
            messages: assembled.messages,
            metadata,
            estimate_tokens,
        }
    }

    fn sync_water_from_estimate(
        &mut self,
        session: &mut dyn ContextSession,
        tools: &[ToolDefinition],
        estimator: &TokenEstimator,
        compaction_config: &CompactionConfig,
    ) {
        let view = session.view();
        let estimated = tokens::estimate_context(&view.messages, tools, estimator) as u32;
        self.water.record_estimate(estimated);
        self.water.last_prompt_tokens = Some(estimated);
        session.update_context_usage(estimated, compaction_config.context_window_tokens);
    }

    fn session_id_for(&self, session: &dyn ContextSession) -> String {
        self.external_session_id
            .clone()
            .unwrap_or_else(|| session.session_id().to_string())
    }

    async fn emit(
        deps: &mut ContextDeps<'_>,
        events: &mut Vec<ContextEvent>,
        event: ContextEvent,
    ) -> Result<EventOutcome> {
        if let ContextEvent::Checkpoint { reason } = &event {
            deps.session.persist_checkpoint(reason)?;
            events.push(event);
            return Ok(EventOutcome::Void);
        }
        let outcome = deps.handler.handle(&event).await?;
        events.push(event);
        Ok(outcome)
    }

    async fn emit_compaction_segments(
        deps: &mut ContextDeps<'_>,
        events: &mut Vec<ContextEvent>,
        result: &CompactionResult,
    ) -> Result<()> {
        if let Some(segment) = &result.segment {
            Self::emit(
                deps,
                events,
                ContextEvent::CompactionSegment {
                    session_id: segment.session_id.clone(),
                    body: segment.body.clone(),
                },
            )
            .await?;
        }
        Ok(())
    }

    async fn flush_pending_publish(
        &mut self,
        deps: &mut ContextDeps<'_>,
        events: &mut Vec<ContextEvent>,
    ) -> Result<()> {
        if !self.pending_publish {
            return Ok(());
        }
        self.pending_publish = false;
        let view = deps.session.view();
        self.gateway_prefix_len = view.messages.len();
        let session_id = self.session_id_for(deps.session);
        Self::emit(
            deps,
            events,
            ContextEvent::PublishPrefix {
                session_id,
                epoch: self.epoch,
                messages: view.messages,
            },
        )
        .await?;
        Ok(())
    }

    async fn ensure_initial_publish(
        &mut self,
        deps: &mut ContextDeps<'_>,
        events: &mut Vec<ContextEvent>,
    ) -> Result<()> {
        if self.initial_publish_done || !gateway_configured() {
            return Ok(());
        }
        self.initial_publish_done = true;
        let view = deps.session.view();
        self.gateway_prefix_len = view.messages.len();
        let session_id = self.session_id_for(deps.session);
        let pinned_boundary = stable_system_boundary(&view.messages);
        info!(
            session_id = %session_id,
            epoch = self.epoch,
            messages = view.messages.len(),
            pinned_boundary,
            "initial gateway prefix publish"
        );
        Self::emit(
            deps,
            events,
            ContextEvent::PublishPrefix {
                session_id,
                epoch: self.epoch,
                messages: view.messages,
            },
        )
        .await?;
        Ok(())
    }

    async fn bump_epoch_and_publish(
        &mut self,
        reason: &'static str,
        deps: &mut ContextDeps<'_>,
        events: &mut Vec<ContextEvent>,
    ) -> Result<()> {
        let old = self.epoch;
        self.epoch = self.epoch.saturating_add(1);
        let view = deps.session.view();
        self.gateway_prefix_len = view.messages.len();
        info!(
            old,
            new = self.epoch,
            reason,
            gateway_prefix_len = self.gateway_prefix_len,
            "context epoch bumped"
        );
        let session_id = self.session_id_for(deps.session);
        Self::emit(
            deps,
            events,
            ContextEvent::PublishPrefix {
                session_id,
                epoch: self.epoch,
                messages: view.messages,
            },
        )
        .await?;
        Ok(())
    }

    fn prefire_cache_for_session(&self, session: &dyn ContextSession) -> Option<PrefireCache> {
        let view = session.view();
        let prefix_start = if view
            .messages
            .first()
            .is_some_and(|m| m.role == zene_llm::Role::System)
        {
            1
        } else {
            0
        };
        let body = &view.messages[prefix_start..];
        self.prefire.valid_cache_for(body)
    }

    fn maybe_start_prefire(&self, deps: &ContextDeps<'_>, _tools: &[ToolDefinition]) {
        #[cfg(feature = "prefire")]
        {
            let Some(factory) = deps.prefire_client_factory.as_ref() else {
                return;
            };
            let lead = prefire::prefire_lead_percent();
            if !self.water.should_prefire(deps.compaction_config, lead) {
                return;
            }
            if self.prefire.is_in_flight() || self.prefire.has_cache() {
                return;
            }

            let messages = deps.session.view().messages;
            let prefix_start = if messages
                .first()
                .is_some_and(|m| m.role == zene_llm::Role::System)
            {
                1
            } else {
                0
            };
            if messages.len().saturating_sub(prefix_start) < 8 {
                return;
            }
            let body = messages[prefix_start..].to_vec();
            let split = two_pass::split_messages_for_two_pass(
                &body,
                deps.estimator,
                two_pass::TWO_PASS_DEFAULT_SPLIT_FRACTION,
            );
            if split.split_idx == 0 || split.split_idx >= body.len() {
                return;
            }
            let pass1_prefix = body[..split.split_idx].to_vec();
            let fingerprint = two_pass::fingerprint_messages(&pass1_prefix);
            if self.prefire.already_launched_for(fingerprint) {
                return;
            }

            let factory = factory.clone();
            let model = deps.model.to_string();
            let window = deps.compaction_config.context_window_tokens;
            let split_idx = split.split_idx;
            let estimator = *deps.estimator;
            info!(
                split_idx,
                usage_percent = self.water.usage_percent(),
                "starting prefire pass1"
            );
            let handle = tokio::spawn(async move {
                let client = match factory().await {
                    Ok(c) => c,
                    Err(err) => {
                        warn!(error = %err, "prefire: failed to create client");
                        return None;
                    }
                };
                match crate::compaction::summarize_messages_with_ladder(
                    &client,
                    &model,
                    &pass1_prefix,
                    Some(window),
                    &estimator,
                )
                .await
                {
                    Ok(note1) => {
                        if note1.trim().is_empty() {
                            return None;
                        }
                        info!(note1_chars = note1.len(), "prefire pass1 cached NOTE₁");
                        Some(PrefireCache {
                            note1,
                            fingerprint,
                            split_idx,
                        })
                    }
                    Err(err) => {
                        warn!(error = %err, "prefire pass1 failed");
                        None
                    }
                }
            });
            self.prefire.set_handle(fingerprint, handle);
        }
    }

    async fn maybe_flush_memory(
        &mut self,
        deps: &mut ContextDeps<'_>,
        events: &mut Vec<ContextEvent>,
    ) -> Result<()> {
        let cycle = deps.session.compaction_cycle();
        let marker = cycle.saturating_add(1);
        let threshold =
            ContextWaterLevel::auto_compact_threshold_percent(deps.compaction_config);
        if !memory::should_flush(
            self.water.usage_percent(),
            threshold,
            self.last_memory_flush_compaction == marker,
        ) {
            return Ok(());
        }
        let conversation = memory::format_flush_input(&deps.session.view().messages);
        if conversation.trim().is_empty() {
            return Ok(());
        }
        let outcome = Self::emit(
            deps,
            events,
            ContextEvent::MemoryFlush { conversation },
        )
        .await?;
        if let EventOutcome::MemoryFlush(memory::FlushResult::Accepted) = outcome {
            self.last_memory_flush_compaction = marker;
        }
        Ok(())
    }
}
