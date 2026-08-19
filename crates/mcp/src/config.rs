use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use zene_config::zene_home;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// stdio transport: command to spawn.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// HTTP / Streamable HTTP transport endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Extra HTTP headers (Authorization, etc.).
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl McpServerConfig {
    pub fn is_http(&self) -> bool {
        self.url.as_ref().is_some_and(|u| !u.trim().is_empty())
    }

    pub fn is_stdio(&self) -> bool {
        self.command.as_ref().is_some_and(|c| !c.trim().is_empty())
    }

    pub fn validate(&self, name: &str) -> Result<()> {
        match (self.is_stdio(), self.is_http()) {
            (true, false) | (false, true) => Ok(()),
            (true, true) => {
                bail!("MCP server `{name}` sets both `command` and `url`; choose one transport")
            }
            (false, false) => {
                bail!("MCP server `{name}` needs either `command` (stdio) or `url` (HTTP)")
            }
        }
    }
}

pub fn global_mcp_config_path() -> PathBuf {
    zene_home().join("mcp.json")
}

pub fn project_mcp_config_path(workdir: &Path) -> PathBuf {
    workdir.join(".zene").join("mcp.json")
}

pub fn load_mcp_config(workdir: &Path) -> Result<McpConfig> {
    let mut merged = McpConfig::default();

    let global_path = global_mcp_config_path();
    if global_path.exists() {
        let config = parse_config_file(&global_path)?;
        merged.merge(config);
    }

    let project_path = project_mcp_config_path(workdir);
    if project_path.exists() {
        let config = parse_config_file(&project_path)?;
        merged.merge(config);
    }

    Ok(merged)
}

impl McpConfig {
    pub fn merge(&mut self, other: McpConfig) {
        self.servers.extend(other.servers);
    }
}

fn parse_config_file(path: &Path) -> Result<McpConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read MCP config at {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse MCP config at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parses_stdio_server_config() {
        let raw = r#"{
            "servers": {
                "demo": {
                    "command": "node",
                    "args": ["server.js"],
                    "env": { "FOO": "bar" }
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(raw).unwrap();
        let server = config.servers.get("demo").unwrap();
        assert_eq!(server.command.as_deref(), Some("node"));
        assert_eq!(server.args, vec!["server.js"]);
        assert_eq!(server.env.get("FOO").map(String::as_str), Some("bar"));
        assert!(server.is_stdio());
    }

    #[test]
    fn parses_http_server_config() {
        let raw = r#"{
            "servers": {
                "remote": {
                    "url": "https://example.com/mcp",
                    "headers": { "Authorization": "Bearer tok" }
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(raw).unwrap();
        let server = config.servers.get("remote").unwrap();
        assert!(server.is_http());
        assert_eq!(server.url.as_deref(), Some("https://example.com/mcp"));
        server.validate("remote").unwrap();
    }

    #[test]
    fn merge_overrides_same_server_name() {
        let mut base = McpConfig {
            servers: HashMap::from([(
                "a".to_string(),
                McpServerConfig {
                    command: Some("global".to_string()),
                    args: vec![],
                    env: HashMap::new(),
                    url: None,
                    headers: HashMap::new(),
                },
            )]),
        };
        base.merge(McpConfig {
            servers: HashMap::from([
                (
                    "a".to_string(),
                    McpServerConfig {
                        command: Some("project".to_string()),
                        args: vec!["--local".to_string()],
                        env: HashMap::new(),
                        url: None,
                        headers: HashMap::new(),
                    },
                ),
                (
                    "b".to_string(),
                    McpServerConfig {
                        command: Some("other".to_string()),
                        args: vec![],
                        env: HashMap::new(),
                        url: None,
                        headers: HashMap::new(),
                    },
                ),
            ]),
        });
        assert_eq!(base.servers.len(), 2);
        assert_eq!(base.servers["a"].command.as_deref(), Some("project"));
        assert_eq!(base.servers["b"].command.as_deref(), Some("other"));
    }

    #[test]
    fn load_merges_global_and_project_configs() {
        let temp = TempDir::new().unwrap();
        let workdir = temp.path().join("repo");
        fs::create_dir_all(workdir.join(".zene")).unwrap();

        fs::write(
            workdir.join(".zene/mcp.json"),
            r#"{"servers":{"shared":{"command":"project","args":[],"env":{}},"local":{"command":"local-cmd","args":[],"env":{}}}}"#,
        )
        .unwrap();

        let _guard = EnvOverride::set("ZENE_HOME", temp.path());
        let global_path = global_mcp_config_path();

        fs::write(
            global_path,
            r#"{"servers":{"shared":{"command":"global","args":[],"env":{}}}}"#,
        )
        .unwrap();

        let loaded = load_mcp_config(&workdir).unwrap();
        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(loaded.servers["shared"].command.as_deref(), Some("project"));
        assert_eq!(
            loaded.servers["local"].command.as_deref(),
            Some("local-cmd")
        );
    }

    struct EnvOverride;

    impl EnvOverride {
        fn set(key: &str, value: &Path) -> Self {
            unsafe { std::env::set_var(key, value) };
            Self
        }
    }

    impl Drop for EnvOverride {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("ZENE_HOME") };
        }
    }
}
