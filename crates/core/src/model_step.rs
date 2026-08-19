//! Main-agent model-step orchestration extracted from [`crate::Agent`].
//!
//! Wave 14: the default turn path still enters via Agent wiring, but the
//! overflow-retry / stream assembly loop lives here (parallel to
//! [`crate::tool_executor::DefaultToolExecutor`] for tools). Request type
//! remains [`zene_model_executor::ModelRequest`]; providers still see `ChatRequest`
//! inside [`zene_model_executor::ChatClientExecutor`].

use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use zene_config::ZeneConfig;
use zene_context::{
    ContextEngine, ContextModel, EstimateProvider, PrefireClientFactory, StepContext,
    TokenEstimator,
};
use zene_llm::{ChatClient, Message, StreamEvent, TokenUsage, ToolDefinition};
use zene_model_executor::{ModelExecutor, ModelRequest};
use zene_session::{AgentRecordWriter, RecordEntry, SessionRecord};
use zene_tools::{SharedBackgroundTasks, SharedTodoStore};
use zene_turn::PreparedContext;

use crate::context_config;
use crate::context_events::AgentContextHandler;
use crate::context_hooks::ZeneContextHooks;
use crate::events::{emit_event, AgentEvent};
use crate::model_executor;
use crate::PromptOptions;

/// Mutable Agent pieces needed to run one model step (including overflow recovery).
pub(crate) struct ModelStepDeps<'a> {
    pub config: &'a ZeneConfig,
    pub model_executor: &'a dyn ModelExecutor,
    pub context_model: &'a dyn ContextModel,
    pub context: &'a mut ContextEngine,
    pub session: &'a mut SessionRecord,
    pub system_prompt: &'a str,
    pub workdir: &'a Path,
    pub background: &'a SharedBackgroundTasks,
    pub todos: &'a SharedTodoStore,
    pub plan_mode_active: bool,
    pub record_writer: &'a AgentRecordWriter,
}

/// Run a prepared model step: overflow-retry loop + stream/complete via [`ModelExecutor`].
pub(crate) async fn run_model_step(
    deps: ModelStepDeps<'_>,
    context: PreparedContext,
    options: &PromptOptions,
    cancel: Option<&CancellationToken>,
) -> Result<zene_turn::StepResult> {
    let tools = context.tools.clone();
    let step = StepContext {
        estimate_tokens: context.estimate_tokens.unwrap_or(0),
        metadata: context.metadata.unwrap_or_default(),
        messages: context.messages,
    };
    let (assistant_message, usage) = run_llm_step(deps, &step, &tools, options, cancel)
        .await
        .context("llm step")?;
    let had_tool_calls = assistant_message
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty());
    Ok(zene_turn::StepResult {
        message: assistant_message,
        usage,
        had_tool_calls,
    })
}

async fn run_llm_step(
    mut deps: ModelStepDeps<'_>,
    step: &StepContext,
    tools: &[ToolDefinition],
    options: &PromptOptions,
    cancel: Option<&CancellationToken>,
) -> Result<(Message, Option<TokenUsage>)> {
    let mut overflow_state = model_executor::OverflowRetryState::default();
    let mut messages = step.messages.clone();
    let mut metadata = step.metadata.clone();

    loop {
        if check_cancelled(cancel)? {
            return Err(zene_turn::aborted_error());
        }

        debug!(
            estimated_context_tokens = step.estimate_tokens,
            message_count = messages.len(),
            "llm step context estimate"
        );

        let request = model_executor::build_request(
            &deps.config.model,
            messages.clone(),
            tools.to_vec(),
            options.stream,
            Some(metadata.clone()),
        );

        let result = if options.stream {
            run_streaming_step(deps.model_executor, request, options, cancel).await
        } else {
            deps.model_executor
                .complete(request)
                .await
                .map(|response| (response.message, response.usage))
        };

        match result {
            Ok(value) => return Ok(value),
            Err(err) if ContextEngine::is_context_overflow_error(&err) => {
                if let Some(refreshed) =
                    recover_overflow(&mut deps, tools, &mut overflow_state).await?
                {
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

async fn recover_overflow(
    deps: &mut ModelStepDeps<'_>,
    tools: &[ToolDefinition],
    overflow_state: &mut model_executor::OverflowRetryState,
) -> Result<Option<StepContext>> {
    sync_todos_to_session(deps.todos, deps.session);
    let estimator = token_estimator(deps.config);
    let background_tasks = deps.background.lock().list();
    let hooks = ZeneContextHooks::new(deps.session, &background_tasks, deps.plan_mode_active);
    let compaction_config = context_config::context_compaction_config(&deps.config.compaction);
    let mut handler =
        AgentContextHandler::new(deps.context_model, &deps.config.model, deps.workdir);
    let prefire_factory = prefire_client_factory(deps.config);
    let (mut overflow_truncated, mut overflow_summarized) = overflow_state.flags();
    let overflow = {
        let mut context_deps = crate::make_context_deps(
            deps.session,
            &compaction_config,
            &deps.config.model,
            deps.context_model,
            Some(&hooks),
            deps.system_prompt,
            &estimator,
            &mut handler,
            prefire_factory,
        );
        deps.context
            .handle_overflow(
                &mut context_deps,
                tools,
                &mut overflow_truncated,
                &mut overflow_summarized,
            )
            .await?
    };
    overflow_state.set_flags(overflow_truncated, overflow_summarized);
    if let Some(result) = &overflow.compaction {
        record_compaction(deps.record_writer, result)?;
    }
    overflow
        .retry
        .then(|| {
            deps.context
                .try_assemble_step(deps.session, tools, &estimator)
        })
        .transpose()
}

pub(crate) async fn run_streaming_step(
    executor: &dyn ModelExecutor,
    request: ModelRequest,
    options: &PromptOptions,
    cancel: Option<&CancellationToken>,
) -> Result<(Message, Option<TokenUsage>)> {
    if check_cancelled(cancel)? {
        return Err(zene_turn::aborted_error());
    }

    let mut stream = executor.stream(request).await?;
    let mut accumulator = model_executor::StreamAccumulator::default();

    while let Some(event) = stream.next().await {
        if check_cancelled(cancel)? {
            return Err(zene_turn::aborted_error());
        }
        let event = event.context("stream event")?;
        match &event {
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
            }
            StreamEvent::ThoughtDelta(delta) => {
                emit_event(
                    &options.event_handler,
                    AgentEvent::ThoughtDelta {
                        delta: delta.clone(),
                    },
                );
            }
            StreamEvent::ToolCallDelta { .. } => {}
            StreamEvent::Done { .. } => {}
        }
        if accumulator.apply(&event) {
            break;
        }
    }

    if accumulator.has_text() && !options.quiet {
        println!();
    }

    Ok(accumulator.finish())
}

fn sync_todos_to_session(todos: &SharedTodoStore, session: &mut SessionRecord) {
    let store = todos.lock();
    session.todos = store.to_items();
}

fn token_estimator(config: &ZeneConfig) -> TokenEstimator {
    TokenEstimator::for_provider(
        EstimateProvider::from_name(&config.provider),
        &config.model,
        config.chars_per_token_for_model(),
    )
}

fn prefire_client_factory(config: &ZeneConfig) -> Option<PrefireClientFactory> {
    let config = config.clone();
    Some(Arc::new(move || {
        let config = config.clone();
        Box::pin(async move {
            let client = ChatClient::from_config(&config).await?;
            Ok(Arc::new(client) as Arc<dyn ContextModel>)
        })
    }))
}

fn record_compaction(
    record_writer: &AgentRecordWriter,
    result: &zene_context::CompactionResult,
) -> Result<()> {
    record_writer.append(&RecordEntry::Compaction {
        reason: result.reason.clone(),
        compacted_count: result.compacted_count,
        tokens_before: Some(result.stats.tokens_before),
        tokens_after: Some(result.stats.tokens_after),
        ts: chrono::Utc::now(),
    })
}

fn check_cancelled(cancel: Option<&CancellationToken>) -> Result<bool> {
    Ok(zene_turn::is_cancelled(cancel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use zene_model_executor::ModelStream;

    struct TextThenDone;

    #[async_trait]
    impl ModelExecutor for TextThenDone {
        async fn complete(
            &self,
            _request: ModelRequest,
        ) -> Result<zene_model_executor::ModelResponse> {
            unreachable!("complete not used")
        }

        async fn stream(&self, _request: ModelRequest) -> Result<ModelStream> {
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(StreamEvent::TextDelta("hi".into())),
                Ok(StreamEvent::Done { usage: None }),
            ])))
        }
    }

    #[tokio::test]
    async fn streaming_step_assembles_text_deltas() {
        let options = PromptOptions {
            stream: true,
            quiet: true,
            ..PromptOptions::default()
        };
        let request = ModelRequest {
            model: "test".into(),
            messages: vec![],
            tools: vec![],
            stream: true,
            context: None,
        };
        let (message, _) = run_streaming_step(&TextThenDone, request, &options, None)
            .await
            .expect("stream");
        assert_eq!(message.content.as_deref(), Some("hi"));
    }
}
