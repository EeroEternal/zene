use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};
use zene_cloud_domain::CloneAuthResponse;

pub(crate) async fn workspace_ready(workspace: &Path) -> bool {
    if !workspace.join(".git").exists() {
        return false;
    }
    // Partial/interrupted clones leave a .git dir with no usable checkout.
    match run_git_output(workspace, &["rev-parse", "--verify", "HEAD"]).await {
        Ok(sha) => !sha.trim().is_empty(),
        Err(_) => false,
    }
}

pub(crate) async fn bare_cache_ready(cache: &Path) -> bool {
    // A bare repo has HEAD at the root (no nested .git).
    if !cache.join("HEAD").exists() {
        return false;
    }
    match run_git_output(cache, &["rev-parse", "--verify", "HEAD"]).await {
        Ok(sha) => !sha.trim().is_empty(),
        Err(_) => false,
    }
}

pub(crate) fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // HTTP/2 framing errors stall github.com clones on some developer networks.
    cmd.args(["-c", "http.version=HTTP/1.1"]);
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

fn authenticated_clone_url(auth: &CloneAuthResponse) -> String {
    let mut clone_url = auth.clone_url.clone();
    if let Some(token) = &auth.token {
        if let Some(rest) = clone_url.strip_prefix("https://") {
            let user = auth.username.as_deref().unwrap_or("x-access-token");
            clone_url = format!("https://{user}:{token}@{rest}");
        }
    }
    clone_url
}

async fn fetch_cache_base_ref(cache: &Path, auth: &CloneAuthResponse) -> Result<()> {
    let clone_url = authenticated_clone_url(auth);
    let status = git_command()
        .args([
            "-C",
            &cache.display().to_string(),
            "fetch",
            "--depth",
            "1",
            clone_url.as_str(),
            &format!("+refs/heads/{0}:refs/heads/{0}", auth.base_ref),
        ])
        .status()
        .await
        .context("git fetch base_ref")?;
    if !status.success() {
        bail!("git fetch base_ref failed with {status}");
    }
    Ok(())
}

async fn ensure_repo_cache(cache: &Path, auth: &CloneAuthResponse) -> Result<()> {
    if bare_cache_ready(cache).await {
        info!(
            path = %cache.display(),
            repository_id = %auth.repository_id,
            "repo cache ready; fetching latest base_ref"
        );
        let _ = fetch_cache_base_ref(cache, auth).await;
        return Ok(());
    }
    if cache.exists() {
        warn!(path = %cache.display(), "removing incomplete repo cache");
        let _ = tokio::fs::remove_dir_all(cache).await;
    }

    let clone_url = authenticated_clone_url(auth);
    std::fs::create_dir_all(cache.parent().unwrap_or_else(|| Path::new(".")))?;
    info!(
        url = %auth.clone_url,
        path = %cache.display(),
        repository_id = %auth.repository_id,
        "cloning repository into cache (shallow bare)"
    );
    let clone = git_command()
        .args([
            "clone",
            "--bare",
            "--depth",
            "1",
            "--single-branch",
            "--branch",
            &auth.base_ref,
            &clone_url,
            &cache.display().to_string(),
        ])
        .status();
    let status = tokio::time::timeout(Duration::from_secs(10 * 60), clone)
        .await
        .context("git clone --bare timed out")?
        .context("git clone --bare")?;
    if !status.success() {
        bail!("git bare clone failed with {status}");
    }
    Ok(())
}

pub(crate) async fn prepare_workspace(
    workspace_root: &Path,
    workspace: &Path,
    auth: &CloneAuthResponse,
) -> Result<()> {
    if workspace_ready(workspace).await {
        info!(path = %workspace.display(), "workspace already initialized");
        return Ok(());
    }
    if workspace.exists() {
        warn!(path = %workspace.display(), "removing incomplete workspace before clone");
        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    if auth.mock {
        std::fs::create_dir_all(workspace)?;
        info!(path = %workspace.display(), "preparing mock git workspace");
        run_git(workspace, &["init"]).await?;
        if run_git(workspace, &["checkout", "-b", &auth.head_branch])
            .await
            .is_err()
        {
            // git init may already be on main/master
            let _ = run_git(workspace, &["branch", "-M", &auth.head_branch]).await;
        }
        tokio::fs::write(
            workspace.join("README.md"),
            format!(
                "# {}/{}\n\nMock workspace for Zene Cloud Phase 0.\n\nBase ref: `{}`\n",
                "repo", "workspace", auth.base_ref
            ),
        )
        .await?;
        tokio::fs::create_dir_all(workspace.join("src")).await?;
        tokio::fs::write(
            workspace.join("src/main.rs"),
            "fn main() {\n    println!(\"hello cloud\");\n}\n",
        )
        .await?;
        run_git(workspace, &["add", "."]).await?;
        run_git(
            workspace,
            &[
                "-c",
                "user.email=zene-cloud@localhost",
                "-c",
                "user.name=Zene Cloud",
                "commit",
                "-m",
                "chore: initial mock workspace",
            ],
        )
        .await?;
        return Ok(());
    }

    let cache = workspace_root
        .join(".repo-cache")
        .join(auth.repository_id.to_string());
    ensure_repo_cache(&cache, auth).await?;

    std::fs::create_dir_all(workspace.parent().unwrap_or_else(|| Path::new(".")))?;
    info!(
        cache = %cache.display(),
        path = %workspace.display(),
        "attaching worktree from local repo cache"
    );

    // Try adding git worktree from the local bare cache
    let worktree_status = git_command()
        .args([
            "-C",
            &cache.display().to_string(),
            "worktree",
            "add",
            "--force",
            "--detach",
            &workspace.display().to_string(),
            auth.base_ref.as_str(),
        ])
        .status()
        .await;

    let worktree_ok = match worktree_status {
        Ok(s) if s.success() => true,
        other => {
            warn!(status = ?other, "git worktree add failed; falling back to git clone --local");
            false
        }
    };

    if !worktree_ok {
        if workspace.exists() {
            let _ = tokio::fs::remove_dir_all(workspace).await;
        }
        let status = git_command()
            .args([
                "clone",
                "--local",
                &cache.display().to_string(),
                &workspace.display().to_string(),
            ])
            .status()
            .await
            .context("git clone --local")?;
        if !status.success() {
            bail!("git local clone failed with {status}");
        }
    }

    let _ = run_git(
        workspace,
        &["checkout", "-B", &auth.head_branch, &auth.base_ref],
    )
    .await;
    Ok(())
}

async fn run_git(workspace: &Path, args: &[&str]) -> Result<()> {
    let output = git_command()
        .current_dir(workspace)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {stderr}", args.join(" "));
    }
    Ok(())
}

async fn run_git_output(workspace: &Path, args: &[&str]) -> Result<String> {
    let output = git_command()
        .current_dir(workspace)
        .args(args)
        .output()
        .await
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {stderr}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[allow(dead_code)]
async fn drain_stderr_lines(stderr: tokio::process::ChildStderr) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
}
