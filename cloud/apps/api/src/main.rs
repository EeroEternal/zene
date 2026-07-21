mod auth;
mod error;
mod routes;
mod state;
mod workspace;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use zene_cloud_db::Db;
use zene_cloud_git_broker::GitBroker;
use zene_cloud_github::GithubClient;

use crate::state::AppState;

#[derive(Debug, Parser)]
#[command(name = "zene-cloud-api")]
struct Cli {
    #[arg(long, env = "ZENE_CLOUD_BIND", default_value = "127.0.0.1:8788")]
    bind: SocketAddr,

    #[arg(
        long,
        env = "ZENE_CLOUD_DATABASE_URL",
        default_value = "sqlite:./data/zene-cloud.db"
    )]
    database_url: String,

    #[arg(long, env = "ZENE_CLOUD_WORKER_TOKEN", default_value = "dev-worker-token")]
    worker_token: String,

    #[arg(long, env = "ZENE_CLOUD_WEB_DIR", default_value = "apps/web/dist")]
    web_dir: PathBuf,

    #[arg(long, env = "ZENE_CLOUD_WORKSPACE_ROOT", default_value = "./data/workspaces")]
    workspace_root: PathBuf,

    #[arg(long, env = "ZENE_CLOUD_PUBLIC_BASE_URL", default_value = "http://127.0.0.1:8788")]
    public_base_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.workspace_root)?;

    let db = Db::connect(&cli.database_url).await?;
    db.migrate().await?;
    db.ensure_dev_worker_token(&cli.worker_token).await?;

    let github = zene_cloud_github::from_env().unwrap_or_else(|_| GithubClient::mock());
    let git_broker = GitBroker::new(db.clone(), github.clone());

    let state = AppState {
        db,
        worker_token: cli.worker_token.clone(),
        github,
        git_broker,
        workspace_root: cli.workspace_root.clone(),
        public_base_url: cli.public_base_url.clone(),
    };

    let api = routes::router(state);
    let app = api
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .fallback_service(ServeDir::new(&cli.web_dir));

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    tracing::info!("zene-cloud-api listening on http://{}", cli.bind);
    tracing::info!(
        github_mode = ?std::env::var("ZENE_CLOUD_GITHUB_MODE").unwrap_or_else(|_| "mock".into()),
        "github + git broker ready"
    );
    axum::serve(listener, app).await?;
    Ok(())
}
