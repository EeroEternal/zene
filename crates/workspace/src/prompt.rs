use crate::skills::format_available_skills;
use crate::WorkspaceProvider;

/// Combine base system prompt with optional workspace context from a provider.
pub fn build_system_prompt(
    base: &str,
    provider: &dyn WorkspaceProvider,
    include_workspace_context: bool,
) -> String {
    if !include_workspace_context {
        return base.to_string();
    }

    let mut sections = vec![base.to_string()];

    if let Some(instructions) = provider.agent_instructions() {
        sections.push(format!("# Project instructions\n\n{instructions}"));
    }

    sections.push(provider.workspace_overview());

    if let Some(skills) = format_available_skills(&provider.discover_skills()) {
        sections.push(skills);
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::FsWorkspaceProvider;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn skips_workspace_when_disabled() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "hidden").unwrap();
        let provider = FsWorkspaceProvider::new(dir.path());
        assert_eq!(
            build_system_prompt("base only", &provider, false),
            "base only"
        );
    }

    #[test]
    fn includes_sections_when_enabled() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "Use Rust.").unwrap();
        let provider = FsWorkspaceProvider::new(dir.path());
        let prompt = build_system_prompt("Base prompt.", &provider, true);
        assert!(prompt.starts_with("Base prompt."));
        assert!(prompt.contains("Project instructions"));
        assert!(prompt.contains("Use Rust."));
        assert!(prompt.contains("# Workspace"));
    }
}
