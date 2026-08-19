use crate::skills::{skill_meta_from_file, SkillMeta};

/// Source for workspace sections injected into the system prompt.
pub trait WorkspaceProvider: Send + Sync {
    fn agent_instructions(&self) -> Option<String>;
    fn workspace_overview(&self) -> String;
    fn discover_skills(&self) -> Vec<SkillMeta>;
}

const MAX_LISTING_ENTRIES: usize = 50;
const AGENT_INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

/// Filesystem + local `git` workspace provider.
pub struct FsWorkspaceProvider {
    workdir: std::path::PathBuf,
}

impl FsWorkspaceProvider {
    pub fn new(workdir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
        }
    }

    pub fn workdir(&self) -> &std::path::Path {
        &self.workdir
    }
}

impl WorkspaceProvider for FsWorkspaceProvider {
    fn agent_instructions(&self) -> Option<String> {
        let mut sections = Vec::new();

        for filename in AGENT_INSTRUCTION_FILES {
            let path = self.workdir.join(filename);
            if !path.is_file() {
                continue;
            }
            match std::fs::read_to_string(&path) {
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

    fn workspace_overview(&self) -> String {
        let canonical = self
            .workdir
            .canonicalize()
            .unwrap_or_else(|_| self.workdir.clone());

        let mut parts = vec![format!(
            "# Workspace\n\nWorking directory: `{}`",
            canonical.display()
        )];

        if let Some(listing) = top_level_listing(&canonical) {
            parts.push(format!("## Top-level entries\n\n{listing}"));
        }

        if let Some(branch) = git_branch(&canonical) {
            parts.push(format!("## Git\n\nCurrent branch: `{branch}`"));
        }

        parts.join("\n\n")
    }

    fn discover_skills(&self) -> Vec<SkillMeta> {
        let skills_root = self.workdir.join(".agents/skills");
        if !skills_root.is_dir() {
            return Vec::new();
        }

        let mut skills = Vec::new();
        let entries = match std::fs::read_dir(&skills_root) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!(path = %skills_root.display(), %err, "failed to read skills directory");
                return Vec::new();
            }
        };

        for entry in entries.flatten() {
            let skill_md = entry.path().join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            match std::fs::read_to_string(&skill_md) {
                Ok(content) => {
                    if let Some(meta) = skill_meta_from_file(&skill_md, &content) {
                        skills.push(meta);
                    }
                }
                Err(err) => {
                    tracing::warn!(path = %skill_md.display(), %err, "failed to read skill file");
                }
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }
}

fn top_level_listing(workdir: &std::path::Path) -> Option<String> {
    let mut entries: Vec<String> = std::fs::read_dir(workdir)
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

fn git_branch(workdir: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn discovers_agents_md() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "Use Rust idioms.").unwrap();
        let provider = FsWorkspaceProvider::new(dir.path());
        let content = provider.agent_instructions().unwrap();
        assert!(content.contains("AGENTS.md"));
        assert!(content.contains("Use Rust idioms."));
    }

    #[test]
    fn listing_skips_hidden_except_gitignore() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "target/").unwrap();
        fs::create_dir(dir.path().join(".hidden")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "").unwrap();

        let provider = FsWorkspaceProvider::new(dir.path());
        let overview = provider.workspace_overview();
        assert!(overview.contains(".gitignore"));
        assert!(overview.contains("Cargo.toml"));
        assert!(!overview.contains(".hidden"));
    }

    #[test]
    fn discovers_skills_from_temp_dir() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo\ndescription: A demo skill\n---\n\n# Demo\n\nDo the thing.\n",
        )
        .unwrap();

        let provider = FsWorkspaceProvider::new(dir.path());
        let skills = provider.discover_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].description, "A demo skill");
    }
}
