use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// Discover `.agents/skills/*/SKILL.md` under `workdir`.
/// Parses YAML-style frontmatter for `name` and `description` only.
pub fn discover_skills(workdir: &Path) -> Vec<SkillMeta> {
    let skills_root = workdir.join(".agents/skills");
    if !skills_root.is_dir() {
        return Vec::new();
    }

    let mut skills = Vec::new();
    let entries = match fs::read_dir(&skills_root) {
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
        match fs::read_to_string(&skill_md) {
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

pub fn format_available_skills(skills: &[SkillMeta]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }

    let lines: Vec<String> = skills
        .iter()
        .map(|skill| format!("- {}: {}", skill.name, skill.description))
        .collect();
    Some(format!("Available skills:\n{}", lines.join("\n")))
}

fn skill_meta_from_file(path: &Path, content: &str) -> Option<SkillMeta> {
    let frontmatter = parse_frontmatter(content)?;
    let fallback_name = path
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());

    let name = frontmatter
        .get("name")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_name);

    let description = frontmatter
        .get("description")
        .cloned()
        .unwrap_or_default();

    Some(SkillMeta {
        name,
        description,
        path: path.to_path_buf(),
    })
}

fn parse_frontmatter(content: &str) -> Option<std::collections::HashMap<String, String>> {
    let (frontmatter, _) = split_frontmatter(content)?;
    let mut map = std::collections::HashMap::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        map.insert(key.trim().to_lowercase(), value.trim().to_string());
    }
    Some(map)
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let trimmed = content.strip_prefix("---")?;
    let rest = trimmed.strip_prefix('\n').or(Some(trimmed))?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let body = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    Some((frontmatter, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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

        let skills = discover_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].description, "A demo skill");
    }

    #[test]
    fn format_available_skills_lists_entries() {
        let formatted = format_available_skills(&[SkillMeta {
            name: "demo".to_string(),
            description: "Demo skill".to_string(),
            path: PathBuf::from(".agents/skills/demo/SKILL.md"),
        }])
        .unwrap();
        assert!(formatted.contains("Available skills:"));
        assert!(formatted.contains("- demo: Demo skill"));
    }
}
