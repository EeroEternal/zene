use std::future::Future;
use std::pin::Pin;

use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOperation {
    Read,
    Write,
    ReadWrite,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResourceAccess {
    File {
        operation: FileOperation,
        path: Option<String>,
        recursive: bool,
    },
    Shell,
    All,
}

pub type ToolAccesses = Vec<ToolResourceAccess>;

pub fn classify_tool_accesses(name: &str, arguments: &str) -> ToolAccesses {
    let args = serde_json::from_str::<Value>(arguments).ok();
    match name {
        "Read" => {
            let path = args.and_then(|v| str_field(&v, "path"));
            let recursive = path
                .as_ref()
                .is_some_and(|p| p.ends_with('/') || p.ends_with('\\'));
            vec![ToolResourceAccess::File {
                operation: FileOperation::Read,
                path,
                recursive,
            }]
        }
        "Grep" => {
            let path = args.and_then(|v| str_field(&v, "path"));
            let recursive = path
                .as_ref()
                .is_none_or(|p| p.ends_with('/') || p.ends_with('\\'));
            vec![ToolResourceAccess::File {
                operation: FileOperation::Search,
                path: path.clone(),
                recursive,
            }]
        }
        "Glob" => vec![ToolResourceAccess::File {
            operation: FileOperation::Search,
            path: None,
            recursive: true,
        }],
        "RepoMap" => {
            let path = args.and_then(|v| str_field(&v, "path"));
            vec![ToolResourceAccess::File {
                operation: FileOperation::Search,
                path,
                recursive: true,
            }]
        }
        "Skill" => vec![ToolResourceAccess::File {
            operation: FileOperation::Read,
            path: None,
            recursive: false,
        }],
        "Write" => vec![ToolResourceAccess::File {
            operation: FileOperation::Write,
            path: args.and_then(|v| str_field(&v, "path")),
            recursive: false,
        }],
        "Edit" => vec![ToolResourceAccess::File {
            operation: FileOperation::ReadWrite,
            path: args.and_then(|v| str_field(&v, "path")),
            recursive: false,
        }],
        "Bash" => vec![ToolResourceAccess::Shell],
        _ => vec![ToolResourceAccess::All],
    }
}

pub fn accesses_conflict(left: &ToolAccesses, right: &ToolAccesses) -> bool {
    left.iter()
        .any(|l| right.iter().any(|r| resource_access_conflict(l, r)))
}

fn resource_access_conflict(left: &ToolResourceAccess, right: &ToolResourceAccess) -> bool {
    match (left, right) {
        (ToolResourceAccess::All, _) | (_, ToolResourceAccess::All) => true,
        (ToolResourceAccess::Shell, ToolResourceAccess::Shell) => true,
        (ToolResourceAccess::Shell, ToolResourceAccess::File { operation, .. })
        | (ToolResourceAccess::File { operation, .. }, ToolResourceAccess::Shell) => {
            file_operation_writes(*operation)
        }
        (
            ToolResourceAccess::File {
                operation: left_op,
                path: left_path,
                recursive: left_recursive,
            },
            ToolResourceAccess::File {
                operation: right_op,
                path: right_path,
                recursive: right_recursive,
            },
        ) => {
            if !file_operations_conflict(*left_op, *right_op) {
                return false;
            }
            file_accesses_overlap(left_path, *left_recursive, right_path, *right_recursive)
        }
    }
}

fn file_operations_conflict(left: FileOperation, right: FileOperation) -> bool {
    file_operation_writes(left) || file_operation_writes(right)
}

fn file_operation_writes(operation: FileOperation) -> bool {
    matches!(operation, FileOperation::Write | FileOperation::ReadWrite)
}

fn file_accesses_overlap(
    left_path: &Option<String>,
    left_recursive: bool,
    right_path: &Option<String>,
    right_recursive: bool,
) -> bool {
    match (left_path, right_path) {
        (None, _) | (_, None) => true,
        (Some(left), Some(right)) => {
            let left_path = normalize_path(left);
            let right_path = normalize_path(right);
            if left_path == right_path {
                return true;
            }
            let left_prefix = if left_path.ends_with('/') {
                left_path.clone()
            } else {
                format!("{left_path}/")
            };
            let right_prefix = if right_path.ends_with('/') {
                right_path.clone()
            } else {
                format!("{right_path}/")
            };
            (left_recursive && right_path.starts_with(&left_prefix))
                || (right_recursive && left_path.starts_with(&right_prefix))
        }
    }
}

fn normalize_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    let folded = normalized.to_lowercase();
    if folded.len() > 1 && folded.ends_with('/') {
        folded.trim_end_matches('/').to_string()
    } else {
        folded
    }
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type TaskItem<T> = (ToolAccesses, BoxFuture<T>);
type InFlightTasks<T> = FuturesUnordered<BoxFuture<(usize, T)>>;

struct ScheduledTask<T> {
    index: usize,
    accesses: ToolAccesses,
    future: BoxFuture<T>,
}

struct ActiveTask {
    index: usize,
    accesses: ToolAccesses,
}

pub struct ToolScheduler;

impl ToolScheduler {
    /// Run tool futures with conflict-aware parallelism. Results are returned in input order.
    pub async fn run_ordered<T>(tasks: Vec<TaskItem<T>>) -> Vec<T>
    where
        T: Send + 'static,
    {
        let count = tasks.len();
        if count == 0 {
            return Vec::new();
        }

        let mut queued: Vec<ScheduledTask<T>> = tasks
            .into_iter()
            .enumerate()
            .map(|(index, (accesses, future))| ScheduledTask {
                index,
                accesses,
                future,
            })
            .collect();
        let mut active: Vec<ActiveTask> = Vec::new();
        let mut in_flight: InFlightTasks<T> = FuturesUnordered::new();
        let mut results: Vec<Option<T>> = (0..count).map(|_| None).collect();
        let mut completed = 0;

        loop {
            start_queued_tasks(&mut queued, &mut active, &mut in_flight);

            if completed >= count {
                break;
            }

            if in_flight.is_empty() {
                tokio::task::yield_now().await;
                continue;
            }

            let (index, result) = in_flight
                .next()
                .await
                .expect("in-flight tool future ended unexpectedly");
            active.retain(|task| task.index != index);
            results[index] = Some(result);
            completed += 1;
        }

        results
            .into_iter()
            .map(|slot| slot.expect("missing scheduled tool result"))
            .collect()
    }
}

fn is_blocked<T>(
    accesses: &ToolAccesses,
    active: &[ActiveTask],
    queued: &[ScheduledTask<T>],
) -> bool {
    active
        .iter()
        .any(|task| accesses_conflict(accesses, &task.accesses))
        || queued
            .iter()
            .any(|task| accesses_conflict(accesses, &task.accesses))
}

fn start_queued_tasks<T>(
    queued: &mut Vec<ScheduledTask<T>>,
    active: &mut Vec<ActiveTask>,
    in_flight: &mut InFlightTasks<T>,
) where
    T: Send + 'static,
{
    let mut still_queued = Vec::new();
    for task in queued.drain(..) {
        if is_blocked(&task.accesses, active, &still_queued) {
            still_queued.push(task);
        } else {
            let ScheduledTask {
                index,
                accesses,
                future,
            } = task;
            active.push(ActiveTask { index, accesses });
            in_flight.push(Box::pin(async move { (index, future.await) }));
        }
    }
    *queued = still_queued;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn read_same_file_conflicts_with_write() {
        let read = classify_tool_accesses("Read", r#"{"path":"src/lib.rs"}"#);
        let write = classify_tool_accesses("Write", r#"{"path":"src/lib.rs"}"#);
        assert!(accesses_conflict(&read, &write));
    }

    #[test]
    fn read_different_files_do_not_conflict() {
        let a = classify_tool_accesses("Read", r#"{"path":"a.rs"}"#);
        let b = classify_tool_accesses("Read", r#"{"path":"b.rs"}"#);
        assert!(!accesses_conflict(&a, &b));
    }

    #[test]
    fn write_same_file_conflicts() {
        let a = classify_tool_accesses("Write", r#"{"path":"x.rs"}"#);
        let b = classify_tool_accesses(
            "Edit",
            r#"{"path":"x.rs","old_string":"a","new_string":"b"}"#,
        );
        assert!(accesses_conflict(&a, &b));
    }

    #[test]
    fn bash_conflicts_with_bash() {
        let a = classify_tool_accesses("Bash", r#"{"command":"ls"}"#);
        let b = classify_tool_accesses("Bash", r#"{"command":"pwd"}"#);
        assert!(accesses_conflict(&a, &b));
    }

    #[test]
    fn bash_does_not_conflict_with_unrelated_read() {
        let bash = classify_tool_accesses("Bash", r#"{"command":"ls"}"#);
        let read = classify_tool_accesses("Read", r#"{"path":"README.md"}"#);
        assert!(!accesses_conflict(&bash, &read));
    }

    #[test]
    fn bash_conflicts_with_write() {
        let bash = classify_tool_accesses("Bash", r#"{"command":"touch x"}"#);
        let write = classify_tool_accesses("Write", r#"{"path":"x.rs"}"#);
        assert!(accesses_conflict(&bash, &write));
    }

    #[test]
    fn recursive_grep_conflicts_with_write_in_subtree() {
        let grep = classify_tool_accesses("Grep", r#"{"pattern":"foo","path":"src/"}"#);
        let write = classify_tool_accesses("Write", r#"{"path":"src/deep/x.rs"}"#);
        assert!(accesses_conflict(&grep, &write));
    }

    #[test]
    fn unknown_tool_is_globally_exclusive() {
        let custom = classify_tool_accesses("McpFoo", r#"{}"#);
        let read = classify_tool_accesses("Read", r#"{"path":"a.rs"}"#);
        assert!(accesses_conflict(&custom, &read));
    }

    #[tokio::test]
    async fn scheduler_runs_non_conflicting_tasks_in_parallel() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let mut tasks = Vec::new();
        for path in ["a.rs", "b.rs"] {
            let concurrent = Arc::clone(&concurrent);
            let max_concurrent = Arc::clone(&max_concurrent);
            let path = path.to_string();
            let accesses = classify_tool_accesses("Read", &format!(r#"{{"path":"{path}"}}"#));
            let future: std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>> =
                Box::pin(async move {
                    let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    path
                });
            tasks.push((accesses, future));
        }

        let results = ToolScheduler::run_ordered(tasks).await;
        assert_eq!(results, vec!["a.rs", "b.rs"]);
        assert!(max_concurrent.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn scheduler_serializes_conflicting_writes() {
        let order = Arc::new(Mutex::new(Vec::<usize>::new()));
        let mut tasks = Vec::new();
        for index in 0..2 {
            let order = Arc::clone(&order);
            let accesses = classify_tool_accesses("Write", r#"{"path":"same.rs"}"#);
            let future: std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send>> =
                Box::pin(async move {
                    order.lock().unwrap().push(index);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    index
                });
            tasks.push((accesses, future));
        }

        let results = ToolScheduler::run_ordered(tasks).await;
        assert_eq!(results, vec![0, 1]);
        let seen = order.lock().unwrap().clone();
        assert_eq!(seen, vec![0, 1]);
    }
}
