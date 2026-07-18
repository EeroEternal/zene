use std::sync::Arc;

use tempfile::tempdir;
use zene_config::WebSearchConfig;
use zene_sandbox::LocalSandbox;
use zene_tools::{builtin_tools, Tool, ToolContext, WebSearchTool};

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
async fn web_search_rejects_empty_query() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());
    let result = builtin_tools(WebSearchConfig::default())
        .execute("WebSearch", r#"{"query":"  "}"#, &ctx)
        .await
        .expect("execute");
    assert!(result.is_error);
    assert!(result.content.contains("non-empty"));
}

#[tokio::test]
async fn web_search_schema_requires_query() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());
    let result = builtin_tools(WebSearchConfig::default())
        .execute("WebSearch", "{}", &ctx)
        .await
        .expect("execute");
    assert!(result.is_error);
    assert!(result.content.contains("validation failed"));
}

#[test]
fn web_search_registered_in_builtin_tools() {
    let tools = builtin_tools(WebSearchConfig::default());
    assert!(tools.definitions().iter().any(|d| d.name == "WebSearch"));
    let def = tools
        .definitions()
        .into_iter()
        .find(|d| d.name == "WebSearch")
        .expect("definition");
    assert!(def.parameters["properties"]["query"].is_object());
    assert!(def.parameters["properties"]["num_results"].is_object());
}

#[test]
fn web_search_tool_definition_matches_schema() {
    let tool = WebSearchTool::new(WebSearchConfig::default());
    let def = tool.definition();
    assert_eq!(def.name, "WebSearch");
    assert_eq!(def.parameters["required"], serde_json::json!(["query"]));
}
