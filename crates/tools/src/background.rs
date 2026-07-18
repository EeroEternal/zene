//! Background task store for Bash / Task jobs (grok-aligned minimal set).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskKind {
    Bash,
    Subagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl BackgroundTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: String,
    pub kind: BackgroundTaskKind,
    pub status: BackgroundTaskStatus,
    pub label: String,
    pub output: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Default)]
pub struct BackgroundTaskStore {
    tasks: HashMap<String, BackgroundTask>,
    cancels: HashMap<String, CancellationToken>,
}

impl BackgroundTaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc_id(prefix: &str) -> String {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{n}")
    }

    pub fn insert_running(
        &mut self,
        id: String,
        kind: BackgroundTaskKind,
        label: String,
        cancel: CancellationToken,
    ) {
        self.cancels.insert(id.clone(), cancel);
        self.tasks.insert(
            id.clone(),
            BackgroundTask {
                id,
                kind,
                status: BackgroundTaskStatus::Running,
                label,
                output: String::new(),
                exit_code: None,
            },
        );
    }

    pub fn cancel_token(&self, id: &str) -> Option<CancellationToken> {
        self.cancels.get(id).cloned()
    }

    pub fn finish(
        &mut self,
        id: &str,
        status: BackgroundTaskStatus,
        output: String,
        exit_code: Option<i32>,
    ) {
        if let Some(task) = self.tasks.get_mut(id) {
            // Don't overwrite an explicit cancel with a late worker finish.
            if task.status == BackgroundTaskStatus::Cancelled
                && status != BackgroundTaskStatus::Cancelled
            {
                return;
            }
            task.status = status;
            if !output.is_empty() {
                task.output = output;
            }
            task.exit_code = exit_code;
        }
        self.cancels.remove(id);
    }

    pub fn request_cancel(&mut self, id: &str) -> bool {
        let Some(task) = self.tasks.get(id) else {
            return false;
        };
        if task.status != BackgroundTaskStatus::Running {
            return false;
        }
        if let Some(token) = self.cancels.get(id) {
            token.cancel();
        }
        let output = format!("{}\n[cancelled by TaskOutput]", task.output);
        self.finish(id, BackgroundTaskStatus::Cancelled, output, None);
        true
    }

    pub fn get(&self, id: &str) -> Option<BackgroundTask> {
        self.tasks.get(id).cloned()
    }

    pub fn list(&self) -> Vec<BackgroundTask> {
        let mut tasks: Vec<_> = self.tasks.values().cloned().collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        tasks
    }
}

pub type SharedBackgroundTasks = Arc<Mutex<BackgroundTaskStore>>;

pub fn shared_background_tasks() -> SharedBackgroundTasks {
    Arc::new(Mutex::new(BackgroundTaskStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_updates_status() {
        let mut store = BackgroundTaskStore::new();
        store.insert_running(
            "bg-1".into(),
            BackgroundTaskKind::Bash,
            "ls".into(),
            CancellationToken::new(),
        );
        store.finish(
            "bg-1",
            BackgroundTaskStatus::Completed,
            "ok\n".into(),
            Some(0),
        );
        let task = store.get("bg-1").unwrap();
        assert_eq!(task.status, BackgroundTaskStatus::Completed);
        assert_eq!(task.output, "ok\n");
        assert_eq!(task.exit_code, Some(0));
    }

    #[test]
    fn request_cancel_marks_cancelled() {
        let mut store = BackgroundTaskStore::new();
        let token = CancellationToken::new();
        store.insert_running(
            "bg-2".into(),
            BackgroundTaskKind::Bash,
            "sleep".into(),
            token.clone(),
        );
        assert!(store.request_cancel("bg-2"));
        assert!(token.is_cancelled());
        assert_eq!(
            store.get("bg-2").unwrap().status,
            BackgroundTaskStatus::Cancelled
        );
    }
}
