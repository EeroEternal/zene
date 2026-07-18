use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use zene_config::WebSearchConfig;
use zene_sandbox::LocalSandbox;
use zene_tools::{builtin_tools, shared_background_tasks, ToolContext};

fn ctx(workdir: &std::path::Path) -> ToolContext {
    ToolContext {
        sandbox: Arc::new(LocalSandbox::new(workdir)),
        cancel: None,
        subagent: None,
        permission: None,
        plan_mode: None,
        todos: None,
        ask_user: None,
        background: Some(shared_background_tasks()),
    }
}

#[tokio::test]
async fn background_bash_completes_and_is_readable() {
    let dir = tempdir().unwrap();
    let ctx = ctx(dir.path());
    let tools = builtin_tools(WebSearchConfig::default());

    let started = tools
        .execute(
            "Bash",
            r#"{"command":"echo hello-bg","run_in_background":true}"#,
            &ctx,
        )
        .await
        .unwrap();
    assert!(!started.is_error);
    assert!(started.content.contains("bash-"));

    let task_id = started
        .content
        .split('`')
        .nth(1)
        .expect("task id in backticks")
        .to_string();

    // Poll until complete.
    let mut output = String::new();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let result = tools
            .execute(
                "TaskOutput",
                &serde_json::json!({ "task_id": task_id, "action": "get" }).to_string(),
                &ctx,
            )
            .await
            .unwrap();
        output = result.content.clone();
        if output.contains("status: completed") {
            break;
        }
    }
    assert!(
        output.contains("hello-bg"),
        "expected command output, got: {output}"
    );
}
