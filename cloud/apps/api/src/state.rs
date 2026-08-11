use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use uuid::Uuid;
use zene_cloud_db::Db;
use zene_cloud_git_broker::GitBroker;
use zene_cloud_github::{GithubClient, GithubConfig};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub worker_token: String,
    pub github: Arc<RwLock<GithubClient>>,
    pub git_broker: Arc<RwLock<GitBroker>>,
    pub workspace_root: PathBuf,
    pub public_base_url: String,
}

impl AppState {
    pub fn new(
        db: Db,
        worker_token: String,
        github: GithubClient,
        workspace_root: PathBuf,
        public_base_url: String,
    ) -> Self {
        let git_broker = GitBroker::new(db.clone(), github.clone());
        Self {
            db,
            worker_token,
            github: Arc::new(RwLock::new(github)),
            git_broker: Arc::new(RwLock::new(git_broker)),
            workspace_root,
            public_base_url,
        }
    }

    pub async fn github_client(&self) -> GithubClient {
        self.github.read().await.clone()
    }

    pub async fn git_broker(&self) -> GitBroker {
        self.git_broker.read().await.clone()
    }

    pub async fn reload_github_for_org(&self, org_id: Uuid) -> Result<GithubClient> {
        let stored = self.db.get_github_provider_config(org_id).await?;
        let config = GithubConfig::merge_env_and_db(stored);
        let client = GithubClient::new(config);
        *self.github.write().await = client.clone();
        *self.git_broker.write().await = GitBroker::new(self.db.clone(), client.clone());
        Ok(client)
    }
}
