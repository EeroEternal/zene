use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

pub struct SkillTool;

#[derive(Debug, Deserialize)]
struct SkillArgs {
    name: String,
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Skill".to_string(),
            description: "Load a skill by name. Returns the SKILL.md body so you can follow its instructions.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Skill name from the available skills list" }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: SkillArgs = serde_json::from_str(arguments).context("parse Skill args")?;
        match load_skill(&args.name, ctx.sandbox.workdir()) {
            Ok(content) => Ok(ToolResult {
                content,
                is_error: false,
            }),
            Err(err) => Ok(ToolResult {
                content: err.to_string(),
                is_error: true,
            }),
        }
    }
}

fn load_skill(name: &str, workdir: &Path) -> Result<String> {
    let skills_root = workdir.join(".agents/skills");
    if !skills_root.is_dir() {
        anyhow::bail!("no skills directory at .agents/skills");
    }

    for entry in fs::read_dir(&skills_root).context("read skills directory")? {
        let entry = entry.context("read skills entry")?;
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let content = fs::read_to_string(&skill_md)
            .with_context(|| format!("read skill file: {}", skill_md.display()))?;
        let Some(meta_name) = skill_name_from_content(&skill_md, &content) else {
            continue;
        };
        if meta_name == name {
            return Ok(skill_body(&content).to_string());
        }
    }

    anyhow::bail!("skill not found: {name}")
}

fn skill_name_from_content(path: &Path, content: &str) -> Option<String> {
    let frontmatter = parse_frontmatter(content)?;
    let fallback = path
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())?;

    frontmatter
        .get("name")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .or(Some(fallback))
}

fn skill_body(content: &str) -> &str {
    if let Some((_, body)) = split_frontmatter(content) {
        body.trim_start()
    } else {
        content.trim_start()
    }
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
