use std::sync::Arc;

use tempfile::tempdir;
use zene_sandbox::LocalSandbox;
use zene_config::WebSearchConfig;
use zene_tools::{builtin_tools, shared_todo_store, ToolContext};

fn ctx(workdir: &std::path::Path) -> ToolContext {
    ToolContext {
        sandbox: Arc::new(LocalSandbox::new(workdir)),
        cancel: None,
        subagent: None,
        permission: None,
        plan_mode: None,
        todos: Some(shared_todo_store()),
        ask_user: None,
        background: None,
    }
}

async fn run_tool(name: &str, args: serde_json::Value, ctx: &ToolContext) -> zene_tools::ToolResult {
    builtin_tools(WebSearchConfig::default())
        .execute(name, &args.to_string(), ctx)
        .await
        .expect("tool execution should not fail")
}

#[tokio::test]
async fn todo_write_merges_by_id() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    let result = run_tool(
        "TodoWrite",
        serde_json::json!({
            "todos": [
                { "id": "1", "content": "First task", "status": "pending" },
                { "id": "2", "content": "Second task", "status": "in_progress" }
            ]
        }),
        &ctx,
    )
    .await;
    assert!(!result.is_error);
    assert!(result.content.contains("First task"));
    assert!(result.content.contains("Second task"));

    let result = run_tool(
        "TodoWrite",
        serde_json::json!({
            "todos": [
                { "id": "1", "content": "First task done", "status": "completed" }
            ]
        }),
        &ctx,
    )
    .await;
    assert!(!result.is_error);
    assert!(result.content.contains("First task done"));
    assert!(result.content.contains("Second task"));
}

#[tokio::test]
async fn todo_list_reads_without_mutation() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    run_tool(
        "TodoWrite",
        serde_json::json!({
            "todos": [{ "id": "a", "content": "Alpha", "status": "pending" }]
        }),
        &ctx,
    )
    .await;

    let result = run_tool("TodoList", serde_json::json!({}), &ctx).await;
    assert!(!result.is_error);
    assert!(result.content.contains("Alpha"));
}

#[tokio::test]
async fn ask_user_uses_custom_prompter() {
    let dir = tempdir().unwrap();
    let mut ctx = ctx(dir.path());
    ctx.ask_user = Some(Arc::new(|question, options| {
        assert_eq!(question, "Pick one");
        assert!(options.is_some());
        Ok("Option A".to_string())
    }));

    let result = run_tool(
        "AskUserQuestion",
        serde_json::json!({
            "question": "Pick one",
            "options": [
                { "label": "Option A", "description": "First" },
                { "label": "Option B" }
            ]
        }),
        &ctx,
    )
    .await;
    assert!(!result.is_error);
    assert_eq!(result.content, "Option A");
}

#[tokio::test]
async fn fetch_url_rejects_empty_url() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());
    let result = run_tool("FetchUrl", serde_json::json!({ "url": "  " }), &ctx).await;
    assert!(result.is_error);
}

#[test]
fn html_strip_unit() {
    // Covered by unit tests in fetch_url module; smoke check via builtin registration.
    let tools = zene_tools::default_builtin_tools();
    assert!(tools.definitions().iter().any(|d| d.name == "FetchUrl"));
    assert!(tools.definitions().iter().any(|d| d.name == "WebSearch"));
}
