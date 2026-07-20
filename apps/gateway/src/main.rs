use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;
use zene_gateway::agent::{resolve_zene_bin, AgentManager};
use zene_gateway::auth::AuthState;
use zene_gateway::http::{self, AppState};
use zene_gateway::lease::LeaseManager;

#[derive(Debug, Parser)]
#[command(
    name = "zene-gateway",
    about = "Thin local HTTP gateway for Zene ACP / Web Agent UI"
)]
struct Cli {
    /// Address to bind. Defaults to loopback only.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,

    /// Port to listen on. Use 0 to let the OS assign an ephemeral port.
    #[arg(long, default_value_t = 8787)]
    port: u16,

    /// Shared access token. Generated when omitted.
    #[arg(long)]
    token: Option<String>,

    /// Path to the `zene` binary.
    #[arg(long)]
    zene_bin: Option<PathBuf>,

    /// Override the full ACP command. When set, `--zene-bin` is ignored.
    #[arg(long)]
    acp_command: Option<PathBuf>,

    /// Extra args for the ACP process after the command.
    /// Defaults to `acp` when using `zene`.
    #[arg(long = "acp-arg")]
    acp_args: Vec<String>,

    /// Pass `--yolo` to `zene` so gated tools auto-approve in Web UI.
    #[arg(long, default_value_t = false)]
    yolo: bool,

    /// Prefer Keel sandbox off for local Web demos (`zene --sandbox off`).
    #[arg(long, default_value_t = false)]
    sandbox_off: bool,

    /// Extra `KEY=VALUE` environment variables for the ACP child.
    #[arg(long = "acp-env")]
    acp_env: Vec<String>,

    /// Allow non-loopback binds. Required for any bind outside 127.0.0.1/::1.
    #[arg(long, default_value_t = false)]
    allow_remote: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    ensure_bind_safe(&cli.bind, cli.allow_remote)?;

    let token = cli.token.clone().unwrap_or_else(AuthState::generate_token);
    let (command, args) = resolve_acp_command(&cli)?;
    let env = parse_acp_env(&cli.acp_env)?;
    let agents = AgentManager::new(command.clone(), args.clone()).with_env(env);

    let listener = tokio::net::TcpListener::bind((cli.bind.as_str(), cli.port))
        .await
        .with_context(|| format!("failed to bind {}:{}", cli.bind, cli.port))?;
    let addr = listener.local_addr()?;
    let auth = AuthState::new(token.clone(), cli.bind.clone(), addr.port());
    let state = AppState {
        auth,
        agents,
        leases: LeaseManager::new(),
        started_at: chrono::Utc::now(),
        version: env!("CARGO_PKG_VERSION"),
    };

    let url = format!(
        "http://{}:{}/#token={}",
        display_host(&cli.bind),
        addr.port(),
        token
    );
    eprintln!("zene-gateway listening on {addr}");
    eprintln!("open {url}");
    eprintln!("acp command: {} {:?}", command.display(), args);

    let app = http::router(state);
    axum::serve(listener, app)
        .await
        .context("gateway server terminated")?;
    Ok(())
}

fn resolve_acp_command(cli: &Cli) -> Result<(PathBuf, Vec<String>)> {
    if let Some(command) = &cli.acp_command {
        return Ok((command.clone(), cli.acp_args.clone()));
    }
    let command = resolve_zene_bin(cli.zene_bin.clone());
    let mut args = Vec::new();
    if cli.yolo {
        args.push("--yolo".to_string());
    }
    if cli.sandbox_off {
        args.push("--sandbox".to_string());
        args.push("off".to_string());
    }
    if cli.acp_args.is_empty() {
        args.push("acp".to_string());
    } else {
        args.extend(cli.acp_args.clone());
    }
    Ok((command, args))
}

fn parse_acp_env(items: &[String]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for item in items {
        let Some((key, value)) = item.split_once('=') else {
            bail!("invalid --acp-env {item:?}; expected KEY=VALUE");
        };
        if key.is_empty() {
            bail!("invalid --acp-env {item:?}; empty key");
        }
        out.push((key.to_string(), value.to_string()));
    }
    Ok(out)
}

fn ensure_bind_safe(bind: &str, allow_remote: bool) -> Result<()> {
    let is_loopback = matches!(bind, "127.0.0.1" | "localhost" | "::1");
    if !is_loopback && !allow_remote {
        bail!(
            "refusing to bind {bind}; pass --allow-remote for non-loopback addresses (TLS/auth required for production remote use)"
        );
    }
    Ok(())
}

fn display_host(bind: &str) -> &str {
    if bind == "0.0.0.0" || bind == "::" {
        "127.0.0.1"
    } else {
        bind
    }
}
