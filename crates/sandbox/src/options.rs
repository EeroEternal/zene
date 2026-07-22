use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
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
    if matches!(profile, "workspace" | "read-only" | "strict") {
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
        "workspace" => profile_workspace(workdir).map_err(|err| anyhow::anyhow!("{err}")),
        "read-only" => profile_read_only(workdir).map_err(|err| anyhow::anyhow!("{err}")),
        "strict" => profile_strict(workdir).map_err(|err| anyhow::anyhow!("{err}")),
        other => anyhow::bail!("unknown built-in sandbox profile `{other}`"),
    }
}

fn inject_default_secret_denies(policy: &mut Policy) {
    // Keel ≥0.0.12 already merges baseline credential/secret denies into built-in
    // profiles. Keep a small Zene-specific deny (auth store) and any gaps, but skip
    // extra kernel deny injection on Linux when bubblewrap is missing — host
    // `path_policy` still protects agent Read/Write tools.
    if cfg!(target_os = "linux") && !bubblewrap_usable_for_keel() {
        tracing::warn!(
            "bubblewrap unusable for Keel denies; extra credential denies are host path_policy only"
        );
        return;
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));

    // Zene-specific path not covered by Keel baseline.
    let absolute = [home.join(".zene").join("auth")];
    for path in absolute {
        if !policy
            .fs
            .iter()
            .any(|rule| rule.access == FsAccess::Deny && !rule.glob && rule.path == path)
        {
            policy.fs.push(FsRule::deny(path));
        }
    }
}

pub(crate) fn bubblewrap_available() -> bool {
    std::process::Command::new("bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Whether Keel-style deny bind-overs can actually run on this host.
///
/// Keel baseline includes paths like `/etc/master.passwd` that may not exist;
/// creating those mount points often fails in restricted CI/containers.
pub(crate) fn bubblewrap_usable_for_keel() -> bool {
    use std::os::unix::fs::PermissionsExt;

    if !bubblewrap_available() {
        return false;
    }

    let placeholder = std::env::temp_dir().join(format!(
        "zene-bwrap-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    if std::fs::write(&placeholder, b"").is_err() {
        return false;
    }
    let _ = std::fs::set_permissions(&placeholder, std::fs::Permissions::from_mode(0o000));

    // Probe a missing absolute deny target similar to Keel baseline.
    let ok = std::process::Command::new("bwrap")
        .args([
            "--bind",
            "/",
            "/",
            "--dev-bind",
            "/dev",
            "/dev",
            "--proc",
            "/proc",
            "--ro-bind",
        ])
        .arg(&placeholder)
        .arg("/etc/master.passwd")
        .args(["--", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let _ = std::fs::set_permissions(&placeholder, std::fs::Permissions::from_mode(0o600));
    let _ = std::fs::remove_file(&placeholder);
    ok
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
