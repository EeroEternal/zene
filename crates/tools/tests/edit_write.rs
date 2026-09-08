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
async fn write_then_edit_happy_path() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    let write = run_tool(
        "Write",
        serde_json::json!({
            "path": "hello.txt",
            "content": "fn main() {\n    println!(\"hello\");\n}\n"
        }),
        &ctx,
    )
    .await;
    assert!(!write.is_error);

    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "hello.txt",
            "old_string": "println!(\"hello\")",
            "new_string": "println!(\"world\")"
        }),
        &ctx,
    )
    .await;
    assert!(!edit.is_error);
    assert!(edit.content.contains("Replaced 1 occurrence"));

    let content = ctx.sandbox.read_text("hello.txt").await.unwrap();
    assert!(content.contains("println!(\"world\")"));
    assert!(!content.contains("println!(\"hello\")"));
}

#[tokio::test]
async fn duplicate_old_string_without_replace_all_fails() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    run_tool(
        "Write",
        serde_json::json!({
            "path": "dup.txt",
            "content": "foo bar foo\n"
        }),
        &ctx,
    )
    .await;

    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "dup.txt",
            "old_string": "foo",
            "new_string": "baz"
        }),
        &ctx,
    )
    .await;
    assert!(edit.is_error);
    assert!(edit.content.contains("not unique"));
    assert!(edit.content.contains("2 occurrences"));

    let content = ctx.sandbox.read_text("dup.txt").await.unwrap();
    assert_eq!(content, "foo bar foo\n");
}

#[tokio::test]
async fn crlf_file_edit_preserves_line_endings() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("crlf.txt");
    tokio::fs::write(&file_path, "alpha\r\nbeta\r\n")
        .await
        .unwrap();

    let ctx = ctx(dir.path());
    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "crlf.txt",
            "old_string": "beta",
            "new_string": "gamma"
        }),
        &ctx,
    )
    .await;
    assert!(!edit.is_error);

    let raw = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(raw, "alpha\r\ngamma\r\n");
}

#[tokio::test]
async fn edit_rejects_empty_old_string() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    run_tool(
        "Write",
        serde_json::json!({
            "path": "x.txt",
            "content": "content\n"
        }),
        &ctx,
    )
    .await;

    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "x.txt",
            "old_string": "",
            "new_string": "new"
        }),
        &ctx,
    )
    .await;
    assert!(edit.is_error);
    assert!(edit.content.contains("must not be empty"));
}

#[tokio::test]
async fn edit_rejects_no_op() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    run_tool(
        "Write",
        serde_json::json!({
            "path": "x.txt",
            "content": "same\n"
        }),
        &ctx,
    )
    .await;

    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "x.txt",
            "old_string": "same",
            "new_string": "same"
        }),
        &ctx,
    )
    .await;
    assert!(edit.is_error);
    assert!(edit.content.contains("No changes to make"));
}

#[tokio::test]
async fn edit_recovers_via_whitespace_tolerant_matching() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    run_tool(
        "Write",
        serde_json::json!({
            "path": "indent.txt",
            "content": "fn test() {\n    let a = 1;\n    let b = 2;\n}\n"
        }),
        &ctx,
    )
    .await;

    // Model provides 2 spaces instead of 4 spaces
    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "indent.txt",
            "old_string": "  let a = 1;\n  let b = 2;",
            "new_string": "    let a = 10;\n    let b = 20;"
        }),
        &ctx,
    )
    .await;
    assert!(!edit.is_error);
    assert!(edit.content.contains("whitespace-tolerant alignment"));

    let content = ctx.sandbox.read_text("indent.txt").await.unwrap();
    assert_eq!(
        content,
        "fn test() {\n    let a = 10;\n    let b = 20;\n}\n"
    );
}

#[tokio::test]
async fn edit_whitespace_tolerant_rejects_ambiguous_matches() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());

    run_tool(
        "Write",
        serde_json::json!({
            "path": "ambiguous.txt",
            "content": "    let x = 1;\n    let y = 2;\n\n    let x = 1;\n    let y = 2;\n"
        }),
        &ctx,
    )
    .await;

    // Model provides 2 spaces (exact match count is 0), but it matches 2 normalized blocks
    let edit = run_tool(
        "Edit",
        serde_json::json!({
            "path": "ambiguous.txt",
            "old_string": "  let x = 1;\n  let y = 2;",
            "new_string": "  let x = 10;\n  let y = 20;"
        }),
        &ctx,
    )
    .await;
    assert!(edit.is_error);
    assert!(edit
        .content
        .contains("matched 2 locations with normalized whitespace"));
}
