//! On-disk layout for gateway agent journals and metadata.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMeta {
    pub agent_id: String,
    pub workspace: PathBuf,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct DataStore {
    root: PathBuf,
}

impl DataStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("create gateway data dir {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn agent_dir(&self, agent_id: &str) -> PathBuf {
        self.root.join("agents").join(agent_id)
    }

    pub fn journal_path(&self, agent_id: &str) -> PathBuf {
        self.agent_dir(agent_id).join("journal.jsonl")
    }

    pub fn meta_path(&self, agent_id: &str) -> PathBuf {
        self.agent_dir(agent_id).join("meta.json")
    }

    pub fn write_meta(&self, meta: &AgentMeta) -> Result<()> {
        let dir = self.agent_dir(&meta.agent_id);
        fs::create_dir_all(&dir)?;
        let path = self.meta_path(&meta.agent_id);
        let body = serde_json::to_vec_pretty(meta)?;
        fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    pub fn read_meta(&self, agent_id: &str) -> Result<AgentMeta> {
        let path = self.meta_path(agent_id);
        if !path.exists() {
            bail!("unknown persisted agentId: {agent_id}");
        }
        let raw = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn list_agent_ids(&self) -> Result<Vec<String>> {
        let agents = self.root.join("agents");
        if !agents.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(agents)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    if self.meta_path(name).exists() {
                        ids.push(name.to_string());
                    }
                }
            }
        }
        ids.sort();
        Ok(ids)
    }
}

pub fn default_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("ZENE_GATEWAY_DATA") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zene")
        .join("gateway")
}
