use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use zene_config::ZeneConfig;
use zene_llm::{ChatClient, ChatRequest, ChatResponse, Message, TokenUsage, ToolCall};
use zene_sandbox::Sandbox;
use zene_tools::{
    tools_for_profile, SubagentEnv, SubagentProfile, SubagentRunner, ToolContext,
    ToolRegistry, DEFAULT_SUBAGENT_MAX_DEPTH,
};

use zene_context::{
    compact_message_list_with_chat, estimate_context, should_compact,
    subagent_compaction_config, TokenEstimator,
};
use crate::context_config;
use zene_permission::{PermissionGate, SharedToolPermission};
use zene_turn::{
    aborted_error, max_turns_notice, run_turn_loop, StepResult, ToolBatchOutcome, TurnRuntime,
    TurnState,
};

#[async_trait]
pub trait ChatBackend: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
}

#[async_trait]
impl ChatBackend for ChatClient {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        ChatClient::chat(self, request).await
    }
}

pub struct CoreSubagentRunner {
    config: ZeneConfig,
}

impl CoreSubagentRunner {
    pub fn new(config: ZeneConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SubagentRunner for CoreSubagentRunner {
    async fn run_subagent(
        &self,
        prompt: &str,
        profile: SubagentProfile,
        cwd: Option<&Path>,
        parent_ctx: &ToolContext,
    ) -> Result<String> {
        let client = ChatClient::from_config(&self.config).await?;
        let sandbox = resolve_subagent_sandbox(&parent_ctx.sandbox, cwd)?;
        let parent_depth = parent_ctx
            .subagent
            .as_ref()
            .map(|env| env.depth)
            .unwrap_or(0);

        run_subagent(
            prompt,
            profile,
            sandbox,
            &self.config,
            &client,
            parent_ctx.cancel.as_ref(),
            parent_depth,
            parent_ctx.permission.clone(),
        )
        .await
    }
}

pub async fn run_subagent(
    prompt: &str,
    profile: SubagentProfile,
    sandbox: Arc<dyn Sandbox>,
    config: &ZeneConfig,
    backend: &dyn ChatBackend,
    cancel: Option<&CancellationToken>,
    parent_depth: u32,
    permission: Option<SharedToolPermission>,
) -> Result<String> {
    run_subagent_with_runner(
        prompt,
        profile,
        sandbox,
        config,
        backend,
        cancel,
        parent_depth,
        permission,
        None,
    )
    .await
}

pub(crate) async fn run_subagent_with_runner(
    prompt: &str,
    profile: SubagentProfile,
    sandbox: Arc<dyn Sandbox>,
    config: &ZeneConfig,
    backend: &dyn ChatBackend,
    cancel: Option<&CancellationToken>,
    parent_depth: u32,
    permission: Option<SharedToolPermission>,
    runner: Option<Arc<dyn SubagentRunner>>,
) -> Result<String> {
    let subagent_depth = parent_depth + 1;
    if subagent_depth > DEFAULT_SUBAGENT_MAX_DEPTH {
        anyhow::bail!(
            "Subagent nesting limit reached (max depth {DEFAULT_SUBAGENT_MAX_DEPTH})"
        );
    }

    let runner = runner.unwrap_or_else(|| Arc::new(CoreSubagentRunner::new(config.clone())));
    let subagent_env = SubagentEnv {
        depth: subagent_depth,
        max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
        runner,
    };
    let mut runtime = SubagentTurnRuntime::new(
        profile,
        sandbox,
        config,
        backend,
        subagent_env,
        permission,
    );

    run_turn_loop(&mut runtime, prompt, &(), cancel).await
}

/// TurnRuntime adapter for subagents.
///
/// Subagents intentionally remain ephemeral: they share the generic turn
/// state machine, but keep their conversation in memory and do not publish
/// the parent runtime's events or checkpoints.
struct SubagentTurnRuntime<'a> {
    sandbox: Arc<dyn Sandbox>,
    config: &'a ZeneConfig,
    backend: &'a dyn ChatBackend,
    subagent_env: SubagentEnv,
    permission: Option<SharedToolPermission>,
    tools: ToolRegistry,
    messages: Vec<Message>,
    compaction_config: zene_context::CompactionConfig,
    active_turn: Option<TurnState>,
}

impl<'a> SubagentTurnRuntime<'a> {
    fn new(
        profile: SubagentProfile,
        sandbox: Arc<dyn Sandbox>,
        config: &'a ZeneConfig,
        backend: &'a dyn ChatBackend,
        subagent_env: SubagentEnv,
        permission: Option<SharedToolPermission>,
    ) -> Self {
        let system_prompt = subagent_system_prompt(profile, sandbox.workdir());
        Self {
            sandbox,
            config,
            backend,
            subagent_env,
            permission,
            tools: tools_for_profile(profile),
            messages: vec![Message::system(&system_prompt)],
            compaction_config: subagent_compaction_config(
                &context_config::context_compaction_config(&config.compaction),
            ),
            active_turn: None,
        }
    }
}

#[async_trait]
impl TurnRuntime for SubagentTurnRuntime<'_> {
    type Options = ();

    fn max_steps(&self) -> u32 {
        self.config.max_turns
    }

    fn active_turn(&mut self) -> Option<&mut TurnState> {
        self.active_turn.as_mut()
    }

    fn on_step_begin(
        &self,
        _turn_id: zene_turn::TurnId,
        _step_id: zene_turn::StepId,
        _step: u32,
        _options: &Self::Options,
    ) {
    }

    async fn prepare_turn(&mut self, user_input: &str) -> Result<()> {
        self.active_turn = Some(TurnState::begin());
        self.messages.push(Message::user(user_input));
        Ok(())
    }

    async fn run_step(
        &mut self,
        _options: &Self::Options,
        cancel: Option<&CancellationToken>,
    ) -> Result<StepResult> {
        if check_cancelled(cancel)? {
            return Err(aborted_error());
        }
        maybe_compact_subagent_messages(
            &mut self.messages,
            &self.tools,
            &self.compaction_config,
            &self.config.model,
            self.backend,
        )
        .await?;
        let response = self
            .backend
            .chat(ChatRequest {
                model: self.config.model.clone(),
                messages: self.messages.clone(),
                tools: self.tools.definitions(),
                stream: false,
                context: None,
            })
            .await
            .context("subagent llm step")?;
        let had_tool_calls = response
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());
        Ok(StepResult {
            message: response.message,
            usage: response.usage,
            had_tool_calls,
        })
    }

    async fn on_step_usage(
        &mut self,
        _usage: &TokenUsage,
        _options: &Self::Options,
    ) -> Result<()> {
        Ok(())
    }

    async fn run_tools(
        &mut self,
        tool_calls: &[ToolCall],
        _options: &Self::Options,
        cancel: Option<&CancellationToken>,
    ) -> Result<ToolBatchOutcome> {
        run_subagent_tools(
            &self.tools,
            tool_calls,
            &self.sandbox,
            cancel,
            &self.subagent_env,
            self.permission.clone(),
            &mut self.messages,
        )
        .await?;
        Ok(ToolBatchOutcome::Continue)
    }

    fn inject_steer(&mut self, _options: &Self::Options) -> Result<bool> {
        Ok(false)
    }

    fn push_assistant(&mut self, message: Message) {
        self.messages.push(message);
    }

    fn on_incomplete_turn(
        &mut self,
        max_steps: u32,
        final_text: &mut String,
        _options: &Self::Options,
    ) -> Result<()> {
        let notice = max_turns_notice(max_steps);
        *final_text = if final_text.trim().is_empty() {
            notice
        } else {
            format!("{final_text}\n\n{notice}")
        };
        self.messages.push(Message::assistant(final_text.clone()));
        Ok(())
    }

    async fn finish_turn(&mut self) -> Result<()> {
        Ok(())
    }
}

async fn run_subagent_tools(
    tools: &ToolRegistry,
    tool_calls: &[ToolCall],
    sandbox: &Arc<dyn Sandbox>,
    cancel: Option<&CancellationToken>,
    subagent_env: &SubagentEnv,
    permission: Option<SharedToolPermission>,
    messages: &mut Vec<Message>,
) -> Result<()> {
    let ctx = ToolContext {
        sandbox: Arc::clone(sandbox),
        cancel: cancel.cloned(),
        subagent: Some(subagent_env.clone()),
        permission: permission.clone(),
        plan_mode: None,
        todos: None,
        ask_user: None,
        background: None,
    };

    for call in tool_calls {
        if check_cancelled(cancel)? {
            return Err(aborted_error());
        }

        let allowed = if let Some(ref gate) = permission {
            gate.lock().approve_tool_call(&call.name, &call.arguments)?
        } else {
            true
        };

        let result = if !allowed {
            zene_tools::ToolResult {
                content: PermissionGate::denied_message(&call.name),
                is_error: true,
            }
        } else {
            tools
                .execute(&call.name, &call.arguments, &ctx)
                .await
                .unwrap_or_else(|err| zene_tools::ToolResult {
                    content: err.to_string(),
                    is_error: true,
                })
        };

        let content = if result.content.is_empty() {
            if result.is_error {
                "(tool returned empty error output)".to_string()
            } else {
                "(tool returned no output)".to_string()
            }
        } else {
            result.content
        };

        messages.push(Message::tool_result_with_error(
            &call.id,
            &call.name,
            content,
            result.is_error,
        ));
    }

    Ok(())
}

fn subagent_system_prompt(profile: SubagentProfile, workdir: &Path) -> String {
    let workdir = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());

    match profile {
        SubagentProfile::Explore => format!(
            "You are an explore subagent for Zene. Investigate the codebase using read-only tools (Read, Grep, Glob). \
             Report findings concisely. Working directory: `{}`.",
            workdir.display()
        ),
        SubagentProfile::Coder => format!(
            "You are a coder subagent for Zene. Read and modify code as requested using the available tools. \
             Working directory: `{}`.",
            workdir.display()
        ),
    }
}

fn resolve_subagent_sandbox(
    parent: &Arc<dyn Sandbox>,
    cwd: Option<&Path>,
) -> Result<Arc<dyn Sandbox>> {
    match cwd {
        None => Ok(Arc::clone(parent)),
        Some(path) => {
            let resolved = parent.resolve(path.to_str().unwrap_or(""))?;
            if !resolved.is_dir() {
                anyhow::bail!("Task cwd is not a directory: {}", resolved.display());
            }
            parent.scoped_to(resolved)
        }
    }
}

fn check_cancelled(cancel: Option<&CancellationToken>) -> Result<bool> {
    Ok(cancel.is_some_and(CancellationToken::is_cancelled))
}

async fn maybe_compact_subagent_messages(
    messages: &mut Vec<Message>,
    tools: &ToolRegistry,
    compaction_config: &zene_context::CompactionConfig,
    model: &str,
    backend: &dyn ChatBackend,
) -> Result<()> {
    let tool_defs = tools.definitions();
    let estimator = TokenEstimator::default();
    let estimated = estimate_context(messages, &tool_defs, &estimator) as u32;
    if !should_compact(estimated, compaction_config) {
        return Ok(());
    }

    if compact_message_list_with_chat(
        messages,
        model,
        compaction_config,
        "subagent_token_threshold",
        &tool_defs,
        &estimator,
        |request| backend.chat(request),
    )
    .await?
    .is_some()
    {
        ensure_subagent_system_message(messages);
    }

    Ok(())
}

fn ensure_subagent_system_message(messages: &mut Vec<Message>) {
    if messages.first().is_some_and(|m| m.role == zene_llm::Role::System) {
        return;
    }
    if let Some(system) = messages.iter().find(|m| m.role == zene_llm::Role::System) {
        let system = system.clone();
        messages.retain(|m| m.role != zene_llm::Role::System);
        messages.insert(0, system);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use parking_lot::Mutex;

    use zene_permission::{PermissionGate, PermissionMode, PromptChoice, SharedToolPermission};
    use tempfile::tempdir;
    use zene_llm::ToolCall;
    use zene_sandbox::LocalSandbox;
    use zene_tools::default_builtin_tools;

    fn test_permission_deny() -> SharedToolPermission {
        Arc::new(Mutex::new(PermissionGate::with_prompter(
            PermissionMode::Manual,
            Box::new(|_tool, _args| Ok(PromptChoice::Deny)),
        )))
    }

    struct ScriptedBackend {
        responses: Vec<ChatResponse>,
        calls: AtomicUsize,
        on_first_call: Option<Box<dyn Fn(&ChatRequest) + Send + Sync>>,
    }

    impl ScriptedBackend {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses,
                calls: AtomicUsize::new(0),
                on_first_call: None,
            }
        }

        fn with_first_call_check(
            responses: Vec<ChatResponse>,
            check: impl Fn(&ChatRequest) + Send + Sync + 'static,
        ) -> Self {
            Self {
                responses,
                calls: AtomicUsize::new(0),
                on_first_call: Some(Box::new(check)),
            }
        }
    }

    #[async_trait]
    impl ChatBackend for ScriptedBackend {
        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self
                .responses
                .get(idx)
                .cloned()
                .with_context(|| format!("no scripted response for call {idx}"))?;

            if idx == 0 {
                if let Some(check) = &self.on_first_call {
                    check(&request);
                }
            }

            Ok(response)
        }
    }

    struct RecordingRunner {
        config: ZeneConfig,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SubagentRunner for RecordingRunner {
        async fn run_subagent(
            &self,
            prompt: &str,
            profile: SubagentProfile,
            cwd: Option<&Path>,
            parent_ctx: &ToolContext,
        ) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let client = ChatClient::from_config(&self.config).await?;
            let sandbox = resolve_subagent_sandbox(&parent_ctx.sandbox, cwd)?;
            let parent_depth = parent_ctx
                .subagent
                .as_ref()
                .map(|env| env.depth)
                .unwrap_or(0);
            run_subagent(
                prompt,
                profile,
                sandbox,
                &self.config,
                &client,
                parent_ctx.cancel.as_ref(),
                parent_depth,
                parent_ctx.permission.clone(),
            )
            .await
        }
    }

    #[tokio::test]
    async fn explore_subagent_lists_files_via_glob_without_write() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("alpha.txt"), "a")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("beta.txt"), "b")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("notes.md"), "n")
            .await
            .unwrap();

        let sandbox = zene_sandbox::into_arc(LocalSandbox::new(dir.path()));
        let config = ZeneConfig::default();

        let backend = ScriptedBackend::with_first_call_check(
            vec![
            ChatResponse {
                message: Message::assistant_with_tools(
                    None,
                    vec![ToolCall {
                        id: "call_glob".to_string(),
                        name: "Glob".to_string(),
                        arguments: r#"{"pattern":"**/*.txt"}"#.to_string(),
                    }],
                ),
                usage: None,
            },
            ChatResponse {
                message: Message::assistant("Found alpha.txt and beta.txt"),
                usage: None,
            },
        ],
            |request| {
                let tool_names: Vec<_> = request
                    .tools
                    .iter()
                    .map(|tool| tool.name.as_str())
                    .collect();
                assert!(tool_names.contains(&"Glob"));
                assert!(!tool_names.contains(&"Write"));
                assert!(!tool_names.contains(&"Task"));
            },
        );

        let result = run_subagent(
            "List all .txt files in the workspace",
            SubagentProfile::Explore,
            Arc::clone(&sandbox),
            &config,
            &backend,
            None,
            0,
            None,
        )
        .await
        .expect("subagent should complete");

        let explore_tools: Vec<_> = tools_for_profile(SubagentProfile::Explore)
            .definitions()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(explore_tools.contains(&"Glob".to_string()));
        assert!(!explore_tools.contains(&"Write".to_string()));
        assert!(!explore_tools.contains(&"Task".to_string()));

        assert!(result.contains("alpha.txt"));
        assert!(result.contains("beta.txt"));
        assert!(!result.contains("notes.md"));

        let write_attempt = tools_for_profile(SubagentProfile::Explore)
            .execute(
                "Write",
                r#"{"path":"blocked.txt","content":"nope"}"#,
                &ToolContext::without_subagent(Arc::clone(&sandbox)),
            )
            .await;
        assert!(write_attempt.is_err());
        assert!(
            write_attempt
                .unwrap_err()
                .to_string()
                .contains("unknown tool")
        );
    }

    #[tokio::test]
    async fn subagent_uses_shared_turn_engine_step_budget() {
        let dir = tempdir().unwrap();
        let sandbox = zene_sandbox::into_arc(LocalSandbox::new(dir.path()));
        let mut config = ZeneConfig::default();
        config.max_turns = 1;
        let backend = ScriptedBackend::new(vec![ChatResponse {
            message: Message::assistant_with_tools(
                None,
                vec![ToolCall {
                    id: "call_glob".into(),
                    name: "Glob".into(),
                    arguments: r#"{"pattern":"**/*.rs"}"#.into(),
                }],
            ),
            usage: None,
        }]);

        let result = run_subagent(
            "Inspect Rust files",
            SubagentProfile::Explore,
            sandbox,
            &config,
            &backend,
            None,
            0,
            None,
        )
        .await
        .expect("incomplete subagent should return a notice");
        assert!(result.contains("Reached max_turns (1)"));
    }

    #[tokio::test]
    async fn task_tool_rejects_nested_subagent_at_max_depth() {
        let dir = tempdir().unwrap();
        let sandbox = zene_sandbox::into_arc(LocalSandbox::new(dir.path()));
        let config = ZeneConfig::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = Arc::new(RecordingRunner {
            config,
            calls: Arc::clone(&calls),
        });

        let env = SubagentEnv {
            depth: DEFAULT_SUBAGENT_MAX_DEPTH,
            max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
            runner,
        };
        let ctx = ToolContext {
            sandbox: Arc::clone(&sandbox),
            cancel: None,
            subagent: Some(env),
            permission: None,
            plan_mode: None,
            todos: None,
            ask_user: None,
            background: None,
        };

        let result = default_builtin_tools()
            .execute(
                "Task",
                r#"{"prompt":"nested","agent":"explore"}"#,
                &ctx,
            )
            .await
            .expect("task execution");

        assert!(result.is_error);
        assert!(result.content.contains("nesting limit"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn coder_subagent_manual_mode_rejects_write_without_approval() {
        let dir = tempdir().unwrap();
        let sandbox = zene_sandbox::into_arc(LocalSandbox::new(dir.path()));
        let config = ZeneConfig::default();
        let permission = test_permission_deny();

        let backend = ScriptedBackend::new(vec![
            ChatResponse {
                message: Message::assistant_with_tools(
                    None,
                    vec![ToolCall {
                        id: "call_write".to_string(),
                        name: "Write".to_string(),
                        arguments: r#"{"path":"secret.txt","content":"nope"}"#.to_string(),
                    }],
                ),
                usage: None,
            },
            ChatResponse {
                message: Message::assistant("Write was denied"),
                usage: None,
            },
        ]);

        let result = run_subagent(
            "Write secret.txt",
            SubagentProfile::Coder,
            Arc::clone(&sandbox),
            &config,
            &backend,
            None,
            0,
            Some(permission),
        )
        .await
        .expect("subagent should complete after denial");

        assert!(result.contains("denied") || result.contains("Denied"));
        assert!(!dir.path().join("secret.txt").exists());
    }
}
