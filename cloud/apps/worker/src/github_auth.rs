//! Per-run GitHub credentials for the agent process.
//!
//! Clone tokens are short-lived installation tokens. Leaving them in `git remote`
//! URLs or inheriting the worker host's `gh` login (`~/.config/gh`, `GH_TOKEN`)
//! makes later `gh` / GitHub API calls fail with Bad credentials.
//!
//! Files live under `{workspace}/.zene/github/` so the agent sandbox can read
//! them (host `~/.config/gh` is outside the workspace and shows up as a config
//! permission error). The token file is refreshed in the background. Git uses a
//! credential helper; `gh` is wrapped so each invocation reads the current file.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;
use tracing::{info, warn};
use zene_cloud_domain::CloneAuthResponse;

pub fn auth_dir(workspace: &Path) -> PathBuf {
    workspace.join(".zene").join("github")
}

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join("token")
}

pub fn bin_dir(dir: &Path) -> PathBuf {
    dir.join("bin")
}

pub fn github_repo_slug(url: &str) -> Option<String> {
    let url = strip_userinfo(url);
    let url = url.trim_end_matches('/').trim_end_matches(".git");
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let mut parts = rest.split('/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    Some(format!("{owner}/{repo}"))
}

pub fn strip_userinfo(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        if let Some(at) = rest.find('@') {
            return format!("https://{}", &rest[at + 1..]);
        }
    }
    if let Some(rest) = url.strip_prefix("http://") {
        if let Some(at) = rest.find('@') {
            return format!("http://{}", &rest[at + 1..]);
        }
    }
    url.to_string()
}

fn shell_lit(path: &Path) -> String {
    path.display().to_string().replace('\'', "'\\''")
}

async fn chmod(path: &Path, mode: u32) -> Result<()> {
    let mut perms = tokio::fs::metadata(path).await?.permissions();
    perms.set_mode(mode);
    tokio::fs::set_permissions(path, perms).await?;
    Ok(())
}

async fn mkdir_private(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    chmod(path, 0o700).await?;
    Ok(())
}

pub async fn write_token_file(dir: &Path, token: &str) -> Result<PathBuf> {
    mkdir_private(dir).await?;
    let path = token_path(dir);
    tokio::fs::write(&path, token.trim()).await?;
    chmod(&path, 0o600).await?;
    Ok(path)
}

fn real_gh() -> Option<PathBuf> {
    let output = std::process::Command::new("which").arg("gh").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = PathBuf::from(path.trim());
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

async fn install_pre_push_hook(workspace: &Path) -> Result<()> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .await
        .context("git rev-parse hooks")?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse --git-path hooks failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let rel = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let hooks_dir = {
        let p = PathBuf::from(&rel);
        if p.is_absolute() {
            p
        } else {
            workspace.join(p)
        }
    };
    tokio::fs::create_dir_all(&hooks_dir).await?;
    let hook = hooks_dir.join("pre-push");
    tokio::fs::write(
        &hook,
        "#!/bin/sh\n\
         echo 'git push is disabled in Zene Cloud. Use the PublishGithub tool or Console Commit & Create PR.' >&2\n\
         exit 1\n",
    )
    .await?;
    chmod(&hook, 0o700).await?;
    Ok(())
}

async fn ensure_git_exclude(workspace: &Path) -> Result<()> {
    let exclude = workspace.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let marker = ".zene/";
    let existing = tokio::fs::read_to_string(&exclude).await.unwrap_or_default();
    if existing.lines().any(|line| line.trim() == marker) {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(marker);
    next.push('\n');
    tokio::fs::write(&exclude, next).await?;
    Ok(())
}

async fn write_helpers(dir: &Path) -> Result<PathBuf> {
    let bin = bin_dir(dir);
    mkdir_private(&bin).await?;
    let token_lit = shell_lit(&token_path(dir));
    let gh_config_lit = shell_lit(&dir.join("gh"));
    let state_lit = shell_lit(&dir.join("xdg-state"));
    let cache_lit = shell_lit(&dir.join("xdg-cache"));
    let data_lit = shell_lit(&dir.join("xdg-data"));

    let cred = bin.join("git-credential-zene");
    tokio::fs::write(
        &cred,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" != \"get\" ]; then exit 0; fi\n\
             while IFS= read -r line || [ -n \"$line\" ]; do\n\
               [ -z \"$line\" ] && break\n\
             done\n\
             TOKEN=$(cat '{token}')\n\
             [ -n \"$TOKEN\" ] || exit 1\n\
             printf 'username=x-access-token\\npassword=%s\\n' \"$TOKEN\"\n",
            token = token_lit
        ),
    )
    .await?;
    chmod(&cred, 0o700).await?;

    if let Some(gh) = real_gh() {
        let wrapper = bin.join("gh");
        let gh_lit = shell_lit(&gh);
        tokio::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\n\
                 export GH_CONFIG_DIR='{gh_config}'\n\
                 export XDG_STATE_HOME='{state}'\n\
                 export XDG_CACHE_HOME='{cache}'\n\
                 export XDG_DATA_HOME='{data}'\n\
                 export GH_NO_UPDATE_NOTIFIER=1\n\
                 export GH_PROMPT_DISABLED=1\n\
                 TOKEN=$(cat '{token}')\n\
                 [ -n \"$TOKEN\" ] || exec '{gh}' \"$@\"\n\
                 export GH_TOKEN=\"$TOKEN\"\n\
                 export GITHUB_TOKEN=\"$TOKEN\"\n\
                 exec '{gh}' \"$@\"\n",
                token = token_lit,
                gh = gh_lit,
                gh_config = gh_config_lit,
                state = state_lit,
                cache = cache_lit,
                data = data_lit
            ),
        )
        .await?;
        chmod(&wrapper, 0o700).await?;
    }

    Ok(bin)
}

async fn configure_git(workspace: &Path, helper: &Path, public_url: &str) -> Result<()> {
    let helper_s = helper.display().to_string();
    let _ = Command::new("git")
        .current_dir(workspace)
        .args(["config", "--local", "--unset-all", "credential.helper"])
        .status()
        .await;
    let status = Command::new("git")
        .current_dir(workspace)
        .args(["config", "--local", "credential.helper", &helper_s])
        .status()
        .await
        .context("git config credential.helper")?;
    if !status.success() {
        anyhow::bail!("git config credential.helper failed");
    }

    if public_url.contains("github.com") {
        let set = Command::new("git")
            .current_dir(workspace)
            .args(["remote", "set-url", "origin", public_url])
            .status()
            .await
            .ok();
        if set.is_none_or(|s| !s.success()) {
            let _ = Command::new("git")
                .current_dir(workspace)
                .args(["remote", "add", "origin", public_url])
                .status()
                .await;
        }
    }
    Ok(())
}

/// Install run-private GitHub helpers and persist the current token.
pub async fn install(workspace: &Path, auth: &CloneAuthResponse) -> Result<PathBuf> {
    let dir = auth_dir(workspace);
    mkdir_private(&workspace.join(".zene")).await?;
    mkdir_private(&dir).await?;
    mkdir_private(&dir.join("gh")).await?;
    mkdir_private(&dir.join("xdg-state")).await?;
    mkdir_private(&dir.join("xdg-cache")).await?;
    mkdir_private(&dir.join("xdg-data")).await?;
    let config_yml = dir.join("gh").join("config.yml");
    if !config_yml.exists() {
        tokio::fs::write(&config_yml, "version: 1\n").await?;
        chmod(&config_yml, 0o600).await?;
    }
    if let Err(err) = ensure_git_exclude(workspace).await {
        warn!(error = %err, "failed to exclude .zene/ from git");
    }
    if let Err(err) = install_pre_push_hook(workspace).await {
        warn!(error = %err, "failed to install cloud pre-push hook");
    }
    if let Some(token) = auth.token.as_deref().filter(|t| !t.is_empty()) {
        write_token_file(&dir, token).await?;
    }
    let bin = write_helpers(&dir).await?;
    if !auth.mock {
        let helper = bin.join("git-credential-zene");
        let public = strip_userinfo(&auth.clone_url);
        if let Err(err) = configure_git(workspace, &helper, &public).await {
            warn!(error = %err, "failed to configure git github credentials");
        }
    }
    info!(
        run_id = %auth.run_id,
        path = %dir.display(),
        "installed run-private GitHub credentials"
    );
    Ok(bin)
}

pub fn inject_env(
    env: &mut std::collections::HashMap<String, String>,
    dir: &Path,
    token: Option<&str>,
    repo: Option<&str>,
) {
    let bin = bin_dir(dir);
    let gh_config = dir.join("gh");
    let old_path = env
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    env.insert("PATH".into(), format!("{}:{old_path}", bin.display()));
    env.insert("GH_CONFIG_DIR".into(), gh_config.display().to_string());
    env.insert("XDG_STATE_HOME".into(), dir.join("xdg-state").display().to_string());
    env.insert("XDG_CACHE_HOME".into(), dir.join("xdg-cache").display().to_string());
    env.insert("XDG_DATA_HOME".into(), dir.join("xdg-data").display().to_string());
    env.insert("GH_NO_UPDATE_NOTIFIER".into(), "1".into());
    env.insert("GH_PROMPT_DISABLED".into(), "1".into());
    // Override host `GH_TOKEN` / `~/.config/gh`. The wrapper re-reads the token
    // file on each `gh` invocation so a background refresh stays effective.
    if let Some(token) = token.filter(|t| !t.is_empty()) {
        env.insert("GH_TOKEN".into(), token.to_string());
        env.insert("GITHUB_TOKEN".into(), token.to_string());
    } else {
        env.insert("GH_TOKEN".into(), String::new());
        env.insert("GITHUB_TOKEN".into(), String::new());
    }
    if let Some(repo) = repo.filter(|r| !r.is_empty()) {
        env.insert("GH_REPO".into(), repo.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{auth_dir, github_repo_slug, strip_userinfo};
    use std::path::Path;

    #[test]
    fn strip_https_token() {
        assert_eq!(
            strip_userinfo("https://x-access-token:ghs_abc@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
    }

    #[test]
    fn strip_leaves_clean_url() {
        assert_eq!(
            strip_userinfo("https://github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
    }

    #[test]
    fn slug_from_https_and_ssh() {
        assert_eq!(
            github_repo_slug("https://x-access-token:ghs_abc@github.com/acme/app.git"),
            Some("acme/app".into())
        );
        assert_eq!(
            github_repo_slug("git@github.com:acme/app.git"),
            Some("acme/app".into())
        );
    }

    #[test]
    fn auth_dir_stays_inside_workspace() {
        let dir = auth_dir(Path::new("/tmp/ws"));
        assert_eq!(dir, Path::new("/tmp/ws/.zene/github"));
    }
}
