mod auth;
mod error;
mod routes;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use zene_cloud_db::Db;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    let db = Db::connect(&cli.database_url).await?;
    db.migrate().await?;
    db.ensure_dev_worker_token(&cli.worker_token).await?;

    let state = AppState {
        db,
        worker_token: cli.worker_token.clone(),
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
    tracing::info!("worker token configured (dev)");
    axum::serve(listener, app).await?;
    Ok(())
}
