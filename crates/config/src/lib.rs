use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "openai" | "openai-compatible" | "openai_compatible" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            other if other.is_empty() => Ok(Self::OpenAi),
            other => Err(format!(
                "unknown provider `{other}`; expected `openai` or `anthropic`"
            )),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to parse JSON at {path}: {source}")]
    ParseJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to write config at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: u32 = 128_000;

/// Main agent tool profile. Subset of built-in tools; MCP tools are always merged on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentProfile {
    /// All built-in tools (default).
    #[default]
    Full,
    /// Read-only exploration: Read/Grep/Glob + collaboration + plan tools.
    Explore,
    /// Read/write coding: Write/Edit/Bash + collaboration + Task subagent + plan tools.
    Coder,
}

impl AgentProfile {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "" | "full" | "default" => Ok(Self::Full),
            "explore" => Ok(Self::Explore),
            "coder" => Ok(Self::Coder),
            other => Err(format!(
                "unknown agent profile `{other}`; expected `full`, `explore`, or `coder`"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_compaction_trigger_ratio")]
    pub trigger_ratio: f32,
    #[serde(default = "default_keep_recent_ratio")]
    pub keep_recent_ratio: f32,
    #[serde(default = "default_context_window_tokens")]
    pub context_window_tokens: u32,
    #[serde(default = "default_min_keep_messages")]
    pub min_keep_messages: usize,
    /// Before full compact, aggressively truncate tool results in the current
    /// turn's steps (after the last user message) — grok Intra Steps-first lite.
    #[serde(default = "default_intra_steps_first")]
    pub intra_steps_first: bool,
}

fn default_compaction_trigger_ratio() -> f32 {
    0.85
}

fn default_keep_recent_ratio() -> f32 {
    0.25
}

fn default_context_window_tokens() -> u32 {
    DEFAULT_CONTEXT_WINDOW_TOKENS
}

fn default_min_keep_messages() -> usize {
    20
}

fn default_intra_steps_first() -> bool {
    true
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            trigger_ratio: default_compaction_trigger_ratio(),
            keep_recent_ratio: default_keep_recent_ratio(),
            context_window_tokens: default_context_window_tokens(),
            min_keep_messages: default_min_keep_messages(),
            intra_steps_first: default_intra_steps_first(),
        }
    }
}

fn merge_config_toml(global_path: &Path, project_path: &Path) -> Result<toml::Value, ConfigError> {
    let global_raw = fs::read_to_string(global_path).map_err(|source| ConfigError::Read {
        path: global_path.to_path_buf(),
        source,
    })?;
    let mut merged: toml::Value = toml::from_str(&global_raw).map_err(|source| ConfigError::Parse {
        path: global_path.to_path_buf(),
        source,
    })?;

    if project_path.exists() {
        let project_raw = fs::read_to_string(project_path).map_err(|source| ConfigError::Read {
            path: project_path.to_path_buf(),
            source,
        })?;
        let project: toml::Value = toml::from_str(&project_raw).map_err(|source| ConfigError::Parse {
            path: project_path.to_path_buf(),
            source,
        })?;
        merge_toml_values(&mut merged, project);
    }

    Ok(merged)
}

fn merge_toml_values(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_value) in overlay_table {
                match base_table.get_mut(&key) {
                    Some(base_value) if base_value.is_table() && overlay_value.is_table() => {
                        merge_toml_values(base_value, overlay_value);
                    }
                    _ => {
                        base_table.insert(key, overlay_value);
                    }
                }
            }
        }
        (base_slot, overlay) => *base_slot = overlay,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookEntry {
    pub event: String,
    pub command: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HooksFile {
    #[serde(default)]
    pub hooks: Vec<HookEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSearchProviderKind {
    Tavily,
    DuckDuckGo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchConfig {
    #[serde(default = "default_web_search_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_web_search_provider() -> String {
    "duckduckgo".to_string()
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: default_web_search_provider(),
            api_key: None,
        }
    }
}

impl WebSearchConfig {
    pub fn resolved_api_key(&self) -> Option<String> {
        if let Some(key) = &self.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }
        if let Ok(key) = env::var("ZENE_WEB_SEARCH_API_KEY") {
            if !key.is_empty() {
                return Some(key);
            }
        }
        None
    }

    pub fn effective_provider(&self) -> WebSearchProviderKind {
        match self.provider.trim().to_lowercase().as_str() {
            "tavily" => WebSearchProviderKind::Tavily,
            "duckduckgo" | "ddg" => WebSearchProviderKind::DuckDuckGo,
            _ if self.resolved_api_key().is_some() => WebSearchProviderKind::Tavily,
            _ => WebSearchProviderKind::DuckDuckGo,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeneConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub anthropic_base_url: Option<String>,
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub model_context_windows: HashMap<String, u32>,
    #[serde(default = "default_chars_per_token")]
    pub chars_per_token: f32,
    #[serde(default)]
    pub model_chars_per_token: HashMap<String, f32>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default = "default_include_workspace_context")]
    pub include_workspace_context: bool,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    #[serde(default)]
    pub permission_rules: PermissionRulesConfig,
    #[serde(default)]
    pub hooks: Vec<HookEntry>,
    #[serde(default)]
    pub web_search: WebSearchConfig,
    #[serde(default)]
    pub agent_profile: AgentProfile,
    #[serde(default)]
    pub sandbox: SandboxSettings,
}

/// Keel sandbox profile selection and host-side network policy knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxSettings {
    /// `off` | `workspace` | `read-only` | `strict` | custom name from `sandbox.toml`.
    /// When omitted, defaults to `workspace` (or `read-only` for `agent_profile = "explore"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Extra egress allowlist entries (`host` or `host:port`). When non-empty,
    /// network becomes an allowlist (even on top of workspace).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_hosts: Vec<String>,
    /// When a sandbox profile is active (not `off`), auto-approve Bash prompts.
    #[serde(default)]
    pub auto_allow_bash: bool,
}

impl SandboxSettings {
    /// Resolve the effective profile name.
    pub fn effective_profile(&self, agent_profile: AgentProfile) -> String {
        if let Some(profile) = self.profile.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return normalize_sandbox_profile(profile);
        }
        match agent_profile {
            AgentProfile::Explore => "read-only".to_string(),
            AgentProfile::Full | AgentProfile::Coder => "workspace".to_string(),
        }
    }
}

fn normalize_sandbox_profile(profile: &str) -> String {
    match profile.trim().to_lowercase().as_str() {
        "readonly" | "read_only" => "read-only".to_string(),
        other => other.to_string(),
    }
}

fn default_provider() -> String {
    "openai".to_string()
}

fn default_permission_mode() -> String {
    "default".to_string()
}

/// Permission allow/deny/ask rule loaded from config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRuleConfig {
    /// Tool name pattern (`Bash`, `mcp__*`, `*`).
    pub pattern: String,
    /// `allow` | `deny` | `ask`
    pub action: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRulesConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
}

impl PermissionRulesConfig {
    pub fn to_flat_rules(&self) -> Vec<PermissionRuleConfig> {
        let mut out = Vec::new();
        for pattern in &self.deny {
            out.push(PermissionRuleConfig {
                pattern: pattern.clone(),
                action: "deny".into(),
            });
        }
        for pattern in &self.allow {
            out.push(PermissionRuleConfig {
                pattern: pattern.clone(),
                action: "allow".into(),
            });
        }
        for pattern in &self.ask {
            out.push(PermissionRuleConfig {
                pattern: pattern.clone(),
                action: "ask".into(),
            });
        }
        out
    }
}

fn default_include_workspace_context() -> bool {
    true
}

fn default_model() -> String {
    "deepseek-chat".to_string()
}

fn default_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

fn default_max_turns() -> u32 {
    50
}

fn default_chars_per_token() -> f32 {
    4.0
}

fn default_system_prompt() -> String {
    "You are Zene, a local coding agent. You help users read, edit, and navigate codebases using the provided tools. Prefer small, focused changes. Explain briefly what you did.".to_string()
}

impl Default for ZeneConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            api_key: None,
            base_url: default_base_url(),
            anthropic_base_url: None,
            anthropic_api_key: None,
            model_context_windows: HashMap::new(),
            chars_per_token: default_chars_per_token(),
            model_chars_per_token: HashMap::new(),
            max_turns: default_max_turns(),
            system_prompt: default_system_prompt(),
            compaction: CompactionConfig::default(),
            include_workspace_context: default_include_workspace_context(),
            permission_mode: default_permission_mode(),
            permission_rules: PermissionRulesConfig::default(),
            hooks: Vec::new(),
            web_search: WebSearchConfig::default(),
            agent_profile: AgentProfile::default(),
            sandbox: SandboxSettings::default(),
        }
    }
}

impl ZeneConfig {
    /// Load global `~/.zene/config.toml`, merge project `.zene/config.toml` when present
    /// (project wins on key collision), then apply environment overrides.
    pub fn load(workdir: &Path) -> Result<Self, ConfigError> {
        let global_path = config_path();
        if !global_path.exists() {
            Self::default().ensure_file()?;
        }

        let project_path = project_config_path(workdir);
        let merged = merge_config_toml(&global_path, &project_path)?;
        let mut config: Self = merged.try_into().map_err(|source| ConfigError::Parse {
            path: global_path.clone(),
            source,
        })?;

        config.apply_env_overrides();
        config.apply_model_context_window();
        Ok(config)
    }

    pub fn load_hooks(&self) -> Result<Vec<HookEntry>, ConfigError> {
        load_hooks(&self.hooks)
    }

    pub fn ensure_file(&self) -> Result<(), ConfigError> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: path.clone(),
                source,
            })?;
        }
        if !path.exists() {
            let raw = toml::to_string_pretty(self).expect("config serializes");
            fs::write(&path, raw).map_err(|source| ConfigError::Write {
                path: path.clone(),
                source,
            })?;
        }
        Ok(())
    }

    pub fn provider_kind(&self) -> ProviderKind {
        self.provider_kind_parse()
            .unwrap_or(ProviderKind::OpenAi)
    }

    pub fn provider_kind_parse(&self) -> Result<ProviderKind, String> {
        ProviderKind::parse(&self.provider)
    }

    pub fn context_window_for_model(&self) -> u32 {
        if let Some(tokens) = self.model_context_windows.get(&self.model) {
            return *tokens;
        }
        default_context_window_for_model(&self.model)
    }

    pub fn chars_per_token_for_model(&self) -> f32 {
        if let Some(ratio) = self.model_chars_per_token.get(&self.model) {
            return ratio.max(1.0);
        }
        self.chars_per_token.max(1.0)
    }

    fn apply_model_context_window(&mut self) {
        if self.compaction.context_window_tokens == DEFAULT_CONTEXT_WINDOW_TOKENS {
            let window = self.context_window_for_model();
            if window != DEFAULT_CONTEXT_WINDOW_TOKENS {
                self.compaction.context_window_tokens = window;
            }
        }
    }

    /// Recompute compaction window after model change (e.g. from agent `switch_model`).
    pub fn refresh_model_context_window(&mut self) {
        self.apply_model_context_window();
    }

    pub fn openai_base_url(&self) -> String {
        self.base_url.clone()
    }

    pub fn openai_api_key(&self) -> Result<String, anyhow::Error> {
        if let Some(key) = &self.api_key {
            if !key.is_empty() {
                return Ok(key.clone());
            }
        }
        for var in [
            "DEEPSEEK_API_KEY",
            "MOONSHOT_API_KEY",
            "ZHIPUAI_API_KEY",
            "ZHIPU_API_KEY",
            "ZENE_API_KEY",
            "OPENAI_API_KEY",
        ] {
            if let Ok(key) = env::var(var) {
                if !key.is_empty() {
                    return Ok(key);
                }
            }
        }
        Ok("".to_string())
    }

    pub fn anthropic_base_url(&self) -> String {
        self.anthropic_base_url
            .clone()
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| "https://api.anthropic.com".to_string())
    }

    pub fn anthropic_api_key(&self) -> Result<String, anyhow::Error> {
        if let Some(key) = &self.anthropic_api_key {
            if !key.is_empty() {
                return Ok(key.clone());
            }
        }
        if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                return Ok(key);
            }
        }
        Ok("".to_string())
    }

    pub fn api_key(&self) -> Result<String, anyhow::Error> {
        match self.provider_kind() {
            ProviderKind::Anthropic => self.anthropic_api_key(),
            ProviderKind::OpenAi => self.openai_api_key(),
        }
    }

    pub fn provider_family(&self) -> String {
        let base = self.base_url.to_lowercase();
        if base.contains("deepseek") {
            "deepseek".to_string()
        } else if base.contains("moonshot") {
            "moonshot".to_string()
        } else if base.contains("bigmodel") {
            "glm".to_string()
        } else if base.contains("anthropic") {
            "anthropic".to_string()
        } else if base.contains("openai") {
            "openai".to_string()
        } else {
            "openai".to_string()
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(provider) = env::var("ZENE_PROVIDER") {
            if !provider.is_empty() {
                self.provider = provider;
            }
        }
        if let Ok(model) = env::var("ZENE_MODEL") {
            if !model.is_empty() {
                self.model = model;
            }
        }
        if let Ok(base_url) = env::var("ZENE_BASE_URL") {
            if !base_url.is_empty() {
                self.base_url = base_url;
            }
        }
        if let Ok(base_url) = env::var("ZENE_ANTHROPIC_BASE_URL") {
            if !base_url.is_empty() {
                self.anthropic_base_url = Some(base_url);
            }
        }
        if let Ok(key) = env::var("ZENE_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        } else if let Ok(key) = env::var("DEEPSEEK_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        } else if let Ok(key) = env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        } else if let Ok(key) = env::var("MOONSHOT_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        } else if let Ok(key) = env::var("ZHIPUAI_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        } else if let Ok(key) = env::var("ZHIPU_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        }
        if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                self.anthropic_api_key = Some(key);
            }
        }
        if let Ok(key) = env::var("ZENE_WEB_SEARCH_API_KEY") {
            if !key.is_empty() {
                self.web_search.api_key = Some(key);
            }
        }
        if let Ok(profile) = env::var("ZENE_AGENT_PROFILE") {
            if !profile.is_empty() {
                if let Ok(parsed) = AgentProfile::parse(&profile) {
                    self.agent_profile = parsed;
                }
            }
        }
        if let Ok(profile) = env::var("ZENE_SANDBOX") {
            if !profile.is_empty() {
                self.sandbox.profile = Some(normalize_sandbox_profile(&profile));
            }
        }
        if let Ok(hosts) = env::var("ZENE_SANDBOX_ALLOW_HOSTS") {
            let parsed: Vec<String> = hosts
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !parsed.is_empty() {
                self.sandbox.allow_hosts = parsed;
            }
        }
        if let Ok(flag) = env::var("ZENE_SANDBOX_AUTO_ALLOW_BASH") {
            let lower = flag.trim().to_lowercase();
            if matches!(lower.as_str(), "1" | "true" | "yes" | "on") {
                self.sandbox.auto_allow_bash = true;
            } else if matches!(lower.as_str(), "0" | "false" | "no" | "off") {
                self.sandbox.auto_allow_bash = false;
            }
        }
        // 0 = unlimited; invalid values are ignored.
        if let Ok(raw) = env::var("ZENE_MAX_TURNS") {
            if let Ok(n) = raw.trim().parse::<u32>() {
                self.max_turns = n;
            }
        }
    }

    /// Persist model/provider/base_url/api_key to `~/.zene/config.toml`.
    pub fn persist_connection_settings(&self) -> Result<(), ConfigError> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: path.clone(),
                source,
            })?;
        }

        let mut table = if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
                path: path.clone(),
                source,
            })?;
            match toml::from_str::<toml::Value>(&raw) {
                Ok(toml::Value::Table(t)) => t,
                _ => toml::map::Map::new(),
            }
        } else {
            toml::map::Map::new()
        };

        table.insert("provider".into(), toml::Value::String(self.provider.clone()));
        table.insert("model".into(), toml::Value::String(self.model.clone()));
        table.insert("base_url".into(), toml::Value::String(self.base_url.clone()));
        if let Some(ref key) = self.api_key {
            if !key.is_empty() {
                table.insert("api_key".into(), toml::Value::String(key.clone()));
            }
        }
        if let Some(ref url) = self.anthropic_base_url {
            if !url.is_empty() {
                table.insert(
                    "anthropic_base_url".into(),
                    toml::Value::String(url.clone()),
                );
            }
        }
        if let Some(ref key) = self.anthropic_api_key {
            if !key.is_empty() {
                table.insert(
                    "anthropic_api_key".into(),
                    toml::Value::String(key.clone()),
                );
            }
        }

        let raw = toml::to_string_pretty(&toml::Value::Table(table)).expect("config serializes");
        fs::write(&path, raw).map_err(|source| ConfigError::Write {
            path,
            source,
        })?;
        Ok(())
    }
}

pub fn load_hooks(from_config: &[HookEntry]) -> Result<Vec<HookEntry>, ConfigError> {
    let mut hooks = from_config.to_vec();
    let path = hooks_path();
    if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let file: HooksFile = serde_json::from_str(&raw).map_err(|source| ConfigError::ParseJson {
            path: path.clone(),
            source,
        })?;
        hooks.extend(file.hooks);
    }
    Ok(hooks)
}

pub fn default_context_window_for_model(model: &str) -> u32 {
    match model {
        "gpt-4o" | "gpt-4o-mini" | "gpt-4-turbo" | "gpt-4" => 128_000,
        "gpt-3.5-turbo" => 16_385,
        "deepseek-v4-flash" | "deepseek-v4-pro" | "deepseek-chat" | "deepseek-reasoner" => 1_000_000,
        m if m.starts_with("moonshot-") || m.starts_with("kimi-") => 128_000,
        "glm-4" | "glm-4-flash" | "glm-4-plus" | "glm-4-air" => 128_000,
        "claude-3-5-sonnet-20241022"
        | "claude-3-5-sonnet-latest"
        | "claude-3-5-haiku-20241022"
        | "claude-3-opus-20240229"
        | "claude-3-sonnet-20240229"
        | "claude-3-haiku-20240307" => 200_000,
        _ => DEFAULT_CONTEXT_WINDOW_TOKENS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn with_temp_home<F: FnOnce()>(test: F) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().expect("test lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let prev = env::var("ZENE_HOME").ok();
        env::set_var("ZENE_HOME", temp.path());
        test();
        match prev {
            Some(value) => env::set_var("ZENE_HOME", value),
            None => env::remove_var("ZENE_HOME"),
        }
    }

    #[test]
    fn load_hooks_merges_config_and_file() {
        with_temp_home(|| {
            fs::write(
                hooks_path(),
                r#"{"hooks":[{"event":"PostToolUse","command":"./post.sh"}]}"#,
            )
            .expect("write hooks.json");

            let from_config = vec![HookEntry {
                event: "PreToolUse".into(),
                command: "./pre.sh".into(),
            }];
            let loaded = load_hooks(&from_config).expect("load hooks");
            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded[0].event, "PreToolUse");
            assert_eq!(loaded[1].event, "PostToolUse");
        });
    }

    #[test]
    fn hooks_deserialize_from_config_toml() {
        let raw = r#"
[[hooks]]
event = "PreToolUse"
command = "./scripts/pre-tool.sh"
"#;
        let config: ZeneConfig = toml::from_str(raw).expect("parse config");
        assert_eq!(config.hooks.len(), 1);
        assert_eq!(config.hooks[0].event, "PreToolUse");
    }

    #[test]
    fn load_merges_global_and_project_config() {
        with_temp_home(|| {
            let workdir = tempfile::tempdir().expect("workdir");
            fs::create_dir_all(workdir.path().join(".zene")).expect("project .zene");

            fs::write(
                config_path(),
                r#"
model = "gpt-4o"
max_turns = 50
permission_mode = "manual"

[compaction]
trigger_ratio = 0.85
context_window_tokens = 128000
"#,
            )
            .expect("write global config");

            fs::write(
                project_config_path(workdir.path()),
                r#"
model = "deepseek-chat"
permission_mode = "yolo"

[compaction]
trigger_ratio = 0.9

[model_context_windows]
deepseek-chat = 64000
"#,
            )
            .expect("write project config");

            let config = ZeneConfig::load(workdir.path()).expect("load config");
            assert_eq!(config.model, "deepseek-chat");
            assert_eq!(config.max_turns, 50);
            assert_eq!(config.permission_mode, "yolo");
            assert!((config.compaction.trigger_ratio - 0.9).abs() < f32::EPSILON);
            assert_eq!(config.compaction.context_window_tokens, 64_000);
            assert_eq!(
                config.model_context_windows.get("deepseek-chat"),
                Some(&64_000)
            );
        });
    }

    #[test]
    fn load_uses_global_when_no_project_config() {
        with_temp_home(|| {
            let workdir = tempfile::tempdir().expect("workdir");

            fs::write(
                config_path(),
                r#"
model = "gpt-4o-mini"
"#,
            )
            .expect("write global config");

            let config = ZeneConfig::load(workdir.path()).expect("load config");
            assert_eq!(config.model, "gpt-4o-mini");
        });
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;

    #[test]
    fn provider_kind_parsing() {
        assert_eq!(
            ProviderKind::parse("openai").unwrap(),
            ProviderKind::OpenAi
        );
        assert_eq!(
            ProviderKind::parse("openai-compatible").unwrap(),
            ProviderKind::OpenAi
        );
        assert_eq!(
            ProviderKind::parse("anthropic").unwrap(),
            ProviderKind::Anthropic
        );
        assert!(ProviderKind::parse("unknown").is_err());
    }

    #[test]
    fn web_search_config_defaults_to_duckduckgo() {
        let config = WebSearchConfig::default();
        assert_eq!(config.provider, "duckduckgo");
        assert_eq!(config.effective_provider(), WebSearchProviderKind::DuckDuckGo);
    }

    #[test]
    fn web_search_tavily_when_configured() {
        let config = WebSearchConfig {
            provider: "tavily".into(),
            api_key: Some("tvly-test".into()),
        };
        assert_eq!(config.effective_provider(), WebSearchProviderKind::Tavily);
        assert_eq!(config.resolved_api_key().as_deref(), Some("tvly-test"));
    }

    #[test]
    fn context_window_defaults() {
        let config = ZeneConfig {
            model: "gpt-4o".to_string(),
            ..Default::default()
        };
        assert_eq!(config.context_window_for_model(), 128_000);
    }

    #[test]
    fn agent_profile_parsing() {
        assert_eq!(AgentProfile::parse("full").unwrap(), AgentProfile::Full);
        assert_eq!(AgentProfile::parse("explore").unwrap(), AgentProfile::Explore);
        assert_eq!(AgentProfile::parse("coder").unwrap(), AgentProfile::Coder);
        assert!(AgentProfile::parse("unknown").is_err());
    }

    #[test]
    fn sandbox_effective_profile_defaults() {
        let settings = SandboxSettings::default();
        assert_eq!(
            settings.effective_profile(AgentProfile::Full),
            "workspace"
        );
        assert_eq!(
            settings.effective_profile(AgentProfile::Explore),
            "read-only"
        );
        let strict = SandboxSettings {
            profile: Some("strict".into()),
            ..SandboxSettings::default()
        };
        assert_eq!(
            strict.effective_profile(AgentProfile::Explore),
            "strict"
        );
    }
}

pub fn zene_home() -> PathBuf {
    if let Ok(home) = env::var("ZENE_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zene")
}

pub fn config_path() -> PathBuf {
    zene_home().join("config.toml")
}

pub fn project_config_path(workdir: &Path) -> PathBuf {
    workdir.join(".zene").join("config.toml")
}

pub fn hooks_path() -> PathBuf {
    zene_home().join("hooks.json")
}

pub fn mcp_config_path() -> PathBuf {
    zene_home().join("mcp.json")
}

pub fn sessions_dir() -> PathBuf {
    zene_home().join("sessions")
}

pub fn ensure_home() -> Result<(), ConfigError> {
    fs::create_dir_all(zene_home()).map_err(|source| ConfigError::Write {
        path: zene_home(),
        source,
    })?;
    fs::create_dir_all(sessions_dir()).map_err(|source| ConfigError::Write {
        path: sessions_dir(),
        source,
    })?;
    Ok(())
}

pub fn workdir_slug(workdir: &Path) -> String {
    let canonical = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
    let raw = canonical.display().to_string();
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
