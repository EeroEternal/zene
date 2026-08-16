use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use uuid::Uuid;
use zene_cloud_db::Db;
use zene_cloud_domain::Run;
use zene_cloud_git_broker::GitBroker;
use zene_cloud_github::{GithubClient, GithubConfig};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub worker_token: String,
    github_clients: Arc<RwLock<HashMap<Uuid, GithubClient>>>,
    git_brokers: Arc<RwLock<HashMap<Uuid, GitBroker>>>,
    fallback_github: Arc<RwLock<GithubClient>>,
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
        Self {
            db,
            worker_token,
            github_clients: Arc::new(RwLock::new(HashMap::new())),
            git_brokers: Arc::new(RwLock::new(HashMap::new())),
            fallback_github: Arc::new(RwLock::new(github)),
            workspace_root,
            public_base_url,
        }
    }

    /// On-disk checkout the worker actually uses (`ws/{workspace_id}`).
    pub fn run_checkout_dir(&self, run: &Run) -> PathBuf {
        if run.workspace_id.is_nil() {
            self.workspace_root.join(run.id.to_string())
        } else {
            Db::workspace_checkout_dir(&self.workspace_root, run.workspace_id)
        }
    }

    pub async fn github_client(&self) -> GithubClient {
        self.fallback_github.read().await.clone()
    }

    pub async fn github_client_for_org(&self, org_id: Uuid) -> Result<GithubClient> {
        {
            let map = self.github_clients.read().await;
            if let Some(client) = map.get(&org_id) {
                return Ok(client.clone());
            }
        }
        self.reload_github_for_org(org_id).await
    }

    pub async fn git_broker_for_org(&self, org_id: Uuid) -> Result<GitBroker> {
        {
            let map = self.git_brokers.read().await;
            if let Some(broker) = map.get(&org_id) {
                return Ok(broker.clone());
            }
        }
        let client = self.github_client_for_org(org_id).await?;
        let broker = GitBroker::new(self.db.clone(), client);
        self.git_brokers.write().await.insert(org_id, broker.clone());
        Ok(broker)
    }

    pub async fn reload_github_for_org(&self, org_id: Uuid) -> Result<GithubClient> {
        let stored = self.db.get_github_provider_config(org_id).await?;
        let config = GithubConfig::merge_env_and_db(stored);
        let client = GithubClient::new(config);
        let broker = GitBroker::new(self.db.clone(), client.clone());
        self.github_clients.write().await.insert(org_id, client.clone());
        self.git_brokers.write().await.insert(org_id, broker);
        Ok(client)
    }
}
