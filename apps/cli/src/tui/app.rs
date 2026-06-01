use std::collections::VecDeque;
use std::time::Instant;

use zene_core::{AgentEvent, PermissionMode, PromptChoice};
use zene_llm::TokenUsage;

use super::diff::compact_unified_diff;
use super::input_line::InputLine;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatLine {
    User(String),
    Assistant(String),
    Tool {
        header: String,
        body: Option<String>,
        is_error: bool,
    },
    Status(String),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Cancelling,
}

pub struct PermissionPrompt {
    pub tool_name: String,
    pub args_preview: String,
    pub response_tx: std::sync::mpsc::Sender<PromptChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSelectorMode {
    /// Pick a model under the current provider (provider/base_url unchanged).
    SelectModel,
    /// Pick or change provider (updates provider, base_url, and default model).
    SelectProvider,
}

pub struct ModelSelector {
    pub mode: ModelSelectorMode,
    pub selected_provider_index: usize,
    pub selected_model_index: usize,
    /// Custom model id (e.g. ollama/<name>) when not a known variant.
    pub model_name_override: Option<String>,
    pub input_key: Option<String>,
    pub input_model: Option<String>,
}

impl ModelSelector {
    pub fn for_models(config: &zene_config::ZeneConfig) -> Self {
        let current = config.model.clone();
        let base_url = config.base_url.clone();
        let provider_index = crate::model_config::provider_index_for_config(config).unwrap_or(0);
        let model_index = if let Some(provider) = crate::model_config::current_provider(config) {
            crate::model_config::model_index_for_provider(provider, &current)
        } else {
            let (pi, mi) =
                crate::model_config::selection_for_model_with_base_url(&current, &base_url);
            if pi == provider_index {
                mi
            } else {
                0
            }
        };
        let model_name_override = if crate::model_config::is_known_model(&current) {
            None
        } else {
            Some(current)
        };
        Self {
            mode: ModelSelectorMode::SelectModel,
            selected_provider_index: provider_index,
            selected_model_index: model_index,
            model_name_override,
            input_key: None,
            input_model: None,
        }
    }

    pub fn for_providers(config: &zene_config::ZeneConfig) -> Self {
        let provider_index = crate::model_config::provider_index_for_config(config).unwrap_or(0);
        Self {
            mode: ModelSelectorMode::SelectProvider,
            selected_provider_index: provider_index,
            selected_model_index: 0,
            model_name_override: None,
            input_key: None,
            input_model: None,
        }
    }

    pub fn for_model_with_api_key(model_id: &str) -> Self {
        let (provider_index, model_index) = crate::model_config::selection_for_model(model_id);
        Self {
            mode: ModelSelectorMode::SelectProvider,
            selected_provider_index: provider_index,
            selected_model_index: model_index,
            model_name_override: Some(model_id.to_string()),
            input_key: Some(String::new()),
            input_model: None,
        }
    }

    pub fn selected_provider(&self) -> &crate::model_config::ProviderPreset {
        &crate::model_config::PROVIDER_PRESETS[self.selected_provider_index]
    }

    pub fn effective_model_name(&self) -> String {
        if let Some(name) = &self.model_name_override {
            if !name.is_empty() {
                return name.clone();
            }
        }
        crate::model_config::preset_models(self.selected_provider())
            .get(self.selected_model_index)
            .map(|v| v.model_id.to_string())
            .unwrap_or_default()
    }
}

struct PendingEdit {
    path: String,
    old_string: String,
    new_string: String,
}

pub struct App {
    pub lines: Vec<ChatLine>,
    pub streaming: Option<String>,
    pub input: InputLine,
    pub session_id: String,
    pub model: String,
    pub usage: TokenUsage,
    pub permission_mode: PermissionMode,
    pub run_state: RunState,
    pub permission: Option<PermissionPrompt>,
    pub model_selector: Option<ModelSelector>,
    pub should_quit: bool,
    pub last_esc: Option<Instant>,
    pub scroll: u16,
    pub stick_to_bottom: bool,
    pub chat_viewport_height: u16,
    pub max_scroll: u16,
    pending_edit: Option<PendingEdit>,
    /// FIFO line indices for in-flight tool calls (paired with ToolResult).
    pending_tool_lines: VecDeque<(usize, String)>,
    /// Submitted prompts, oldest first, for ↑/↓ recall.
    history: Vec<String>,
    /// Cursor into `history`; `None` means "editing a fresh line".
    history_index: Option<usize>,
    /// Draft preserved while browsing history.
    history_draft: String,
    /// Last measured inner width of the input box (for wrap-aware cursor/history).
    pub input_wrap_width: u16,
    /// When false, mouse capture is off so Shift+drag can select terminal text.
    pub mouse_scroll_enabled: bool,
}

impl App {
    pub fn new(session_id: String, model: String, permission_mode: PermissionMode) -> Self {
        Self {
            lines: Vec::new(),
            streaming: None,
            input: InputLine::new(),
            session_id,
            model,
            usage: TokenUsage::default(),
            permission_mode,
            run_state: RunState::Idle,
            permission: None,
            model_selector: None,
            should_quit: false,
            last_esc: None,
            scroll: 0,
            stick_to_bottom: true,
            chat_viewport_height: 1,
            max_scroll: 0,
            pending_edit: None,
            pending_tool_lines: VecDeque::new(),
            history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            input_wrap_width: 80,
            mouse_scroll_enabled: true,
        }
    }

    pub fn toggle_mouse_scroll(&mut self) -> bool {
        self.mouse_scroll_enabled = !self.mouse_scroll_enabled;
        self.mouse_scroll_enabled
    }

    /// Record a submitted prompt for later recall (skips blanks and dupes).
    pub fn push_history(&mut self, entry: &str) {
        let entry = entry.trim();
        if entry.is_empty() {
            self.history_index = None;
            return;
        }
        if self.history.last().map(String::as_str) != Some(entry) {
            self.history.push(entry.to_string());
        }
        self.history_index = None;
        self.history_draft.clear();
    }

    /// Recall the previous (older) history entry into the input buffer.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_index = match self.history_index {
            None => {
                self.history_draft = self.input.as_str().to_string();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_index = Some(next_index);
        let entry = self.history[next_index].clone();
        self.input.set_text(&entry);
    }

    /// Recall the next (newer) history entry, or restore the draft past the end.
    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            let entry = self.history[index + 1].clone();
            self.history_index = Some(index + 1);
            self.input.set_text(&entry);
        } else {
            self.history_index = None;
            let draft = self.history_draft.clone();
            self.input.set_text(&draft);
        }
    }

    pub fn finish_turn(&mut self) {
        self.run_state = RunState::Idle;
        self.streaming = None;
    }

    pub fn request_cancel(&mut self) {
        if self.run_state == RunState::Running {
            self.run_state = RunState::Cancelling;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.stick_to_bottom = true;
    }

    pub fn scroll_lines_up(&mut self, lines: u16) {
        self.stick_to_bottom = false;
        self.scroll = self.scroll.saturating_sub(lines);
    }

    pub fn scroll_lines_down(&mut self, lines: u16, max_scroll: u16) {
        self.scroll = self.scroll.saturating_add(lines).min(max_scroll);
        if self.scroll >= max_scroll {
            self.stick_to_bottom = true;
        }
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

    pub fn scroll_to_top(&mut self) {
        self.stick_to_bottom = false;
        self.scroll = 0;
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TurnStart { .. } => {
                self.run_state = RunState::Running;
                self.streaming = Some(String::new());
                self.pending_tool_lines.clear();
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
                let idx = self.lines.len();
                self.lines.push(ChatLine::Tool {
                    header: format!("[tool] {name}({summary})"),
                    body: None,
                    is_error: false,
                });
                self.pending_tool_lines.push_back((idx, summary));
            }
            AgentEvent::ToolResult {
                name,
                content,
                is_error,
                ..
            } => {
                let mark = if is_error { "✗" } else { "✓" };
                let (idx, summary) = self
                    .pending_tool_lines
                    .pop_front()
                    .unwrap_or_else(|| (self.lines.len(), "…".to_string()));
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
                        .or_else(|| tool_result_body(&name, &content, is_error))
                } else {
                    self.pending_edit = None;
                    tool_result_body(&name, &content, is_error)
                };
                let header = format!("[tool] {name}({summary}) {mark}");
                let line = ChatLine::Tool {
                    header,
                    body,
                    is_error,
                };
                if idx < self.lines.len() {
                    self.lines[idx] = line;
                } else {
                    self.lines.push(line);
                }
            }
            AgentEvent::TurnEnd { .. } => {
                self.flush_streaming();
                self.lines.retain(|line| !matches!(line, ChatLine::Status(_)));
                self.pending_tool_lines.clear();
                self.finish_turn();
            }
            AgentEvent::Error { message } => {
                self.flush_streaming();
                self.lines.retain(|line| !matches!(line, ChatLine::Status(_)));
                self.pending_tool_lines.clear();
                self.lines.push(ChatLine::Error(message));
                self.finish_turn();
            }
            AgentEvent::StepBegin { step, .. } => {
                self.flush_streaming();
                self.lines.retain(|line| !matches!(line, ChatLine::Status(_)));
                if step > 1 {
                    self.lines.push(ChatLine::Status(format!(
                        "Continuing (step {step})…"
                    )));
                }
            }
            AgentEvent::SteerInput { .. } => {}
        }
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

    /// Show the model's final reply when streaming did not already render it.
    pub fn push_final_assistant(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            if self
                .lines
                .iter()
                .any(|line| matches!(line, ChatLine::Tool { .. }))
            {
                self.lines.push(ChatLine::Status(
                    "Turn finished (no summary from model; see tool results above).".to_string(),
                ));
            }
            return;
        }
        let already_shown = self.lines.iter().rev().find_map(|line| match line {
            ChatLine::Assistant(s) => Some(s.trim() == text),
            _ => None,
        });
        if already_shown != Some(true) {
            self.lines.push(ChatLine::Assistant(text.to_string()));
        }
    }
}

fn tool_result_body(name: &str, content: &str, is_error: bool) -> Option<String> {
    if is_error {
        return if content.is_empty() {
            None
        } else {
            Some(truncate(content, 200))
        };
    }
    if content.is_empty() {
        return Some("(no output)".to_string());
    }
    match name {
        "Read" | "Glob" | "Grep" => {
            let line_count = content.lines().count();
            let preview: String = content.lines().take(2).collect::<Vec<_>>().join("\n");
            let mut out = truncate(&preview, 120);
            if line_count > 2 {
                out.push_str(&format!("\n… ({line_count} lines)"));
            }
            Some(out)
        }
        "Bash" => Some(truncate(content, 200)),
        _ => None,
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
