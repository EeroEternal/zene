use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use zene_core::tool_scheduler::{classify_tool_accesses, ToolScheduler};
use zene_sandbox::LocalSandbox;
use zene_tools::{default_builtin_tools, ToolContext, ToolResult};

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
async fn parallel_read_calls_both_succeed() {
    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.txt"), "alpha").await.unwrap();
    tokio::fs::write(dir.path().join("b.txt"), "beta").await.unwrap();

    let tools = Arc::new(default_builtin_tools());
    let tool_ctx = ctx(dir.path());

    let mut scheduled = Vec::new();
    for path in ["a.txt", "b.txt"] {
        let accesses = classify_tool_accesses("Read", &format!(r#"{{"path":"{path}"}}"#));
        let ctx = ToolContext {
            sandbox: Arc::clone(&tool_ctx.sandbox),
            cancel: tool_ctx.cancel.clone(),
            subagent: tool_ctx.subagent.clone(),
            permission: tool_ctx.permission.clone(),
            plan_mode: tool_ctx.plan_mode.clone(),
            todos: tool_ctx.todos.clone(),
            ask_user: tool_ctx.ask_user.clone(),
            background: tool_ctx.background.clone(),
        };
        let tools = Arc::clone(&tools);
        let args = format!(r#"{{"path":"{path}"}}"#);
        let future: Pin<Box<dyn Future<Output = (ToolResult, Duration)> + Send>> =
            Box::pin(async move {
                let started = Instant::now();
                let result = tools
                    .execute("Read", &args, &ctx)
                    .await
                    .expect("read should execute");
                (result, started.elapsed())
            });
        scheduled.push((accesses, future));
    }

    let started = Instant::now();
    let results = ToolScheduler::run_ordered(scheduled).await;
    let elapsed = started.elapsed();

    assert_eq!(results.len(), 2);
    assert!(!results[0].0.is_error);
    assert!(!results[1].0.is_error);
    assert!(results[0].0.content.contains("alpha"));
    assert!(results[1].0.content.contains("beta"));
    assert!(
        elapsed < Duration::from_millis(500),
        "expected parallel reads, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn parallel_read_results_preserve_provider_order() {
    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("first.txt"), "one").await.unwrap();
    tokio::fs::write(dir.path().join("second.txt"), "two").await.unwrap();

    let tools = Arc::new(default_builtin_tools());
    let base_ctx = ctx(dir.path());

    let paths = ["first.txt", "second.txt"];
    let mut scheduled = Vec::new();
    for path in paths {
        let accesses = classify_tool_accesses("Read", &format!(r#"{{"path":"{path}"}}"#));
        let ctx = ToolContext {
            sandbox: Arc::clone(&base_ctx.sandbox),
            cancel: base_ctx.cancel.clone(),
            subagent: base_ctx.subagent.clone(),
            permission: base_ctx.permission.clone(),
            plan_mode: base_ctx.plan_mode.clone(),
            todos: base_ctx.todos.clone(),
            ask_user: base_ctx.ask_user.clone(),
            background: base_ctx.background.clone(),
        };
        let tools = Arc::clone(&tools);
        let args = format!(r#"{{"path":"{path}"}}"#);
        let path_label = path.to_string();
        let future: Pin<Box<dyn Future<Output = (String, ToolResult)> + Send>> =
            Box::pin(async move {
                let result: ToolResult = tools
                    .execute("Read", &args, &ctx)
                    .await
                    .expect("read should execute");
                (path_label, result)
            });
        scheduled.push((accesses, future));
    }

    let results = ToolScheduler::run_ordered(scheduled).await;
    assert_eq!(results[0].0, "first.txt");
    assert_eq!(results[1].0, "second.txt");
}
