use zene_config::ZeneConfig;
use zene_sandbox::{url_host_port, SandboxOptions};

/// Build Keel sandbox options from config + optional CLI `--sandbox` override.
pub fn build_sandbox_options(config: &ZeneConfig, cli_profile: Option<&str>) -> SandboxOptions {
    let profile = cli_profile
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| config.sandbox.effective_profile(config.agent_profile));

    let mut trusted_hosts = Vec::new();
    push_url_host(&mut trusted_hosts, &config.base_url);
    if let Some(url) = config.anthropic_base_url.as_deref() {
        push_url_host(&mut trusted_hosts, url);
    }

    SandboxOptions {
        profile,
        allow_hosts: config.sandbox.allow_hosts.clone(),
        trusted_hosts,
    }
}

fn push_url_host(out: &mut Vec<String>, url: &str) {
    if let Ok((host, port)) = url_host_port(url) {
        let entry = if port == 443 || port == 80 {
            host
        } else {
            format!("{host}:{port}")
        };
        if !out.iter().any(|h| h.eq_ignore_ascii_case(&entry)) {
            out.push(entry);
        }
    }
}
