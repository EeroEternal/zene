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
    let slice = if truncated { &bytes[..max_bytes] } else { &bytes };
    let content = String::from_utf8_lossy(slice).into_owned();
    Ok(FileContent {
        path: rel.replace('\\', "/"),
        content,
        truncated,
    })
}

pub async fn git_diff(root: &Path) -> Result<String> {
    if !root.join(".git").exists() {
        return Ok(String::new());
    }
    let output = tokio::process::Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(root)
        .output()
        .await
        .context("git diff")?;
    let mut diff = String::from_utf8_lossy(&output.stdout).into_owned();
    if diff.trim().is_empty() {
        let staged = tokio::process::Command::new("git")
            .args(["diff", "--cached"])
            .current_dir(root)
            .output()
            .await
            .context("git diff --cached")?;
        diff = String::from_utf8_lossy(&staged.stdout).into_owned();
    }
    if diff.trim().is_empty() {
        let untracked = tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(root)
            .output()
            .await
            .context("git status")?;
        diff = String::from_utf8_lossy(&untracked.stdout).into_owned();
    }
    Ok(diff)
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let cleaned = rel.trim_start_matches('/');
    let path = PathBuf::from(cleaned);
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        bail!("invalid path");
    }
    let full = root.join(&path);
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canon = full.canonicalize().unwrap_or(full);
    if !canon.starts_with(&canon_root) {
        bail!("path escapes workspace");
    }
    Ok(canon)
}
