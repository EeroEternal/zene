use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use zene_config::zene_home;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
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
    serde_json::from_str(&raw)
        .with_context(|| format!("parse MCP config at {}", path.display()))
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
        assert_eq!(server.command, "node");
        assert_eq!(server.args, vec!["server.js"]);
        assert_eq!(server.env.get("FOO").map(String::as_str), Some("bar"));
    }

    #[test]
    fn merge_overrides_same_server_name() {
        let mut base = McpConfig {
            servers: HashMap::from([(
                "a".to_string(),
                McpServerConfig {
                    command: "global".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                },
            )]),
        };
        base.merge(McpConfig {
            servers: HashMap::from([
                (
                    "a".to_string(),
                    McpServerConfig {
                        command: "project".to_string(),
                        args: vec!["--local".to_string()],
                        env: HashMap::new(),
                    },
                ),
                (
                    "b".to_string(),
                    McpServerConfig {
                        command: "other".to_string(),
                        args: vec![],
                        env: HashMap::new(),
                    },
                ),
            ]),
        });
        assert_eq!(base.servers.len(), 2);
        assert_eq!(base.servers["a"].command, "project");
        assert_eq!(base.servers["b"].command, "other");
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

        // Write to the actual global path under temp ZENE_HOME
        fs::write(
            global_path,
            r#"{"servers":{"shared":{"command":"global","args":[],"env":{}}}}"#,
        )
        .unwrap();

        let loaded = load_mcp_config(&workdir).unwrap();
        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(loaded.servers["shared"].command, "project");
        assert_eq!(loaded.servers["local"].command, "local-cmd");
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
