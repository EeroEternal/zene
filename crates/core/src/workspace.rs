use std::fs;
use std::path::Path;
use std::process::Command;

use crate::skills::{discover_skills, format_available_skills};

const MAX_LISTING_ENTRIES: usize = 50;
const AGENT_INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

/// Discover AGENTS.md / CLAUDE.md in `workdir` and return concatenated content.
pub fn discover_agent_instructions(workdir: &Path) -> Option<String> {
    let mut sections = Vec::new();

    for filename in AGENT_INSTRUCTION_FILES {
        let path = workdir.join(filename);
        if !path.is_file() {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => {
                sections.push(format!("## {filename}\n\n{content}"));
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "failed to read agent instructions");
            }
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

/// Build workspace context for the system prompt: path, directory listing, git branch.
pub fn build_workspace_overview(workdir: &Path) -> String {
    let canonical = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());

    let mut parts = vec![format!("# Workspace\n\nWorking directory: `{}`", canonical.display())];

    if let Some(listing) = top_level_listing(&canonical) {
        parts.push(format!("## Top-level entries\n\n{listing}"));
    }

    if let Some(branch) = git_branch(&canonical) {
        parts.push(format!("## Git\n\nCurrent branch: `{branch}`"));
    }

    parts.join("\n\n")
}

fn top_level_listing(workdir: &Path) -> Option<String> {
    let mut entries: Vec<String> = fs::read_dir(workdir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != ".gitignore" {
                return None;
            }
            let suffix = if entry.file_type().ok()?.is_dir() {
                "/"
            } else {
                ""
            };
            Some(format!("{name}{suffix}"))
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    entries.sort();
    let total = entries.len();
    let truncated = total > MAX_LISTING_ENTRIES;
    entries.truncate(MAX_LISTING_ENTRIES);

    let mut listing = entries.join("\n");
    if truncated {
        listing.push_str(&format!(
            "\n\n... and {} more entries (listing capped at {MAX_LISTING_ENTRIES})",
            total - MAX_LISTING_ENTRIES
        ));
    }

    Some(listing)
}

fn git_branch(workdir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workdir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

/// Combine base system prompt with optional workspace context.
pub fn build_system_prompt(base: &str, workdir: &Path, include_workspace_context: bool) -> String {
    if !include_workspace_context {
        return base.to_string();
    }

    let mut sections = vec![base.to_string()];

    if let Some(instructions) = discover_agent_instructions(workdir) {
        sections.push(format!("# Project instructions\n\n{instructions}"));
    }

    let overview = build_workspace_overview(workdir);
    sections.push(overview);

    if let Some(skills) = format_available_skills(&discover_skills(workdir)) {
        sections.push(skills);
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn discovers_agents_md() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "Use Rust idioms.").unwrap();
        let content = discover_agent_instructions(dir.path()).unwrap();
        assert!(content.contains("AGENTS.md"));
        assert!(content.contains("Use Rust idioms."));
    }

    #[test]
    fn listing_skips_hidden_except_gitignore() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "target/").unwrap();
        fs::create_dir(dir.path().join(".hidden")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "").unwrap();

        let listing = top_level_listing(dir.path()).unwrap();
        assert!(listing.contains(".gitignore"));
        assert!(listing.contains("Cargo.toml"));
        assert!(!listing.contains(".hidden"));
    }
}
