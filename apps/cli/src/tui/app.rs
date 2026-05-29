use std::time::Instant;

use zene_core::{AgentEvent, PermissionMode, PromptChoice};
use zene_llm::TokenUsage;

use super::diff::compact_unified_diff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatLine {
    User(String),
    Assistant(String),
    Tool {
        header: String,
        body: Option<String>,
        is_error: bool,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
}

pub struct PermissionPrompt {
    pub tool_name: String,
    pub args_preview: String,
    pub response_tx: std::sync::mpsc::Sender<PromptChoice>,
}

struct PendingEdit {
    path: String,
    old_string: String,
    new_string: String,
}

pub struct App {
    pub lines: Vec<ChatLine>,
    pub streaming: Option<String>,
    pub input: String,
    pub session_id: String,
    pub model: String,
    pub usage: TokenUsage,
    pub permission_mode: PermissionMode,
    pub run_state: RunState,
    pub permission: Option<PermissionPrompt>,
    pub should_quit: bool,
    pub last_esc: Option<Instant>,
    pub scroll: u16,
    pub stick_to_bottom: bool,
    pub chat_viewport_height: u16,
    pub max_scroll: u16,
    pending_edit: Option<PendingEdit>,
}

impl App {
    pub fn new(session_id: String, model: String, permission_mode: PermissionMode) -> Self {
        Self {
            lines: Vec::new(),
            streaming: None,
            input: String::new(),
            session_id,
            model,
            usage: TokenUsage::default(),
            permission_mode,
            run_state: RunState::Idle,
            permission: None,
            should_quit: false,
            last_esc: None,
            scroll: 0,
            stick_to_bottom: true,
            chat_viewport_height: 1,
            max_scroll: 0,
            pending_edit: None,
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.stick_to_bottom = true;
    }

    pub fn scroll_page_up(&mut self) {
        self.stick_to_bottom = false;
        let page = self.chat_viewport_height.saturating_sub(2).max(1);
        self.scroll = self.scroll.saturating_sub(page);
    }

    pub fn scroll_page_down(&mut self, max_scroll: u16) {
        let page = self.chat_viewport_height.saturating_sub(2).max(1);
        self.scroll = self.scroll.saturating_add(page).min(max_scroll);
        if self.scroll >= max_scroll {
            self.stick_to_bottom = true;
        }
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TurnStart { .. } => {
                self.run_state = RunState::Running;
                self.streaming = Some(String::new());
            }
            AgentEvent::TextDelta { delta } => {
                if let Some(buf) = &mut self.streaming {
                    buf.push_str(&delta);
                } else {
                    self.streaming = Some(delta);
                }
            }
            AgentEvent::ToolCall { name, arguments } => {
                self.flush_streaming();
                if name == "Edit" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&arguments) {
                        self.pending_edit = Some(PendingEdit {
                            path: value
                                .get("path")
                                .and_then(|p| p.as_str())
                                .unwrap_or("")
                                .to_string(),
                            old_string: value
                                .get("old_string")
                                .and_then(|p| p.as_str())
                                .unwrap_or("")
                                .to_string(),
                            new_string: value
                                .get("new_string")
                                .and_then(|p| p.as_str())
                                .unwrap_or("")
                                .to_string(),
                        });
                    }
                } else {
                    self.pending_edit = None;
                }
                let summary = format_tool_summary(&name, &arguments);
                self.lines.push(ChatLine::Tool {
                    header: format!("[tool] {name}({summary})"),
                    body: None,
                    is_error: false,
                });
            }
            AgentEvent::ToolResult {
                name,
                content,
                is_error,
            } => {
                let mark = if is_error { "✗" } else { "✓" };
                let body = if name == "Edit" && !is_error {
                    self.pending_edit
                        .take()
                        .and_then(|edit| {
                            let diff = compact_unified_diff(
                                &edit.path,
                                &edit.old_string,
                                &edit.new_string,
                                20,
                            );
                            if diff.is_empty() {
                                None
                            } else {
                                Some(diff)
                            }
                        })
                        .or_else(|| {
                            if content.is_empty() {
                                None
                            } else {
                                Some(content)
                            }
                        })
                } else {
                    self.pending_edit = None;
                    if is_error && !content.is_empty() {
                        Some(truncate(&content, 200))
                    } else {
                        None
                    }
                };
                self.lines.push(ChatLine::Tool {
                    header: format!("[tool] {name} {mark}"),
                    body,
                    is_error,
                });
            }
            AgentEvent::TurnEnd { .. } => {
                self.flush_streaming();
                self.run_state = RunState::Idle;
            }
            AgentEvent::Error { message } => {
                self.flush_streaming();
                self.lines.push(ChatLine::Error(message));
                self.run_state = RunState::Idle;
            }
            AgentEvent::StepBegin { .. } => {}
        }
        self.scroll_to_bottom();
    }

    fn flush_streaming(&mut self) {
        if let Some(text) = self.streaming.take() {
            if !text.is_empty() {
                self.lines.push(ChatLine::Assistant(text));
            }
        }
    }

    pub fn update_usage(&mut self, usage: &TokenUsage) {
        self.usage = usage.clone();
    }
}

pub fn format_tool_summary(name: &str, args: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(args) {
        match name {
            "Read" | "Write" | "Edit" | "Glob" | "Grep" => {
                if let Some(path) = value.get("path").and_then(|p| p.as_str()) {
                    return truncate(path, 48);
                }
            }
            "Bash" => {
                if let Some(cmd) = value.get("command").and_then(|c| c.as_str()) {
                    return truncate(cmd, 48);
                }
            }
            _ => {}
        }
    }
    truncate(args, 48)
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}...", input.chars().take(max).collect::<String>())
    }
}
