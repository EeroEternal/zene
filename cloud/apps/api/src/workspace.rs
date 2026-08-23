use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use walkdir::WalkDir;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub kind: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusFile {
    pub path: String,
    /// Single-letter status: M / A / D / R / C / U / ?
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub files: Vec<GitStatusFile>,
    pub total_additions: u64,
    pub total_deletions: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCompare {
    pub base: String,
    pub head: String,
    pub files: Vec<GitStatusFile>,
    pub total_additions: u64,
    pub total_deletions: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author: String,
    pub authored_at: String,
}

pub fn list_files(root: &Path, max_entries: usize) -> Result<Vec<FileEntry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .max_depth(6)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(name.as_ref(), ".git" | "node_modules" | "target" | ".zene")
        })
    {
        let entry = entry?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let meta = entry.metadata()?;
        out.push(FileEntry {
            path: rel,
            kind: if meta.is_dir() { "dir" } else { "file" }.into(),
            size: meta.len(),
        });
        if out.len() >= max_entries {
            break;
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn read_file(root: &Path, rel: &str, max_bytes: usize) -> Result<FileContent> {
    let path = safe_join(root, rel)?;
    if !path.is_file() {
        bail!("not a file");
    }
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let truncated = bytes.len() > max_bytes;
    let slice = if truncated {
        &bytes[..max_bytes]
    } else {
        &bytes
    };
    let content = String::from_utf8_lossy(slice).into_owned();
    Ok(FileContent {
        path: rel.replace('\\', "/"),
        content,
        truncated,
    })
}

pub async fn git_diff(root: &Path, path: Option<&str>) -> Result<String> {
    if !root.join(".git").exists() {
        return Ok(String::new());
    }
    let path = path.map(str::trim).filter(|p| !p.is_empty());
    if let Some(p) = path {
        // Reject path traversal before passing to git (file may be deleted).
        validate_rel_path(p)?;
    }

    let mut diff = run_git_diff(root, &["diff", "HEAD"], path).await?;
    if diff.trim().is_empty() {
        diff = run_git_diff(root, &["diff", "--cached"], path).await?;
    }
    if diff.trim().is_empty() && path.is_none() {
        let untracked = tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(root)
            .output()
            .await
            .context("git status")?;
        diff = String::from_utf8_lossy(&untracked.stdout).into_owned();
    }
    // Untracked single file: show as a synthetic "new file" diff when possible.
    if diff.trim().is_empty() {
        if let Some(p) = path {
            if let Ok(content) = read_file(root, p, 200_000) {
                let mut synthetic = format!("diff --git a/{p} b/{p}\n--- /dev/null\n+++ b/{p}\n");
                for line in content.content.lines() {
                    synthetic.push('+');
                    synthetic.push_str(line);
                    synthetic.push('\n');
                }
                if content.truncated {
                    synthetic.push_str("+… (truncated)\n");
                }
                return Ok(synthetic);
            }
        }
    }
    Ok(diff)
}

async fn run_git_diff(root: &Path, base_args: &[&str], path: Option<&str>) -> Result<String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(base_args).current_dir(root);
    if let Some(p) = path {
        cmd.args(["--", p]);
    }
    let output = cmd
        .output()
        .await
        .with_context(|| format!("git {}", base_args.join(" ")))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn git_status(root: &Path) -> Result<GitStatus> {
    if !root.join(".git").exists() {
        return Ok(GitStatus {
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
        });
    }

    let porcelain = tokio::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-uall"])
        .current_dir(root)
        .output()
        .await
        .context("git status --porcelain")?;
    let porcelain = String::from_utf8_lossy(&porcelain.stdout);

    let mut files: BTreeMap<String, GitStatusFile> = BTreeMap::new();
    for line in porcelain.lines() {
        if line.len() < 3 {
            continue;
        }
        let x = line.as_bytes()[0] as char;
        let y = line.as_bytes()[1] as char;
        let rest = &line[3..];
        // Rename: "R  old -> new"
        let path = if let Some((_, to)) = rest.split_once(" -> ") {
            to.trim().to_string()
        } else {
            rest.trim().to_string()
        };
        if path.is_empty() || path == ".zene" || path.starts_with(".zene/") {
            continue;
        }
        let status = normalize_status(x, y);
        files.insert(
            path.clone(),
            GitStatusFile {
                path,
                status,
                additions: 0,
                deletions: 0,
            },
        );
    }

    // numstat for tracked changes vs HEAD (includes staged + unstaged content when using HEAD).
    merge_numstat(root, &["diff", "HEAD", "--numstat"], &mut files).await?;
    merge_numstat(root, &["diff", "--cached", "--numstat"], &mut files).await?;

    // Untracked: count lines as additions.
    for f in files.values_mut() {
        if f.status == "?" && f.additions == 0 {
            if let Ok(content) = read_file(root, &f.path, 200_000) {
                let n = content.content.lines().count() as u64;
                f.additions = n;
            }
        }
    }

    let mut list: Vec<GitStatusFile> = files.into_values().collect();
    list.sort_by(|a, b| a.path.cmp(&b.path));
    let total_additions = list.iter().map(|f| f.additions).sum();
    let total_deletions = list.iter().map(|f| f.deletions).sum();
    Ok(GitStatus {
        files: list,
        total_additions,
        total_deletions,
    })
}

async fn merge_numstat(
    root: &Path,
    args: &[&str],
    files: &mut BTreeMap<String, GitStatusFile>,
) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .with_context(|| format!("git {}", args.join(" ")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let mut parts = line.split('\t');
        let add = parts.next().unwrap_or("-");
        let del = parts.next().unwrap_or("-");
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let additions = if add == "-" {
            0
        } else {
            add.parse().unwrap_or(0)
        };
        let deletions = if del == "-" {
            0
        } else {
            del.parse().unwrap_or(0)
        };
        let entry = files
            .entry(path.to_string())
            .or_insert_with(|| GitStatusFile {
                path: path.to_string(),
                status: "M".into(),
                additions: 0,
                deletions: 0,
            });
        // Prefer the larger counts if both staged and unstaged report the same file.
        entry.additions = entry.additions.max(additions);
        entry.deletions = entry.deletions.max(deletions);
    }
    Ok(())
}

/// Resolve a usable base OID for compare/log. Prefers `origin/{base_ref}` (single-branch clone).
pub async fn resolve_base_oid(root: &Path, base_ref: &str) -> Result<String> {
    let base_ref = base_ref.trim();
    if base_ref.is_empty() {
        bail!("base ref is empty");
    }
    if !root.join(".git").exists() {
        bail!("not a git repository");
    }

    let candidates = [
        format!("origin/{base_ref}"),
        base_ref.to_string(),
        format!("refs/remotes/origin/{base_ref}"),
    ];
    for rev in &candidates {
        if let Some(oid) = rev_parse(root, rev).await? {
            return Ok(oid);
        }
    }

    let fetch = tokio::process::Command::new("git")
        .args(["fetch", "--depth=1", "origin", base_ref])
        .current_dir(root)
        .output()
        .await
        .context("git fetch base ref")?;
    if !fetch.status.success() {
        let err = String::from_utf8_lossy(&fetch.stderr);
        bail!("could not resolve base ref `{base_ref}`: {}", err.trim());
    }

    for rev in &candidates {
        if let Some(oid) = rev_parse(root, rev).await? {
            return Ok(oid);
        }
    }
    bail!("could not resolve base ref `{base_ref}` after fetch");
}

async fn rev_parse(root: &Path, rev: &str) -> Result<Option<String>> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .current_dir(root)
        .output()
        .await
        .with_context(|| format!("git rev-parse {rev}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if oid.is_empty() {
        return Ok(None);
    }
    Ok(Some(oid))
}

async fn merge_base(root: &Path, base_oid: &str) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .args(["merge-base", base_oid, "HEAD"])
        .current_dir(root)
        .output()
        .await
        .context("git merge-base")?;
    if !output.status.success() {
        // Divergent or shallow history: fall back to base OID itself.
        return Ok(base_oid.to_string());
    }
    let mb = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if mb.is_empty() {
        return Ok(base_oid.to_string());
    }
    Ok(mb)
}

/// Branch + WIP file list: working tree (and index) vs merge-base(base, HEAD), plus untracked.
pub async fn git_compare(root: &Path, base_ref: &str) -> Result<GitCompare> {
    if !root.join(".git").exists() {
        return Ok(GitCompare {
            base: base_ref.to_string(),
            head: "HEAD".into(),
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
        });
    }

    let base_oid = resolve_base_oid(root, base_ref).await?;
    let left = merge_base(root, &base_oid).await?;

    let mut files: BTreeMap<String, GitStatusFile> = BTreeMap::new();
    merge_name_status(root, &["diff", "--name-status", "-M", &left], &mut files).await?;
    merge_numstat(root, &["diff", "--numstat", &left], &mut files).await?;

    // Untracked / WIP status overlay (and catch files only in index/worktree that name-status missed).
    let porcelain = tokio::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-uall"])
        .current_dir(root)
        .output()
        .await
        .context("git status --porcelain")?;
    let porcelain = String::from_utf8_lossy(&porcelain.stdout);
    for line in porcelain.lines() {
        if line.len() < 3 {
            continue;
        }
        let x = line.as_bytes()[0] as char;
        let y = line.as_bytes()[1] as char;
        let rest = &line[3..];
        let path = if let Some((_, to)) = rest.split_once(" -> ") {
            to.trim().to_string()
        } else {
            rest.trim().to_string()
        };
        if path.is_empty() {
            continue;
        }
        let status = normalize_status(x, y);
        let entry = files.entry(path.clone()).or_insert_with(|| GitStatusFile {
            path: path.clone(),
            status: status.clone(),
            additions: 0,
            deletions: 0,
        });
        // WIP status takes priority over committed name-status letter.
        entry.status = status;
    }

    for f in files.values_mut() {
        if f.status == "?" && f.additions == 0 {
            if let Ok(content) = read_file(root, &f.path, 200_000) {
                f.additions = content.content.lines().count() as u64;
            }
        }
    }

    let mut list: Vec<GitStatusFile> = files.into_values().collect();
    list.sort_by(|a, b| a.path.cmp(&b.path));
    let total_additions = list.iter().map(|f| f.additions).sum();
    let total_deletions = list.iter().map(|f| f.deletions).sum();
    Ok(GitCompare {
        base: base_ref.to_string(),
        head: "HEAD".into(),
        files: list,
        total_additions,
        total_deletions,
    })
}

/// Per-file diff of working tree vs merge-base(base, HEAD).
pub async fn git_compare_diff(root: &Path, base_ref: &str, path: &str) -> Result<String> {
    if !root.join(".git").exists() {
        return Ok(String::new());
    }
    let path = path.trim();
    validate_rel_path(path)?;

    let base_oid = resolve_base_oid(root, base_ref).await?;
    let left = merge_base(root, &base_oid).await?;
    let mut diff = run_git_diff(root, &["diff", &left], Some(path)).await?;
    if diff.trim().is_empty() {
        diff = run_git_diff(root, &["diff", "--cached", &left], Some(path)).await?;
    }
    if diff.trim().is_empty() {
        if let Ok(content) = read_file(root, path, 200_000) {
            let mut synthetic =
                format!("diff --git a/{path} b/{path}\n--- /dev/null\n+++ b/{path}\n");
            for line in content.content.lines() {
                synthetic.push('+');
                synthetic.push_str(line);
                synthetic.push('\n');
            }
            if content.truncated {
                synthetic.push_str("+… (truncated)\n");
            }
            return Ok(synthetic);
        }
    }
    Ok(diff)
}

pub async fn git_commits(root: &Path, base_ref: &str, limit: usize) -> Result<Vec<GitCommit>> {
    if !root.join(".git").exists() {
        return Ok(Vec::new());
    }
    let base_oid = resolve_base_oid(root, base_ref).await?;
    let limit = limit.clamp(1, 200);
    let range = format!("{base_oid}..HEAD");
    let output = tokio::process::Command::new("git")
        .args([
            "log",
            &format!("--max-count={limit}"),
            "--format=%H%x09%h%x09%s%x09%an%x09%aI",
            &range,
        ])
        .current_dir(root)
        .output()
        .await
        .context("git log")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("git log failed: {}", err.trim());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(5, '\t');
        let sha = parts.next().unwrap_or("").to_string();
        let short_sha = parts.next().unwrap_or("").to_string();
        let subject = parts.next().unwrap_or("").to_string();
        let author = parts.next().unwrap_or("").to_string();
        let authored_at = parts.next().unwrap_or("").to_string();
        if sha.is_empty() {
            continue;
        }
        commits.push(GitCommit {
            sha,
            short_sha,
            subject,
            author,
            authored_at,
        });
    }
    Ok(commits)
}

async fn merge_name_status(
    root: &Path,
    args: &[&str],
    files: &mut BTreeMap<String, GitStatusFile>,
) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .with_context(|| format!("git {}", args.join(" ")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let mut parts = line.split('\t');
        let status_raw = parts.next().unwrap_or("").trim();
        if status_raw.is_empty() {
            continue;
        }
        let status = status_raw.chars().next().unwrap_or('M').to_string();
        let path = if status == "R" || status == "C" {
            // R100\told\tnew  or C...\told\tnew
            let _old = parts.next();
            parts.next().unwrap_or("").trim().to_string()
        } else {
            parts.next().unwrap_or("").trim().to_string()
        };
        if path.is_empty() {
            continue;
        }
        files.entry(path.clone()).or_insert_with(|| GitStatusFile {
            path,
            status,
            additions: 0,
            deletions: 0,
        });
    }
    Ok(())
}

fn normalize_status(x: char, y: char) -> String {
    if x == '?' || y == '?' {
        return "?".into();
    }
    if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
        return "U".into();
    }
    // Prefer worktree status, then index.
    let c = if y != ' ' { y } else { x };
    match c {
        'M' | 'A' | 'D' | 'R' | 'C' | 'T' => c.to_string(),
        _ if x != ' ' => x.to_string(),
        _ => "M".into(),
    }
}

fn validate_rel_path(rel: &str) -> Result<()> {
    let cleaned = rel.trim_start_matches('/');
    let path = PathBuf::from(cleaned);
    if cleaned.is_empty()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("invalid path");
    }
    Ok(())
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    validate_rel_path(rel)?;
    let cleaned = rel.trim_start_matches('/');
    let path = PathBuf::from(cleaned);
    let full = root.join(&path);
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canon = full.canonicalize().unwrap_or(full);
    if !canon.starts_with(&canon_root) {
        bail!("path escapes workspace");
    }
    Ok(canon)
}

/// Stage and commit all working-tree changes when the repo is dirty.
/// Returns the new HEAD sha, or the existing HEAD when nothing to commit.
pub async fn commit_worktree_if_dirty(root: &Path, message: &str) -> Result<Option<String>> {
    if !root.join(".git").exists() {
        return Ok(None);
    }
    let status = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .await
        .context("git status --porcelain")?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        bail!("git status failed: {}", stderr.trim());
    }
    let porcelain = String::from_utf8_lossy(&status.stdout);
    if porcelain.trim().is_empty() {
        return rev_parse(root, "HEAD").await;
    }

    let add = tokio::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .await
        .context("git add -A")?;
    if !add.status.success() {
        let stderr = String::from_utf8_lossy(&add.stderr);
        bail!("git add failed: {}", stderr.trim());
    }

    let msg = message.trim();
    let msg = if msg.is_empty() { "zene: changes" } else { msg };
    let commit = tokio::process::Command::new("git")
        .args([
            "-c",
            "user.email=zene-cloud@localhost",
            "-c",
            "user.name=Zene Cloud",
            "commit",
            "-m",
            msg,
        ])
        .current_dir(root)
        .output()
        .await
        .context("git commit")?;
    if !commit.status.success() {
        let stderr = String::from_utf8_lossy(&commit.stderr);
        bail!("git commit failed: {}", stderr.trim());
    }
    rev_parse(root, "HEAD").await
}

/// True when HEAD has at least one commit not reachable from `base_ref`.
pub async fn branch_has_commits_ahead(root: &Path, base_ref: &str) -> Result<bool> {
    if !root.join(".git").exists() {
        return Ok(false);
    }
    let base_oid = resolve_base_oid(root, base_ref).await?;
    let output = tokio::process::Command::new("git")
        .args(["rev-list", "--count", &format!("{base_oid}..HEAD")])
        .current_dir(root)
        .output()
        .await
        .context("git rev-list --count")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git rev-list failed: {}", stderr.trim());
    }
    let count = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    Ok(count > 0)
}
