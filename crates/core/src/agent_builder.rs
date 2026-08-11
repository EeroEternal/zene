//! Optional wiring for [`Agent`](crate::Agent). Default path matches legacy `Agent::new`;
//! inject sandbox/tools/context/MCP when assembling a custom runtime.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;
use tracing::{info, warn};
use zene_config::ZeneConfig;
use zene_context::{ensure_memory_in_system, ContextEngine};
use zene_llm::ChatClient;
use zene_sandbox::{LocalSandbox, Sandbox};
use zene_session::{AgentRecordWriter, SessionRecord};
use zene_tools::{
    default_ask_user_prompter, shared_background_tasks, shared_plan_mode,
    shared_todo_store_from, SharedAskUserPrompter, SharedBackgroundTasks, SharedPlanMode,
    SharedTodoStore, ToolRegistry,
};
use zene_mcp::McpManager;

use crate::hooks::HookRunner;
use zene_permission::{PermissionGate, PermissionMode, PermissionRule, RuleAction, SharedToolPermission};
use crate::plan_mode::{default_plan_approval_prompter, PlanApprovalPrompter};
use crate::tool_dedup::ToolDedup;
use crate::turn::SteerBuffer;
use crate::workspace;
use crate::Agent;

/// How MCP servers are attached when building an [`Agent`].
#[derive(Default)]
enum McpAttach {
    /// Connect from workspace MCP config (same as legacy `Agent::new`).
    #[default]
    Auto,
    /// Do not connect or register MCP tools.
    Skip,
    /// Use a pre-connected manager; `ToolRegistry` must already include its tools.
    Inject(McpManager),
}

/// Fluent builder for [`Agent`]. All fields except the four constructor args use
/// product defaults until overridden.
pub struct AgentBuilder {
    config: ZeneConfig,
    sandbox: LocalSandbox,
    session: SessionRecord,
    permission_mode: PermissionMode,

    client: Option<ChatClient>,
    tools: Option<ToolRegistry>,
    mcp: McpAttach,
    context: Option<ContextEngine>,
    hooks: Option<HookRunner>,
    load_hooks_from_config: bool,
    permission: Option<SharedToolPermission>,
    plan_mode: Option<SharedPlanMode>,
    plan_approval: Option<PlanApprovalPrompter>,
    todos: Option<SharedTodoStore>,
    ask_user: Option<SharedAskUserPrompter>,
    background: Option<SharedBackgroundTasks>,
    external_session_id: Option<String>,
    include_workspace_context: Option<bool>,
}

impl AgentBuilder {
    pub fn new(
        config: ZeneConfig,
        sandbox: LocalSandbox,
        session: SessionRecord,
        permission_mode: PermissionMode,
    ) -> Self {
        Self {
            config,
            sandbox,
            session,
            permission_mode,
            client: None,
            tools: None,
            mcp: McpAttach::default(),
            context: None,
            hooks: None,
            load_hooks_from_config: true,
            permission: None,
            plan_mode: None,
            plan_approval: None,
            todos: None,
            ask_user: None,
            background: None,
            external_session_id: None,
            include_workspace_context: None,
        }
    }

    /// Use a pre-built chat client instead of `ChatClient::from_config`.
    pub fn client(mut self, client: ChatClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Replace default `agent_tools` registry (MCP tools are not merged unless [`Self::mcp_auto`]).
    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Skip MCP discovery and registration.
    pub fn without_mcp(mut self) -> Self {
        self.mcp = McpAttach::Skip;
        self
    }

    /// Attach a pre-connected MCP manager (caller must have merged MCP tools into the registry if needed).
    pub fn mcp(mut self, manager: McpManager) -> Self {
        self.mcp = McpAttach::Inject(manager);
        self
    }

    /// Re-enable auto MCP connect after [`Self::without_mcp`] or [`Self::mcp`].
    pub fn mcp_auto(mut self) -> Self {
        self.mcp = McpAttach::Auto;
        self
    }

    /// Inject a custom [`ContextEngine`] (window size taken from config unless you set it on the engine).
    pub fn context_engine(mut self, context: ContextEngine) -> Self {
        self.context = Some(context);
        self
    }

    /// Override inference / gateway session id (also reads `ZENE_SESSION_ID` when unset).
    pub fn external_session_id(mut self, id: impl Into<String>) -> Self {
        self.external_session_id = Some(id.into());
        self
    }

    /// Pre-built hook runner; disables config hook loading unless [`Self::load_hooks_from_config`] is set.
    pub fn hooks(mut self, hooks: HookRunner) -> Self {
        self.hooks = Some(hooks);
        self.load_hooks_from_config = false;
        self
    }

    /// Load `.zene/hooks` from config (default when no custom hooks).
    pub fn load_hooks_from_config(mut self, load: bool) -> Self {
        self.load_hooks_from_config = load;
        self
    }

    pub fn permission(mut self, permission: SharedToolPermission) -> Self {
        self.permission = Some(permission);
        self
    }

    pub fn plan_mode(mut self, plan_mode: SharedPlanMode) -> Self {
        self.plan_mode = Some(plan_mode);
        self
    }

    pub fn plan_approval(mut self, prompter: PlanApprovalPrompter) -> Self {
        self.plan_approval = Some(prompter);
        self
    }

    pub fn todos(mut self, todos: SharedTodoStore) -> Self {
        self.todos = Some(todos);
        self
    }

    pub fn ask_user(mut self, prompter: SharedAskUserPrompter) -> Self {
        self.ask_user = Some(prompter);
        self
    }

    pub fn background_tasks(mut self, background: SharedBackgroundTasks) -> Self {
        self.background = Some(background);
        self
    }

    /// Override workspace context inclusion for the system prompt.
    pub fn include_workspace_context(mut self, include: bool) -> Self {
        self.include_workspace_context = Some(include);
        self
    }

    pub async fn build(mut self) -> Result<Agent> {
        let local = self.sandbox;
        let workdir = local.workdir().to_path_buf();
        let include_workspace = self
            .include_workspace_context
            .unwrap_or(self.config.include_workspace_context);
        let system_prompt = workspace::build_system_prompt(
            &self.config.system_prompt,
            &workdir,
            include_workspace,
        );
        self.session.ensure_system_message(&system_prompt);
        ensure_memory_in_system(&mut self.session.messages, &workdir);

        let client = match self.client {
            Some(client) => client,
            None => ChatClient::from_config(&self.config).await?,
        };
        let record_writer = AgentRecordWriter::for_session(&self.session.meta.id)?;

        let mut tools = match self.tools {
            Some(tools) => tools,
            None => zene_tools::agent_tools(self.config.agent_profile, self.config.web_search.clone()),
        };

        let mcp = match self.mcp {
            McpAttach::Skip => None,
            McpAttach::Inject(manager) => Some(manager),
            McpAttach::Auto => {
                let (mcp, mcp_tools) =
                    McpManager::connect_with_sandbox(&workdir, &local).await?;
                if !mcp_tools.definitions().is_empty() {
                    info!(
                        tool_count = mcp_tools.definitions().len(),
                        "registered MCP tools"
                    );
                    tools.extend(mcp_tools);
                }
                if mcp.is_empty() {
                    None
                } else {
                    Some(mcp)
                }
            }
        };

        let sandbox: Arc<dyn Sandbox> = zene_sandbox::into_arc(local);

        let hooks = match self.hooks {
            Some(hooks) => hooks,
            None if self.load_hooks_from_config => {
                let hook_entries = self.config.load_hooks().unwrap_or_else(|err| {
                    warn!(error = %err, "failed to load hooks; continuing without hooks");
                    Vec::new()
                });
                HookRunner::new(hook_entries, workdir.clone())
            }
            None => HookRunner::new(Vec::new(), workdir.clone()),
        };

        let todos = match self.todos {
            Some(todos) => todos,
            None => shared_todo_store_from(self.session.todos.clone()),
        };

        let mut context = match self.context {
            Some(context) => context,
            None => ContextEngine::new(self.config.compaction.context_window_tokens),
        };

        if let Some(id) = self
            .external_session_id
            .or_else(zene_context::external_session_id_from_env)
        {
            context.set_external_session_id(Some(id));
        }

        let auto_allow_bash = self.config.sandbox.auto_allow_bash && sandbox.is_enforced();
        let permission = match self.permission {
            Some(permission) => permission,
            None => shared_permission_with_rules(
                self.permission_mode,
                permission_rules_from_config(&self.config),
                auto_allow_bash,
            ),
        };

        Ok(Agent {
            config: self.config,
            client,
            tools: Arc::new(tools),
            sandbox,
            session: self.session,
            turn_usage: zene_llm::TokenUsage::default(),
            context,
            active_turn: None,
            steer_buffer: Arc::new(Mutex::new(SteerBuffer::default())),
            system_prompt,
            permission,
            plan_mode: self.plan_mode.unwrap_or_else(shared_plan_mode),
            plan_approval: self
                .plan_approval
                .unwrap_or_else(|| Arc::new(default_plan_approval_prompter)),
            todos,
            ask_user: self.ask_user.unwrap_or_else(default_ask_user_prompter),
            tool_dedup: ToolDedup::new(),
            hooks,
            record_writer,
            mcp,
            background: self.background.unwrap_or_else(shared_background_tasks),
        })
    }
}

pub(crate) fn permission_rules_from_config(config: &ZeneConfig) -> Vec<PermissionRule> {
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

pub(crate) fn shared_permission_with_rules(
    mode: PermissionMode,
    rules: Vec<PermissionRule>,
    auto_allow_bash: bool,
) -> SharedToolPermission {
    Arc::new(Mutex::new(
        PermissionGate::new(mode)
            .with_rules(rules)
            .with_auto_allow_bash(auto_allow_bash),
    ))
}
