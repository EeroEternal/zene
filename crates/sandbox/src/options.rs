use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use keel_core::LocalProcessOptions;
use keel_core::{
    profile_read_only, profile_strict, profile_workspace, FsAccess, FsRule, NetworkPolicy,
    NetworkRule, Policy, SandboxConfig,
};

/// How to construct a Keel-backed [`crate::LocalSandbox`].
#[derive(Debug, Clone)]
pub struct SandboxOptions {
    /// `off` | `workspace` | `read-only` | `strict` | custom profile name.
    pub profile: String,
    /// Extra egress allowlist hosts (`host` or `host:port`).
    pub allow_hosts: Vec<String>,
    /// Hosts always merged into an allowlist when network is restricted
    /// (typically the configured LLM `base_url` host).
    pub trusted_hosts: Vec<String>,
}

impl Default for SandboxOptions {
    fn default() -> Self {
        Self {
            profile: "workspace".to_string(),
            allow_hosts: Vec::new(),
            trusted_hosts: Vec::new(),
        }
    }
}

impl SandboxOptions {
    pub fn normalized_profile(&self) -> String {
        normalize_profile(&self.profile)
    }

    pub fn is_off(&self) -> bool {
        self.normalized_profile() == "off"
    }
}

pub(crate) fn normalize_profile(profile: &str) -> String {
    match profile.trim().to_lowercase().as_str() {
        "readonly" | "read_only" => "read-only".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn resolve_policy(workdir: &Path, opts: &SandboxOptions) -> Result<Policy> {
    let profile = opts.normalized_profile();
    anyhow::ensure!(profile != "off", "resolve_policy called for off profile");

    let mut policy = load_named_policy(workdir, &profile)
        .with_context(|| format!("resolve sandbox profile `{profile}`"))?;

    inject_default_secret_denies(&mut policy);
    apply_network_overrides(&mut policy, opts)?;
    policy
        .validate()
        .map_err(|err| anyhow::anyhow!("invalid sandbox policy: {err}"))?;
    Ok(policy)
}

fn load_named_policy(workdir: &Path, profile: &str) -> Result<Policy> {
    let cfg = load_zene_sandbox_config(workdir);
    if matches!(profile, "workspace" | "read-only" | "strict" | "agentcell") {
        if cfg.profiles.contains_key(profile) {
            return cfg
                .resolve_policy(profile, workdir)
                .map_err(|err| anyhow::anyhow!("{err}"));
        }
        return builtin_policy(profile, workdir);
    }

    if cfg.profiles.contains_key(profile) {
        return cfg
            .resolve_policy(profile, workdir)
            .map_err(|err| anyhow::anyhow!("{err}"));
    }

    // Fall back to Keel's ~/.keel/sandbox.toml + <ws>/.keel/sandbox.toml.
    keel_core::load_policy_from_sandbox_toml(workdir, profile)
        .map_err(|err| anyhow::anyhow!("{err}"))
}

fn load_zene_sandbox_config(workdir: &Path) -> SandboxConfig {
    let home = std::env::var_os("ZENE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".zene")))
        .unwrap_or_else(|| PathBuf::from(".zene"));
    SandboxConfig::load_from(
        home.join("sandbox.toml"),
        workdir.join(".zene").join("sandbox.toml"),
    )
}

fn builtin_policy(profile: &str, workdir: &Path) -> Result<Policy> {
    match profile {
        "workspace" | "agentcell" => {
            profile_workspace(workdir).map_err(|err| anyhow::anyhow!("{err}"))
        }
        "read-only" => profile_read_only(workdir).map_err(|err| anyhow::anyhow!("{err}")),
        "strict" => profile_strict(workdir).map_err(|err| anyhow::anyhow!("{err}")),
        other => anyhow::bail!("unknown built-in sandbox profile `{other}`"),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn local_process_options() -> LocalProcessOptions {
    LocalProcessOptions::default()
}

/// Keel 0.0.12–0.0.15 always outer-wraps with `bwrap` when FS deny rules exist
/// (even if `auto_bwrap` is false), then applies Landlock in `pre_exec` on that
/// bwrap process — which fails with `setting up uid map: Permission denied`.
///
/// On Linux, drop FS denies from the Keel policy so children use Landlock-only
/// isolation. Credential gating stays on host [`crate::path_policy`]. Re-enable
/// Keel denies once Keel applies isolation *inside* the bwrap jail.
#[cfg(target_os = "linux")]
pub(crate) fn adapt_policy_for_keel_spawn(policy: &mut Policy) {
    let before = policy.fs.len();
    policy.fs.retain(|rule| rule.access != FsAccess::Deny);
    if policy.fs.len() != before {
        tracing::warn!(
            removed = before - policy.fs.len(),
            "stripped Keel FS deny rules on Linux (bwrap+Landlock pre_exec userns bug in eero-keel 0.0.15); host path_policy still denies credentials"
        );
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn adapt_policy_for_keel_spawn(policy: &mut Policy) {
    let _ = policy;
}

fn inject_default_secret_denies(policy: &mut Policy) {
    // Keel ≥0.0.12 baseline already covers ~/.ssh, ~/.aws, **/.env*, **/*.pem, etc.
    // Keep Zene-specific + overlapping denies for macOS Seatbelt / soft SpaceFs.
    // Linux strips denies in [`adapt_policy_for_keel_spawn`] before Space::create.
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));

    let absolute = [
        home.join(".ssh"),
        home.join(".gnupg"),
        home.join(".aws"),
        home.join(".azure"),
        home.join(".config").join("gcloud"),
        home.join(".zene").join("auth"),
    ];
    for path in absolute {
        if !policy
            .fs
            .iter()
            .any(|rule| rule.access == FsAccess::Deny && !rule.glob && rule.path == path)
        {
            policy.fs.push(FsRule::deny(path));
        }
    }

    for pattern in ["**/.env", "**/.env.*", "**/*.pem", "**/*.key"] {
        let path = PathBuf::from(pattern);
        if !policy
            .fs
            .iter()
            .any(|rule| rule.access == FsAccess::Deny && rule.glob && rule.path == path)
        {
            policy.fs.push(FsRule::deny_glob(path));
        }
    }
}

fn apply_network_overrides(policy: &mut Policy, opts: &SandboxOptions) -> Result<()> {
    // Only user-configured allow_hosts trigger an allowlist. Trusted LLM hosts are
    // merged in so child processes can still reach the model API when restricted.
    if opts.allow_hosts.is_empty() {
        return Ok(());
    }

    let mut hosts = opts.allow_hosts.clone();
    for host in &opts.trusted_hosts {
        if !hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            hosts.push(host.clone());
        }
    }

    let rules = parse_allow_hosts(&hosts)?;
    // Restrictive profiles start as DenyAll; allow_hosts lifts them to an allowlist.
    // Workspace (Unrestricted) becomes an allowlist when hosts are configured.
    policy.network = NetworkPolicy::Allowlist(rules);
    Ok(())
}

fn parse_allow_hosts(hosts: &[String]) -> Result<Vec<NetworkRule>> {
    let mut rules = Vec::new();
    for host in hosts {
        let h = host.trim();
        if h.is_empty() {
            continue;
        }
        if let Some((name, port_s)) = h.rsplit_once(':') {
            if let Ok(port) = port_s.parse::<u16>() {
                rules.push(NetworkRule::host_port(name, port));
                continue;
            }
        }
        rules.push(NetworkRule::host(h));
    }
    anyhow::ensure!(!rules.is_empty(), "allow_hosts is empty after parsing");
    Ok(rules)
}

/// Extract host + port from an HTTP(S) URL for egress checks.
pub fn url_host_port(url: &str) -> Result<(String, u16)> {
    let url = url.trim();
    anyhow::ensure!(!url.is_empty(), "empty URL");

    let without_scheme = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("HTTPS://"))
        .or_else(|| url.strip_prefix("HTTP://"))
    {
        rest
    } else if url.contains("://") {
        anyhow::bail!("unsupported URL scheme (only http/https): {url}");
    } else {
        url
    };

    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    anyhow::ensure!(!authority.is_empty(), "URL missing host: {url}");

    let default_port = if url.to_ascii_lowercase().starts_with("http://") {
        80
    } else {
        443
    };

    if let Some(host) = authority.strip_prefix('[') {
        // IPv6 literal [::1]:port
        let (host, rest) = host
            .split_once(']')
            .ok_or_else(|| anyhow::anyhow!("invalid IPv6 URL host: {url}"))?;
        let port = if let Some(port_s) = rest.strip_prefix(':') {
            port_s
                .parse::<u16>()
                .with_context(|| format!("invalid port in URL: {url}"))?
        } else {
            default_port
        };
        return Ok((host.to_string(), port));
    }

    if let Some((host, port_s)) = authority.rsplit_once(':') {
        if !host.is_empty() && port_s.chars().all(|c| c.is_ascii_digit()) {
            let port: u16 = port_s
                .parse()
                .with_context(|| format!("invalid port in URL: {url}"))?;
            return Ok((host.to_string(), port));
        }
    }

    Ok((authority.to_string(), default_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_host_port_https_default() {
        let (host, port) = url_host_port("https://api.openai.com/v1/models").unwrap();
        assert_eq!(host, "api.openai.com");
        assert_eq!(port, 443);
    }

    #[test]
    fn url_host_port_custom() {
        let (host, port) = url_host_port("http://localhost:8080/foo").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 8080);
    }

    #[test]
    fn network_override_builds_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = profile_workspace(dir.path()).unwrap();
        let opts = SandboxOptions {
            profile: "workspace".into(),
            allow_hosts: vec!["example.com:443".into()],
            trusted_hosts: vec!["api.openai.com".into()],
        };
        apply_network_overrides(&mut policy, &opts).unwrap();
        match policy.network {
            NetworkPolicy::Allowlist(rules) => assert_eq!(rules.len(), 2),
            other => panic!("expected allowlist, got {other:?}"),
        }
    }

    #[test]
    fn trusted_hosts_alone_do_not_open_network() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = profile_read_only(dir.path()).unwrap();
        let opts = SandboxOptions {
            profile: "read-only".into(),
            allow_hosts: vec![],
            trusted_hosts: vec!["api.openai.com".into()],
        };
        apply_network_overrides(&mut policy, &opts).unwrap();
        assert!(matches!(policy.network, NetworkPolicy::DenyAll));
    }
}
