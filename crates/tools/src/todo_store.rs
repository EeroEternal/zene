use std::sync::Arc;

use parking_lot::Mutex;

pub use zene_session::{TodoItem, TodoStatus};

#[derive(Debug, Default)]
pub struct TodoStore {
    items: Vec<TodoItem>,
}

impl TodoStore {
    pub fn from_items(items: Vec<TodoItem>) -> Self {
        Self { items }
    }

    pub fn merge(&mut self, updates: &[TodoItem]) {
        for update in updates {
            if let Some(existing) = self.items.iter_mut().find(|t| t.id == update.id) {
                existing.content = update.content.clone();
                existing.status = update.status;
            } else {
                self.items.push(update.clone());
            }
        }
    }

    pub fn items(&self) -> &[TodoItem] {
        &self.items
    }

    pub fn to_items(&self) -> Vec<TodoItem> {
        self.items.clone()
    }

    pub fn render_summary(&self) -> String {
        if self.items.is_empty() {
            return "Todo list is empty.".to_string();
        }
        let lines: Vec<String> = self
            .items
            .iter()
            .map(|t| format!("  [{}] {} — {}", status_label(t.status), t.id, t.content))
            .collect();
        format!(
            "Current todo list ({} items):\n{}",
            self.items.len(),
            lines.join("\n")
        )
    }
}

fn status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Pending => "pending",
        TodoStatus::InProgress => "in_progress",
        TodoStatus::Completed => "completed",
    }
}

pub type SharedTodoStore = Arc<Mutex<TodoStore>>;

pub fn shared_todo_store() -> SharedTodoStore {
    Arc::new(Mutex::new(TodoStore::default()))
}

pub fn shared_todo_store_from(items: Vec<TodoItem>) -> SharedTodoStore {
    Arc::new(Mutex::new(TodoStore::from_items(items)))
}
