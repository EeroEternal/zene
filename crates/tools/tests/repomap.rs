use std::sync::Arc;

use tempfile::tempdir;
use zene_sandbox::LocalSandbox;
use zene_config::WebSearchConfig;
use zene_tools::{builtin_tools, ToolContext};

fn ctx(workdir: &std::path::Path) -> ToolContext {
    ToolContext {
        sandbox: Arc::new(LocalSandbox::new(workdir)),
        cancel: None,
        subagent: None,
        permission: None,
        plan_mode: None,
        todos: None,
        ask_user: None,
        background: None,
    }
}

#[tokio::test]
async fn repo_map_returns_structure_not_source() {
    let dir = tempdir().unwrap();
    tokio::fs::write(
        dir.path().join("engine.rs"),
        "pub struct ContextEngine {}\nimpl ContextEngine {\n    pub fn prepare_step() {}\n}\n",
    )
    .await
    .unwrap();

    let ctx = ctx(dir.path());
    let result = builtin_tools(WebSearchConfig::default())
        .execute("RepoMap", r#"{"query":"ContextEngine"}"#, &ctx)
        .await
        .expect("RepoMap should run");

    assert!(!result.is_error);
    assert!(result.content.contains("engine.rs"));
    assert!(result.content.contains("ContextEngine"));
    assert!(result.content.contains("signatures only"));
    assert!(!result.content.contains("pub fn prepare_step() {}"));
}
