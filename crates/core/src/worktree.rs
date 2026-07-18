//! Git worktree isolation for agent sessions (`--worktree`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Create (or reuse) a git worktree under `.zene/worktrees/<slug>` for this session.
pub fn ensure_session_worktree(repo_root: &Path, session_id: &str) -> Result<PathBuf> {
    if !repo_root.join(".git").exists() {
        bail!(
            "--worktree requires a git repository (no .git in {})",
            repo_root.display()
        );
    }

    let slug: String = session_id.chars().take(8).collect();
    let worktree_dir = repo_root.join(".zene").join("worktrees").join(&slug);
    if worktree_dir.exists() {
        return Ok(worktree_dir);
    }

    std::fs::create_dir_all(worktree_dir.parent().unwrap())
        .context("create .zene/worktrees directory")?;

    let branch = format!("zene/{slug}");
    let status = Command::new("git")
        .args([
            "worktree",
            "add",
            "-B",
            &branch,
            worktree_dir.to_str().unwrap_or_default(),
            "HEAD",
        ])
        .current_dir(repo_root)
        .status()
        .context("spawn git worktree add")?;

    if !status.success() {
        bail!("git worktree add failed with status {status}");
    }

    Ok(worktree_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn creates_worktree_in_git_repo() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        assert!(Command::new("git")
            .args(["init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "user.email", "zene@test"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "user.name", "zene"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        std::fs::write(root.join("README.md"), "hi").unwrap();
        assert!(Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());

        let wt = ensure_session_worktree(root, "abcdef12-xxxx").unwrap();
        assert!(wt.exists());
        assert!(wt.join("README.md").exists());
        // Reuse path on second call.
        let wt2 = ensure_session_worktree(root, "abcdef12-yyyy").unwrap();
        assert_eq!(wt, wt2);
    }
}
