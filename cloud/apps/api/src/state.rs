use std::path::PathBuf;

use zene_cloud_db::Db;
use zene_cloud_git_broker::GitBroker;
use zene_cloud_github::GithubClient;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub worker_token: String,
    pub github: GithubClient,
    pub git_broker: GitBroker,
    pub workspace_root: PathBuf,
    pub public_base_url: String,
}
