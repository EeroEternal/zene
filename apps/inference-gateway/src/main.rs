use std::net::SocketAddr;

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;
use zene_inference_gateway::{GatewayOptions, build_gateway};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zene_inference_gateway=info,tower_http=info".into()),
        )
        .init();

    let options = GatewayOptions::from_env();
    let listen = std::env::var("ZENE_GATEWAY_LISTEN").unwrap_or_else(|_| "127.0.0.1:8790".into());
    let addr: SocketAddr = listen.parse().context("parse ZENE_GATEWAY_LISTEN")?;

    let app = build_gateway(options.clone()).await?;

    info!(
        %addr,
        upstream = %options.upstream_url,
        redis = options.session_config.using_redis,
        fingerprint = ?options.session_config.fingerprint_policy,
        "zene inference gateway (unigateway 2.14) starting"
    );
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
