use std::sync::Arc;

use tempfile::tempdir;
use zene_config::WebSearchConfig;
use zene_sandbox::LocalSandbox;
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

async fn run_tool(
    name: &str,
    args: serde_json::Value,
    ctx: &ToolContext,
) -> zene_tools::ToolResult {
    builtin_tools(WebSearchConfig::default())
        .execute(name, &args.to_string(), ctx)
        .await
        .expect("tool execution should not fail")
}

#[tokio::test]
async fn read_directory_lists_entries() {
    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.txt"), "x")
        .await
        .unwrap();
    tokio::fs::create_dir(dir.path().join("sub")).await.unwrap();

    let ctx = ctx(dir.path());
    let result = run_tool("Read", serde_json::json!({ "path": "." }), &ctx).await;
    assert!(!result.is_error);
    assert!(result.content.contains("a.txt"));
    assert!(result.content.contains("sub"));
}

#[tokio::test]
async fn read_crlf_file_uses_lf_view_and_numbered_lines() {
    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("crlf.txt"), "line1\r\nline2\r\n")
        .await
        .unwrap();

    let ctx = ctx(dir.path());
    let result = run_tool("Read", serde_json::json!({ "path": "crlf.txt" }), &ctx).await;
    assert!(!result.is_error);
    assert!(result.content.contains("L1:line1"));
    assert!(result.content.contains("L2:line2"));
    assert!(!result.content.contains('\r'));
}

#[tokio::test]
async fn read_binary_file_returns_error() {
    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("bin.dat"), &[0u8, 1, 2, 0, 4])
        .await
        .unwrap();

    let ctx = ctx(dir.path());
    let result = run_tool("Read", serde_json::json!({ "path": "bin.dat" }), &ctx).await;
    assert!(result.is_error);
    assert!(result.content.contains("binary"));
}

#[tokio::test]
async fn read_missing_file_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());
    let result = run_tool("Read", serde_json::json!({ "path": "nope.txt" }), &ctx).await;
    assert!(result.is_error);
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn glob_matches_nested_rust_files() {
    let dir = tempdir().unwrap();
    tokio::fs::create_dir_all(dir.path().join("src/lib"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("src/lib/mod.rs"), "")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("readme.md"), "")
        .await
        .unwrap();

    let ctx = ctx(dir.path());
    let result = run_tool("Glob", serde_json::json!({ "pattern": "**/*.rs" }), &ctx).await;
    assert!(!result.is_error);
    assert!(result.content.contains("src/lib/mod.rs"));
    assert!(!result.content.contains("readme.md"));
}
