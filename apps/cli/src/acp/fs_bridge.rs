//! ACP client filesystem bridge (`fs/read_text_file`, `fs/write_text_file`).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::json;
use zene_sandbox::RemoteTextFs;

use super::transport::AcpWriter;

pub struct AcpRemoteFs {
    writer: AcpWriter,
    session_id: String,
    can_read: bool,
    can_write: bool,
}

impl AcpRemoteFs {
    pub fn new(
        writer: AcpWriter,
        session_id: impl Into<String>,
        can_read: bool,
        can_write: bool,
    ) -> Self {
        Self {
            writer,
            session_id: session_id.into(),
            can_read,
            can_write,
        }
    }
}

#[async_trait]
impl RemoteTextFs for AcpRemoteFs {
    fn can_read(&self) -> bool {
        self.can_read
    }

    fn can_write(&self) -> bool {
        self.can_write
    }

    async fn read_text(&self, absolute_path: &Path) -> Result<String> {
        let path = absolute_path
            .to_str()
            .ok_or_else(|| anyhow!("path is not valid UTF-8"))?;
        let result = self
            .writer
            .request(
                "fs/read_text_file",
                json!({
                    "sessionId": self.session_id,
                    "path": path,
                }),
            )
            .await
            .context("ACP fs/read_text_file")?;
        result
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("fs/read_text_file missing content"))
    }

    async fn write_text(&self, absolute_path: &Path, content: &str) -> Result<()> {
        let path = absolute_path
            .to_str()
            .ok_or_else(|| anyhow!("path is not valid UTF-8"))?;
        self.writer
            .request(
                "fs/write_text_file",
                json!({
                    "sessionId": self.session_id,
                    "path": path,
                    "content": content,
                }),
            )
            .await
            .context("ACP fs/write_text_file")?;
        Ok(())
    }
}
