use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use zene_cloud_db::Db;
use zene_cloud_github::GithubClient;

use zene_cloud_api::{router, AppState};

fn reject_weak_worker_token(token: &str) -> Result<()> {
    let allow_dev = std::env::var("ZENE_CLOUD_ALLOW_DEV_TOKEN")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let trimmed = token.trim();
    let weak = trimmed.is_empty()
        || trimmed == "dev-worker-token"
        || trimmed == "dev-worker-token-change-me"
        || trimmed.len() < 16;
    if weak && !allow_dev {
        anyhow::bail!(
            "refusing weak ZENE_CLOUD_WORKER_TOKEN; set a random secret or ZENE_CLOUD_ALLOW_DEV_TOKEN=1 for local dev"
        );
    }
    Ok(())
}

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

    #[arg(
        long,
        env = "ZENE_CLOUD_WORKER_TOKEN",
        default_value = "dev-worker-token"
    )]
    worker_token: String,

    #[arg(long, env = "ZENE_CLOUD_WEB_DIR", default_value = "apps/web/dist")]
    web_dir: PathBuf,

    #[arg(
        long,
        env = "ZENE_CLOUD_WORKSPACE_ROOT",
        default_value = "./data/workspaces"
    )]
    workspace_root: PathBuf,

    #[arg(
        long,
        env = "ZENE_CLOUD_PUBLIC_BASE_URL",
        default_value = "http://127.0.0.1:8788"
    )]
    public_base_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    reject_weak_worker_token(&cli.worker_token)?;
    std::fs::create_dir_all(&cli.workspace_root)?;

    let db = Db::connect(&cli.database_url).await?;
    db.migrate().await?;
    db.purge_all_mock_github_data().await?;
    tracing::info!("purged legacy mock GitHub data");
    db.ensure_dev_worker_token(&cli.worker_token).await?;

    let github = zene_cloud_github::from_env()
        .unwrap_or_else(|_| GithubClient::new(zene_cloud_github::GithubConfig::live_default()));
    let state = AppState::new(
        db,
        cli.worker_token.clone(),
        github,
        cli.workspace_root.clone(),
        cli.public_base_url.clone(),
    );

    let api = router(state);
    let cors_origins = std::env::var("ZENE_CLOUD_CORS_ORIGINS").unwrap_or_default();
    let allowed_origins: Vec<axum::http::HeaderValue> = if cors_origins.is_empty() {
        vec![cli
            .public_base_url
            .parse()
            .unwrap_or_else(|_| "http://127.0.0.1:8788".parse().expect("valid header"))]
    } else {
        cors_origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    };
    let app = api
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(allowed_origins))
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .fallback_service(ServeDir::new(&cli.web_dir));

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    tracing::info!("zene-cloud-api listening on http://{}", cli.bind);
    tracing::info!(
        github_mode = ?std::env::var("ZENE_CLOUD_GITHUB_MODE").unwrap_or_else(|_| "live".into()),
        "github + git broker ready"
    );
    axum::serve(listener, app).await?;
    Ok(())
}
