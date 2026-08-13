use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zene_llm::ToolCall;
use zene_permission::{PermissionGate, PermissionMode, SharedApprovalBroker};
use zene_tool_runtime::{apply_tool_bound_plan, plan_tool_output_bound, FsToolOutputStore};
use zene_tools::{
    default_ask_user_prompter, shared_background_tasks, shared_plan_mode, shared_todo_store,
    PlanModeState, SharedAskUserPrompter, SharedBackgroundTasks, SharedPlanMode, SharedTodoStore,
    SharedToolPermission, SubagentEnv, ToolContext, ToolRegistry,
};
use zene_turn::ToolBatchOutcome;

use crate::events::{emit_event, AgentEvent};
use crate::plan_mode::{handle_enter_plan_mode, handle_exit_plan_mode, PlanApprovalPrompter};
use crate::tool_dedup::{append_reminder, ToolDedup};
use crate::tool_scheduler::{classify_tool_accesses, ToolAccesses, ToolScheduler};
use crate::PromptOptions;
use zene_hooks::HookRunner;
use zene_sandbox::Sandbox;

/// Dependencies needed to execute one tool batch without borrowing the Agent.
pub(crate) struct ToolExecutorDeps<'a> {
    pub tools: Arc<ToolRegistry>,
    pub sandbox: Arc<dyn Sandbox>,
    pub permission: zene_tools::SharedToolPermission,
    pub approval_broker: Option<zene_permission::SharedApprovalBroker>,
    pub plan_mode: SharedPlanMode,
    pub plan_approval: &'a PlanApprovalPrompter,
    pub todos: SharedTodoStore,
    pub ask_user: SharedAskUserPrompter,
    pub background: SharedBackgroundTasks,
    pub subagent: Option<SubagentEnv>,
    pub hooks: &'a HookRunner,
}

pub(crate) struct ToolBatchResult {
    pub outcome: ToolBatchOutcome,
    pub mode_changes: Vec<String>,
    pub permission_decisions: Vec<PermissionDecision>,
    pub messages: Vec<ToolMessage>,
}

pub(crate) struct PermissionDecision {
    pub tool_call_id: String,
    pub tool_name: String,
    pub allowed: bool,
}

pub(crate) struct ToolMessage {
    pub call: ToolCall,
    pub content: String,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
}

pub(crate) struct DefaultToolExecutor<'a> {
    deps: ToolExecutorDeps<'a>,
}

impl<'a> DefaultToolExecutor<'a> {
    pub(crate) fn new(deps: ToolExecutorDeps<'a>) -> Self {
        Self { deps }
    }

    pub(crate) async fn execute(
        &self,
        tool_calls: &[ToolCall],
        options: &PromptOptions,
        cancel: Option<&CancellationToken>,
        session_id: &str,
        workdir: &Path,
        dedup: &mut ToolDedup,
    ) -> Result<ToolBatchResult> {
        let ctx = ToolContext {
            sandbox: Arc::clone(&self.deps.sandbox),
            cancel: cancel.cloned(),
            subagent: self.deps.subagent.clone(),
            permission: Some(Arc::clone(&self.deps.permission)),
            plan_mode: Some(Arc::clone(&self.deps.plan_mode)),
            todos: Some(Arc::clone(&self.deps.todos)),
            ask_user: Some(Arc::clone(&self.deps.ask_user)),
            background: Some(Arc::clone(&self.deps.background)),
        };

        struct PreparedTool {
            call: ToolCall,
            immediate: Option<(zene_tools::ToolResult, Option<u64>)>,
            schedule: Option<(ToolAccesses, String, String)>,
            terminal: bool,
        }

        let mut prepared = Vec::with_capacity(tool_calls.len());
        let mut mode_changes = Vec::new();
        let mut permission_decisions = Vec::new();

        for call in tool_calls {
            if zene_turn::is_cancelled(cancel) {
                return Err(zene_turn::aborted_error());
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
                let mut state = self.deps.plan_mode.lock();
                let result = handle_enter_plan_mode(&mut state, &call.arguments);
                drop(state);
                if !result.is_error {
                    mode_changes.push("plan".to_string());
                    emit_event(
                        &options.event_handler,
                        AgentEvent::ModeChanged {
                            mode_id: "plan".to_string(),
                        },
                    );
                }
                Some((result, None))
            } else if call.name == "ExitPlanMode" {
                let mut state = self.deps.plan_mode.lock();
                let result = handle_exit_plan_mode(
                    &mut state,
                    &call.arguments,
                    workdir,
                    session_id,
                    self.deps.plan_approval,
                )
                .unwrap_or_else(|err| zene_tools::ToolResult {
                    content: err.to_string(),
                    is_error: true,
                });
                drop(state);
                if !result.is_error {
                    mode_changes.push("default".to_string());
                    emit_event(
                        &options.event_handler,
                        AgentEvent::ModeChanged {
                            mode_id: "default".to_string(),
                        },
                    );
                }
                Some((result, None))
            } else if let Some(block) = self
                .deps
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
            } else if !self.deps.tools.contains(&call.name) {
                Some((
                    zene_tools::ToolResult {
                        content: format!("unknown tool: {}", call.name),
                        is_error: true,
                    },
                    None,
                ))
            } else if {
                let state = self.deps.plan_mode.lock();
                state.is_active() && !state.is_tool_allowed(&call.name)
            } {
                Some((
                    zene_tools::ToolResult {
                        content: PlanModeState::blocked_message(&call.name),
                        is_error: true,
                    },
                    None,
                ))
            } else {
                let allowed = match zene_permission::resolve_permission(
                    &self.deps.permission,
                    self.deps.approval_broker.as_ref(),
                    zene_permission::ApprovalRequest {
                        request_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        tool_call_id: Some(call.id.clone()),
                    },
                )
                .await
                {
                    Ok(v) => v,
                    Err(err) => {
                        if !options.quiet {
                            eprintln!("permission prompt error: {err}");
                        }
                        false
                    }
                };
                permission_decisions.push(PermissionDecision {
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    allowed,
                });
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

            let schedule = immediate.is_none().then(|| {
                (
                    classify_tool_accesses(&call.name, &call.arguments),
                    call.name.clone(),
                    call.arguments.clone(),
                )
            });
            let terminal = immediate.is_none() && self.deps.tools.terminates_batch(&call.name);

            prepared.push(PreparedTool {
                call: call.clone(),
                immediate,
                schedule,
                terminal,
            });
        }

        let mut scheduled = Vec::new();
        for item in &prepared {
            if let Some((accesses, name, arguments)) = item.schedule.as_ref() {
                let ctx = clone_tool_context(&ctx);
                let tools = Arc::clone(&self.deps.tools);
                let name = name.clone();
                let arguments = arguments.clone();
                let future: std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = (zene_tools::ToolResult, Option<u64>)>
                            + Send,
                    >,
                > = Box::pin(async move {
                    let started = Instant::now();
                    let result =
                        tools
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
        let mut terminal_results = Vec::with_capacity(prepared.len());
        let mut messages = Vec::with_capacity(prepared.len());

        for item in prepared {
            let (result, duration_ms) = if let Some(immediate) = item.immediate {
                immediate
            } else {
                scheduled_iter
                    .next()
                    .expect("missing scheduled tool result")
            };

            terminal_results.push((item.terminal, !result.is_error));
            let call = item.call;

            if !result.is_error {
                self.deps
                    .hooks
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
                bound_tool_output(workdir, &call.name, result.content)
            };

            if let Some(reminder) = dedup.on_call(&call.name, &call.arguments) {
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

            messages.push(ToolMessage {
                call,
                content,
                is_error: result.is_error,
                duration_ms,
            });
        }

        Ok(ToolBatchResult {
            outcome: outcome_for_batch(&terminal_results),
            mode_changes,
            permission_decisions,
            messages,
        })
    }
}

/// Run one Subagent tool batch through the same executor as the main Agent.
///
/// Capability switches come from [`zene_tools::ToolPolicy`]. Subagent scopes
/// pass [`zene_tools::ToolPolicy::subagent`]: plan mode stays inactive, hooks
/// stay empty, and ask-user uses the default prompter while the catalog omits
/// those tools. Missing permission inherits bypass so Explore/Coder keep prior
/// auto-allow unless a gate is passed from the parent.
pub(crate) async fn execute_subagent_tool_batch(
    tools: Arc<ToolRegistry>,
    sandbox: Arc<dyn Sandbox>,
    tool_calls: &[ToolCall],
    cancel: Option<&CancellationToken>,
    subagent_env: SubagentEnv,
    permission: Option<SharedToolPermission>,
    broker: Option<SharedApprovalBroker>,
    tool_policy: zene_tools::ToolPolicy,
) -> Result<ToolBatchResult> {
    debug_assert!(
        !tool_policy.plan_mode && !tool_policy.ask_user && !tool_policy.hooks,
        "subagent tool batch expects ToolPolicy::subagent()"
    );
    let permission = permission.unwrap_or_else(|| {
        Arc::new(Mutex::new(PermissionGate::new(
            PermissionMode::BypassPermissions,
        )))
    });
    let plan_mode = shared_plan_mode();
    let plan_approval: PlanApprovalPrompter = Arc::new(|_path, _body| Ok(false));
    let todos = shared_todo_store();
    let ask_user = default_ask_user_prompter();
    let background = shared_background_tasks();
    let hooks = HookRunner::with_bash(Vec::new(), sandbox.workdir().to_path_buf());
    let _ = tool_policy;
    let options = PromptOptions {
        stream: false,
        quiet: true,
        ..PromptOptions::default()
    };
    let mut dedup = ToolDedup::new();
    let executor = DefaultToolExecutor::new(ToolExecutorDeps {
        tools,
        sandbox: Arc::clone(&sandbox),
        permission,
        approval_broker: broker,
        plan_mode,
        plan_approval: &plan_approval,
        todos,
        ask_user,
        background,
        subagent: Some(subagent_env),
        hooks: &hooks,
    });
    executor
        .execute(
            tool_calls,
            &options,
            cancel,
            "subagent",
            sandbox.workdir(),
            &mut dedup,
        )
        .await
}

fn outcome_for_batch(results: &[(bool, bool)]) -> ToolBatchOutcome {
    if !results.is_empty()
        && results
            .iter()
            .all(|(terminal, success)| *terminal && *success)
    {
        ToolBatchOutcome::Terminate
    } else {
        ToolBatchOutcome::Continue
    }
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}...", input.chars().take(max).collect::<String>())
    }
}

fn bound_tool_output(workdir: &Path, tool_name: &str, content: String) -> String {
    let plan = plan_tool_output_bound(content, tool_name);
    let store = FsToolOutputStore::new(workdir);
    apply_tool_bound_plan(plan, &store)
}

fn clone_tool_context(ctx: &ToolContext) -> ToolContext {
    ToolContext {
        sandbox: Arc::clone(&ctx.sandbox),
        cancel: ctx.cancel.clone(),
        subagent: ctx.subagent.clone(),
        permission: ctx.permission.clone(),
        plan_mode: ctx.plan_mode.clone(),
        todos: ctx.todos.clone(),
        ask_user: ctx.ask_user.clone(),
        background: ctx.background.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::outcome_for_batch;
    use zene_turn::ToolBatchOutcome;

    #[test]
    fn only_all_successful_terminal_tools_terminate() {
        assert_eq!(
            outcome_for_batch(&[(true, true)]),
            ToolBatchOutcome::Terminate
        );
        assert_eq!(
            outcome_for_batch(&[(true, false)]),
            ToolBatchOutcome::Continue
        );
        assert_eq!(
            outcome_for_batch(&[(true, true), (false, true)]),
            ToolBatchOutcome::Continue
        );
        assert_eq!(outcome_for_batch(&[]), ToolBatchOutcome::Continue);
    }
}
