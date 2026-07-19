use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

/// Deny Write/Edit targets under sensitive paths.
pub fn check_write_allowed(path: &str) -> Result<(), String> {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");

    if is_sensitive_env_file(trimmed) {
        return Err(format!("writes are denied for sensitive env file: {path}"));
    }

    if is_under_git_internal(trimmed) {
        return Err(format!("writes are denied under .git/: {path}"));
    }

    Ok(())
}

fn is_sensitive_env_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name == ".env" {
        return true;
    }
    name.starts_with(".env.")
}

/// Blocks paths inside a `.git/` directory segment (not `.gitignore` at repo root).
fn is_under_git_internal(path: &str) -> bool {
    for segment in path.split('/') {
        if segment == ".git" {
            return true;
        }
    }
    false
}

pub fn canonical_workdir(workdir: &Path) -> Result<PathBuf> {
    workdir
        .canonicalize()
        .with_context(|| format!("invalid workdir: {}", workdir.display()))
}

fn candidate_path(workdir: &Path, path: &str) -> PathBuf {
    if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        workdir.join(path)
    }
}

fn ensure_within_workdir(workdir_canon: &Path, resolved: &Path) -> Result<()> {
    if !path_starts_with(resolved, workdir_canon) {
        anyhow::bail!("path escapes workspace: {}", resolved.display());
    }
    Ok(())
}

/// Compare paths after normalization so `/a/b` matches `/a/b/`.
fn path_starts_with(path: &Path, prefix: &Path) -> bool {
    let path = normalize_path(path);
    let prefix = normalize_path(prefix);
    path.starts_with(&prefix)
        && (path.as_os_str().len() == prefix.as_os_str().len()
            || path
                .as_os_str()
                .to_str()
                .is_some_and(|p| p.as_bytes().get(prefix.as_os_str().len()) == Some(&b'/')))
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Resolve a path that must already exist, rejecting workspace escapes via symlinks or `..`.
pub fn resolve_existing(workdir: &Path, path: &str) -> Result<PathBuf> {
    resolve_in_workspace(workdir, path, false)
}

/// Resolve a path that may not exist yet (for writes), still rejecting symlink escapes.
pub fn resolve_for_create(workdir: &Path, path: &str) -> Result<PathBuf> {
    resolve_in_workspace(workdir, path, true)
}

/// Re-check a previously resolved path before IO to catch symlink swaps (TOCTOU).
pub fn verify_resolved_path(workdir: &Path, resolved: &Path) -> Result<()> {
    let workdir_canon = canonical_workdir(workdir)?;
    let meta = resolved
        .symlink_metadata()
        .with_context(|| format!("path not found: {}", resolved.display()))?;

    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(resolved)
            .with_context(|| format!("read symlink: {}", resolved.display()))?;
        let target = if link.is_absolute() {
            link
        } else {
            resolved
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(link)
        };
        let canonical = target
            .canonicalize()
            .with_context(|| format!("broken symlink: {}", resolved.display()))?;
        ensure_within_workdir(&workdir_canon, &canonical)?;
        return Ok(());
    }

    let canonical = resolved
        .canonicalize()
        .with_context(|| format!("path not found: {}", resolved.display()))?;
    ensure_within_workdir(&workdir_canon, &canonical)
}

fn resolve_in_workspace(workdir: &Path, path: &str, allow_missing_leaf: bool) -> Result<PathBuf> {
    let workdir_canon = canonical_workdir(workdir)?;
    let candidate = candidate_path(workdir, path);

    if candidate.exists() || candidate.symlink_metadata().is_ok() {
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path not found: {path}"))?;
        ensure_within_workdir(&workdir_canon, &canonical)?;
        return Ok(canonical);
    }

    if !allow_missing_leaf {
        anyhow::bail!("path not found: {path}");
    }

    resolve_missing_leaf(&workdir_canon, workdir, &candidate, path)
}

fn resolve_missing_leaf(
    workdir_canon: &Path,
    workdir: &Path,
    candidate: &Path,
    original: &str,
) -> Result<PathBuf> {
    let rel = relative_path_within_workdir(workdir, candidate, original)?;
    let components: Vec<_> = Path::new(&rel).components().collect();
    if components.is_empty() {
        return Ok(workdir_canon.to_path_buf());
    }

    let mut current = workdir_canon.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Normal(name) => {
                current.push(name);
                let is_leaf = index + 1 == components.len();
                if is_leaf {
                    ensure_within_workdir(workdir_canon, &current)?;
                    return Ok(current);
                }
                if current.symlink_metadata().is_ok() {
                    current = resolve_existing_component(&current, original)?;
                    ensure_within_workdir(workdir_canon, &current)?;
                } else {
                    for remaining in &components[index + 1..] {
                        match remaining {
                            Component::Normal(part) => current.push(part),
                            Component::CurDir => {}
                            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                                anyhow::bail!("path escapes workspace: {original}");
                            }
                        }
                    }
                    ensure_within_workdir(workdir_canon, &current)?;
                    return Ok(current);
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!("path escapes workspace: {original}");
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("path escapes workspace: {original}");
            }
        }
    }

    ensure_within_workdir(workdir_canon, &current)?;
    Ok(current)
}

fn relative_path_within_workdir(
    workdir: &Path,
    candidate: &Path,
    original: &str,
) -> Result<String> {
    if candidate.is_absolute() {
        if let Ok(relative) = candidate.strip_prefix(workdir) {
            return Ok(relative.to_string_lossy().replace('\\', "/"));
        }
        let workdir_canon = canonical_workdir(workdir)?;
        if let Some(parent) = candidate.parent() {
            if let Ok(parent_canon) = parent.canonicalize() {
                if path_starts_with(&parent_canon, &workdir_canon) {
                    if let Some(file_name) = candidate.file_name() {
                        let mut rel_path = parent_canon
                            .strip_prefix(&workdir_canon)
                            .context("path escapes workspace")?
                            .to_path_buf();
                        rel_path.push(file_name);
                        return Ok(rel_path.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        anyhow::bail!("path escapes workspace: {original}");
    }

    Ok(candidate
        .strip_prefix(workdir)
        .with_context(|| format!("path escapes workspace: {original}"))?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn resolve_existing_component(path: &Path, original: &str) -> Result<PathBuf> {
    let meta = path
        .symlink_metadata()
        .with_context(|| format!("path not found: {original}"))?;

    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(path)
            .with_context(|| format!("read symlink: {}", path.display()))?;
        let target = if link.is_absolute() {
            link
        } else {
            path.parent().unwrap_or_else(|| Path::new(".")).join(link)
        };
        return target
            .canonicalize()
            .with_context(|| format!("broken symlink in path: {original}"));
    }

    path.canonicalize()
        .with_context(|| format!("path not found: {original}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn denies_git_internal() {
        assert!(check_write_allowed(".git/config").is_err());
        assert!(check_write_allowed("foo/.git/HEAD").is_err());
    }

    #[test]
    fn allows_gitignore_at_root() {
        assert!(check_write_allowed(".gitignore").is_ok());
        assert!(check_write_allowed("subdir/.gitignore").is_ok());
    }

    #[test]
    fn denies_env_files() {
        assert!(check_write_allowed(".env").is_err());
        assert!(check_write_allowed(".env.local").is_err());
        assert!(check_write_allowed("secrets/.env.production").is_err());
    }

    #[test]
    fn allows_normal_paths() {
        assert!(check_write_allowed("src/main.rs").is_ok());
        assert!(check_write_allowed("README.md").is_ok());
    }

    #[test]
    fn resolve_existing_rejects_symlink_escape() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
            let err = resolve_existing(dir.path(), "escape/secret.txt").unwrap_err();
            assert!(err.to_string().contains("escapes workspace"));
        }
    }

    #[test]
    fn resolve_for_create_rejects_symlink_prefix() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
            let err = resolve_for_create(dir.path(), "escape/new.txt").unwrap_err();
            assert!(err.to_string().contains("escapes workspace"));
        }
    }

    #[test]
    fn resolve_for_create_allows_new_file_in_workspace() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let resolved = resolve_for_create(dir.path(), "src/new.rs").unwrap();
        assert!(resolved.ends_with("src/new.rs"));
        assert!(path_starts_with(
            &resolved,
            &canonical_workdir(dir.path()).unwrap()
        ));
    }

    #[test]
    fn resolve_for_create_keeps_missing_intermediate_directories() {
        let dir = tempdir().unwrap();
        let resolved = resolve_for_create(dir.path(), "foo/bar/new.rs").unwrap();
        assert_eq!(
            resolved
                .strip_prefix(canonical_workdir(dir.path()).unwrap())
                .unwrap(),
            Path::new("foo/bar/new.rs")
        );
    }

    #[test]
    fn resolve_existing_allows_in_workspace_symlink() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("real")).unwrap();
        fs::write(dir.path().join("real/data.txt"), "ok").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
            let resolved = resolve_existing(dir.path(), "link/data.txt").unwrap();
            assert!(resolved.ends_with("real/data.txt"));
        }
    }

    #[test]
    fn resolve_rejects_parent_dir_escape() {
        let dir = tempdir().unwrap();
        let err = resolve_for_create(dir.path(), "../outside.txt").unwrap_err();
        assert!(err.to_string().contains("escapes workspace"));
    }
}
