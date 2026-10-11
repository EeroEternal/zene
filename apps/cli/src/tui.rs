//! `zene tui` — full terminal chat over the same agent core as `zene acp`.
//!
//! Chat-style transcript with streaming output, a Claude-Code-style popup
//! command menu (type `/` to float candidates, ↑↓ select, Tab/Enter complete,
//! Esc close), and a pi.dev-style provider wizard for API keys. The agent runs
//! in a spawned task and talks to the UI over channels so the terminal stays
//! responsive.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc;
use zene_config::ZeneConfig;
use zene_core::{Agent, AgentEvent, PromptOptions};

/// Slash commands surfaced in the popup menu.
struct Command {
    name: &'static str,
    hint: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "key",
        hint: "configure provider & API key",
    },
    Command {
        name: "model",
        hint: "select model (/model <name>)",
    },
    Command {
        name: "help",
        hint: "list commands",
    },
    Command {
        name: "clear",
        hint: "clear transcript",
    },
    Command {
        name: "quit",
        hint: "exit",
    },
];

/// Model candidates for the current provider family (keyed by base_url).
fn family_models(base_url: &str) -> &'static [&'static str] {
    let b = base_url.to_lowercase();
    if b.contains("deepseek") {
        &["deepseek-chat", "deepseek-reasoner"]
    } else if b.contains("bigmodel") || b.contains("zhipu") {
        &["glm-4.5", "glm-4.5-air", "glm-4-flash", "glm-4-plus"]
    } else if b.contains("moonshot") || b.contains("kimi") {
        &["moonshot-v1-8k", "moonshot-v1-32k", "kimi-k2-0711-preview"]
    } else if b.contains("openai") {
        &["gpt-4o", "gpt-4o-mini", "gpt-4.1"]
    } else if b.contains("anthropic") {
        &["claude-sonnet-4-5", "claude-opus-4-1", "claude-haiku-4-5"]
    } else {
        &[]
    }
}

/// Candidates filtered by the picker query; empty for unknown families
/// (the picker then falls back to free-text input).
fn model_candidates(base_url: &str, query: &str) -> Vec<&'static str> {
    let q = query.to_lowercase();
    family_models(base_url)
        .iter()
        .copied()
        .filter(|m| m.to_lowercase().contains(&q))
        .collect()
}

/// A provider preset for the key wizard.
struct Provider {
    name: &'static str,
    /// Zene config provider field (`openai` / `anthropic`).
    provider: &'static str,
    base_url: &'static str,
    model: &'static str,
    /// Env var consulted for the "configured" status hint.
    env: &'static str,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        name: "DeepSeek",
        provider: "openai",
        base_url: "https://api.deepseek.com",
        model: "deepseek-chat",
        env: "DEEPSEEK_API_KEY",
    },
    Provider {
        name: "Zhipu (GLM)",
        provider: "openai",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        model: "glm-4-plus",
        env: "ZHIPUAI_API_KEY",
    },
    Provider {
        name: "Moonshot (Kimi)",
        provider: "openai",
        base_url: "https://api.moonshot.cn/v1",
        model: "moonshot-v1-8k",
        env: "MOONSHOT_API_KEY",
    },
    Provider {
        name: "OpenAI",
        provider: "openai",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o",
        env: "OPENAI_API_KEY",
    },
    Provider {
        name: "Anthropic",
        provider: "anthropic",
        base_url: "https://api.anthropic.com",
        model: "claude-sonnet-4-5",
        env: "ANTHROPIC_API_KEY",
    },
    Provider {
        name: "Custom (base URL)…",
        provider: "openai",
        base_url: "",
        model: "",
        env: "",
    },
];

pub(crate) async fn run(workdir: &Path) -> Result<()> {
    let config = ZeneConfig::load(workdir).map_err(|err| anyhow!(err.to_string()))?;
    let model = config.model.clone();
    let base_url = if config.provider.trim().eq_ignore_ascii_case("anthropic") {
        config
            .anthropic_base_url
            .clone()
            .unwrap_or_else(|| config.base_url.clone())
    } else {
        config.base_url.clone()
    };
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<UiMsg>();
    tokio::spawn(agent_loop(workdir.to_path_buf(), config, cmd_rx, msg_tx));

    let mut ui = Ui::new(model, workdir.display().to_string());
    ui.base_url = base_url;
    let terminal = ratatui::init();
    let result = ui_loop(terminal, &mut ui, &cmd_tx, &mut msg_rx);
    ratatui::restore();
    let _ = cmd_tx.send(Cmd::Quit);
    result
}

/// Agent-side configuration update (model / provider / endpoint / key).
#[derive(Default, Debug)]
struct Configure {
    model: Option<String>,
    provider: Option<String>,
    base_url: Option<String>,
    api_key: String,
}

/// Messages into the agent task.
enum Cmd {
    Prompt(String),
    Configure(Configure),
    Quit,
}

/// Messages back to the UI.
enum UiMsg {
    Stream(String),
    Tool(String),
    Done(String),
    Error(String),
    Notice(String),
}

/// Owns the agent across turns (session continuity lives here).
async fn agent_loop(
    workdir: PathBuf,
    config: ZeneConfig,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    tx: mpsc::UnboundedSender<UiMsg>,
) {
    let mut agent = match Agent::builder(&workdir)
        .config(config)
        .core_tools()
        .build()
        .await
    {
        Ok(agent) => agent,
        Err(err) => {
            let _ = tx.send(UiMsg::Error(format!("agent setup failed: {err:#}")));
            return;
        }
    };
    let _ = tx.send(UiMsg::Notice("ready — type / for commands".into()));
    while let Some(cmd) = cmds.recv().await {
        match cmd {
            Cmd::Prompt(text) => {
                let event_tx = tx.clone();
                let handler: zene_core::EventHandler = Arc::new(move |event| match event {
                    AgentEvent::TextDelta { delta } => {
                        let _ = event_tx.send(UiMsg::Stream(delta));
                    }
                    AgentEvent::ToolCall { name, .. } => {
                        let _ = event_tx.send(UiMsg::Tool(format!("⚙ {name}")));
                    }
                    AgentEvent::ToolResult { name, is_error, .. } => {
                        let mark = if is_error { "✗" } else { "✓" };
                        let _ = event_tx.send(UiMsg::Tool(format!("{mark} {name}")));
                    }
                    _ => {}
                });
                let reply = agent
                    .prompt(
                        &text,
                        PromptOptions {
                            stream: true,
                            event_handler: Some(handler),
                            quiet: true,
                            ..Default::default()
                        },
                    )
                    .await;
                match reply {
                    Ok(final_text) => {
                        let _ = tx.send(UiMsg::Done(final_text));
                    }
                    Err(err) => {
                        let _ = tx.send(UiMsg::Error(format!("{err:#}")));
                    }
                }
            }
            Cmd::Configure(update) => {
                let current = agent.config().model.clone();
                let model = update.model.clone().unwrap_or(current);
                // Empty key means "unchanged" — never clobber an existing one.
                let api_key = if update.api_key.is_empty() {
                    None
                } else {
                    Some(update.api_key)
                };
                let full = update.provider.is_some() || api_key.is_some();
                match agent
                    .switch_model(
                        &model,
                        update.provider.clone(),
                        update.base_url.clone(),
                        api_key,
                    )
                    .await
                {
                    Ok(()) => {
                        let msg = if full {
                            format!(
                                "configured — model → {model}; provider + key saved to ~/.zene/config.toml"
                            )
                        } else {
                            format!("model → {model} (saved to ~/.zene/config.toml)")
                        };
                        let _ = tx.send(UiMsg::Notice(msg));
                    }
                    Err(err) => {
                        let _ = tx.send(UiMsg::Error(format!("configuration failed: {err:#}")));
                    }
                }
            }
            Cmd::Quit => break,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MsgKind {
    User,
    Assistant,
    Tool,
    Notice,
    Error,
}

struct Msg {
    kind: MsgKind,
    text: String,
}

/// The pi.dev-style wizard: pick provider (for /key) or pick model (for
/// /model), then type values.
#[derive(Clone, Copy, PartialEq, Debug)]
enum WizardStage {
    PickProvider,
    EnterBaseUrl,
    EnterKey,
    PickModel,
    EnterModel,
}

struct Wizard {
    stage: WizardStage,
    /// Provider filter query (stage 1).
    query: String,
    sel: usize,
    provider: Option<&'static Provider>,
    /// Collected for the Custom preset.
    base_url: String,
    /// Model candidates (PickModel stage).
    models: Vec<&'static str>,
}

impl Wizard {
    fn new() -> Self {
        Self {
            stage: WizardStage::PickProvider,
            query: String::new(),
            sel: 0,
            provider: None,
            base_url: String::new(),
            models: Vec::new(),
        }
    }

    fn for_model(models: Vec<&'static str>) -> Self {
        Self {
            stage: if models.is_empty() {
                WizardStage::EnterModel
            } else {
                WizardStage::PickModel
            },
            models,
            ..Self::new()
        }
    }

    fn matches(&self) -> Vec<&'static Provider> {
        let q = self.query.to_lowercase();
        PROVIDERS
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&q))
            .collect()
    }

    /// Current selectable list length (for ↑↓ clamping), by stage.
    fn list_len(&self) -> usize {
        match self.stage {
            WizardStage::PickProvider => self.matches().len(),
            WizardStage::PickModel => {
                let q = self.query.to_lowercase();
                self.models
                    .iter()
                    .filter(|m| m.to_lowercase().contains(&q))
                    .count()
            }
            _ => 0,
        }
    }
}

struct Ui {
    messages: Vec<Msg>,
    streaming: String,
    input: String,
    scroll: u16,
    follow: bool,
    busy: bool,
    model: String,
    workdir: String,
    /// Popup command menu selection index.
    menu_sel: usize,
    /// Key wizard, open while configuring a provider.
    wizard: Option<Wizard>,
    /// Current provider endpoint; selects the model candidate family.
    base_url: String,
}

impl Ui {
    fn new(model: String, workdir: String) -> Self {
        Self {
            messages: Vec::new(),
            streaming: String::new(),
            input: String::new(),
            scroll: 0,
            follow: true,
            busy: false,
            model,
            workdir,
            menu_sel: 0,
            wizard: None,
            base_url: String::new(),
        }
    }

    fn push(&mut self, kind: MsgKind, text: String) {
        self.messages.push(Msg { kind, text });
        self.follow = true;
    }
}

/// What the UI does with one key press (pure; the IO loop only dispatches).
enum Action {
    Quit,
    Send(Cmd),
    None,
}

/// The popup is open while the input is still just a slash word (no argument
/// typed yet).
fn menu_query(input: &str) -> Option<&str> {
    let body = input.strip_prefix('/')?;
    if body.contains(char::is_whitespace) {
        return None;
    }
    Some(body)
}

/// Commands matching the current menu query.
fn menu_matches(query: &str) -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(query))
        .collect()
}

fn apply_key(ui: &mut Ui, key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }
    // The wizard owns the keyboard while open.
    if ui.wizard.is_some() {
        return wizard_key(ui, key);
    }
    // Popup navigation: the menu owns arrows/Tab/Enter/Esc while open.
    if menu_query(&ui.input).is_some() {
        let matches = menu_matches(menu_query(&ui.input).unwrap_or_default());
        if !matches.is_empty() {
            match key.code {
                KeyCode::Up => {
                    ui.menu_sel = ui.menu_sel.saturating_sub(1);
                    return Action::None;
                }
                KeyCode::Down => {
                    if ui.menu_sel + 1 < matches.len() {
                        ui.menu_sel += 1;
                    }
                    return Action::None;
                }
                KeyCode::Tab | KeyCode::Enter | KeyCode::Right => {
                    return menu_pick(ui, matches);
                }
                KeyCode::Esc => {
                    ui.input.clear();
                    ui.menu_sel = 0;
                    return Action::None;
                }
                _ => {}
            }
        }
    }
    match key.code {
        KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Enter if !ui.input.trim().is_empty() => {
            let input = std::mem::take(&mut ui.input);
            ui.menu_sel = 0;
            if let Some(action) = slash_command(ui, &input) {
                return action;
            }
            if ui.busy {
                ui.push(
                    MsgKind::Notice,
                    "busy — wait for the turn to finish (Esc quits)".into(),
                );
                return Action::None;
            }
            ui.push(MsgKind::User, input.clone());
            ui.busy = true;
            ui.follow = true;
            Action::Send(Cmd::Prompt(input))
        }
        KeyCode::Backspace => {
            ui.input.pop();
            ui.menu_sel = 0;
            Action::None
        }
        KeyCode::Char(c) => {
            ui.input.push(c);
            Action::None
        }
        KeyCode::PageUp => {
            ui.follow = false;
            ui.scroll = ui.scroll.saturating_add(5);
            Action::None
        }
        KeyCode::PageDown => {
            ui.scroll = ui.scroll.saturating_sub(5);
            Action::None
        }
        _ => Action::None,
    }
}

/// Complete the highlighted menu entry: arg-taking commands fill the input,
/// the rest execute immediately.
fn menu_pick(ui: &mut Ui, matches: Vec<&'static Command>) -> Action {
    let cmd = matches[ui.menu_sel.min(matches.len() - 1)];
    let input = format!("/{}", cmd.name);
    ui.input.clear();
    ui.menu_sel = 0;
    slash_command(ui, &input).unwrap_or(Action::None)
}

/// Keyboard handling while the key wizard is open.
fn wizard_key(ui: &mut Ui, key: KeyEvent) -> Action {
    let wizard = ui.wizard.as_mut().expect("wizard is open");
    match key.code {
        KeyCode::Esc => {
            ui.wizard = None;
            ui.input.clear();
            ui.push(MsgKind::Notice, "cancelled".into());
            Action::None
        }
        KeyCode::Up => {
            wizard.sel = wizard.sel.saturating_sub(1);
            Action::None
        }
        KeyCode::Down => {
            if wizard.sel + 1 < wizard.list_len() {
                wizard.sel += 1;
            }
            Action::None
        }
        KeyCode::Backspace => {
            match wizard.stage {
                WizardStage::PickProvider | WizardStage::PickModel => {
                    wizard.query.pop();
                }
                WizardStage::EnterBaseUrl | WizardStage::EnterKey | WizardStage::EnterModel => {
                    ui.input.pop();
                }
            }
            Action::None
        }
        KeyCode::Char(c) => {
            match wizard.stage {
                WizardStage::PickProvider | WizardStage::PickModel => wizard.query.push(c),
                WizardStage::EnterBaseUrl | WizardStage::EnterKey | WizardStage::EnterModel => {
                    ui.input.push(c)
                }
            }
            Action::None
        }
        KeyCode::Enter | KeyCode::Right => wizard_advance(ui),
        _ => Action::None,
    }
}

/// Advance the wizard one stage; the last stage sends the configuration.
fn wizard_advance(ui: &mut Ui) -> Action {
    let wizard = ui.wizard.as_mut().expect("wizard is open");
    match wizard.stage {
        WizardStage::PickProvider => {
            let matches = wizard.matches();
            if matches.is_empty() {
                return Action::None;
            }
            let picked = matches[wizard.sel.min(matches.len() - 1)];
            wizard.provider = Some(picked);
            wizard.sel = 0;
            ui.input.clear();
            wizard.stage = if picked.base_url.is_empty() {
                WizardStage::EnterBaseUrl
            } else {
                WizardStage::EnterKey
            };
            Action::None
        }
        WizardStage::EnterBaseUrl => {
            let base = ui.input.trim().to_string();
            if base.is_empty() {
                return Action::None;
            }
            ui.wizard.as_mut().expect("wizard is open").base_url = base;
            ui.input.clear();
            ui.wizard.as_mut().expect("wizard is open").stage = WizardStage::EnterKey;
            Action::None
        }
        WizardStage::EnterKey => {
            let key_text = ui.input.trim().to_string();
            if key_text.is_empty() {
                return Action::None;
            }
            let wizard = ui.wizard.take().expect("wizard is open");
            let preset = wizard.provider.expect("provider picked");
            let base_url = if wizard.base_url.is_empty() {
                preset.base_url.to_string()
            } else {
                wizard.base_url
            };
            ui.input.clear();
            ui.model = if preset.model.is_empty() {
                ui.model.clone()
            } else {
                preset.model.to_string()
            };
            ui.push(
                MsgKind::User,
                format!("/key {} ••••{}", preset.name, tail_mask(&key_text)),
            );
            ui.base_url = base_url.clone();
            ui.busy = true;
            Action::Send(Cmd::Configure(Configure {
                model: if preset.model.is_empty() {
                    None
                } else {
                    Some(preset.model.to_string())
                },
                provider: Some(preset.provider.to_string()),
                base_url: Some(base_url),
                api_key: key_text,
            }))
        }
        WizardStage::PickModel => {
            let candidates: Vec<&'static str> = {
                let q = wizard.query.to_lowercase();
                wizard
                    .models
                    .iter()
                    .copied()
                    .filter(|m| m.to_lowercase().contains(&q))
                    .collect()
            };
            if candidates.is_empty() {
                // No match — fall back to free-text model entry.
                wizard.stage = WizardStage::EnterModel;
                ui.input.clear();
                return Action::None;
            }
            let picked = candidates[wizard.sel.min(candidates.len() - 1)].to_string();
            ui.wizard = None;
            ui.input.clear();
            ui.model = picked.clone();
            ui.push(MsgKind::User, format!("/model {picked}"));
            ui.busy = true;
            Action::Send(Cmd::Configure(Configure {
                model: Some(picked),
                ..Default::default()
            }))
        }
        WizardStage::EnterModel => {
            let name = ui.input.trim().to_string();
            if name.is_empty() {
                return Action::None;
            }
            ui.wizard = None;
            ui.input.clear();
            ui.model = name.clone();
            ui.push(MsgKind::User, format!("/model {name}"));
            ui.busy = true;
            Action::Send(Cmd::Configure(Configure {
                model: Some(name),
                ..Default::default()
            }))
        }
    }
}

/// Handle `/` inputs locally or turn them into agent commands.
/// Returns None when the input is a normal prompt.
fn slash_command(ui: &mut Ui, input: &str) -> Option<Action> {
    let body = input.strip_prefix('/')?;
    let mut parts = body.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or_default();
    let arg = parts.next().unwrap_or_default().trim();
    match cmd {
        "help" => {
            let help = COMMANDS
                .iter()
                .map(|c| format!("/{}  — {}", c.name, c.hint))
                .collect::<Vec<_>>()
                .join("\n");
            ui.push(MsgKind::Notice, help);
            Some(Action::None)
        }
        "model" if arg.is_empty() => {
            // Open the model picker (pi.dev-style), free-text for unknown
            // provider families.
            let candidates = model_candidates(&ui.base_url, "");
            ui.input.clear();
            ui.wizard = Some(Wizard::for_model(candidates));
            Some(Action::None)
        }
        "model" => {
            ui.push(MsgKind::User, input.to_string());
            ui.model = arg.to_string();
            ui.busy = true;
            Some(Action::Send(Cmd::Configure(Configure {
                model: Some(arg.to_string()),
                api_key: String::new(),
                ..Default::default()
            })))
        }
        "clear" => {
            ui.messages.clear();
            ui.streaming.clear();
            Some(Action::None)
        }
        "quit" => Some(Action::Quit),
        "key" | "api-key" if !arg.is_empty() => {
            ui.push(MsgKind::User, format!("/key ••••{}", tail_mask(arg)));
            ui.busy = true;
            Some(Action::Send(Cmd::Configure(Configure {
                api_key: arg.to_string(),
                ..Default::default()
            })))
        }
        "key" | "api-key" => {
            // Open the provider wizard (pi.dev-style).
            ui.input.clear();
            ui.wizard = Some(Wizard::new());
            Some(Action::None)
        }
        other => {
            ui.push(
                MsgKind::Error,
                format!("unknown command: /{other} — type / for commands"),
            );
            Some(Action::None)
        }
    }
}

/// Last two chars of a secret, for a masked transcript echo.
fn tail_mask(secret: &str) -> String {
    let len = secret.chars().count();
    secret.chars().skip(len.saturating_sub(2)).collect()
}

/// State transition for one agent message (pure; testable).
fn apply_msg(ui: &mut Ui, msg: UiMsg) {
    match msg {
        UiMsg::Stream(delta) => ui.streaming.push_str(&delta),
        UiMsg::Tool(line) => ui.push(MsgKind::Tool, line),
        UiMsg::Done(text) => {
            ui.streaming.clear();
            ui.push(MsgKind::Assistant, text);
            ui.busy = false;
        }
        UiMsg::Error(text) => {
            ui.streaming.clear();
            ui.push(MsgKind::Error, text);
            ui.busy = false;
        }
        // Notice is also the completion signal for Configure commands (and
        // startup "ready"): always settle busy.
        UiMsg::Notice(text) => {
            ui.push(MsgKind::Notice, text);
            ui.busy = false;
        }
    }
}

fn ui_loop(
    mut terminal: DefaultTerminal,
    ui: &mut Ui,
    cmds: &mpsc::UnboundedSender<Cmd>,
    msgs: &mut mpsc::UnboundedReceiver<UiMsg>,
) -> Result<()> {
    loop {
        while let Ok(msg) = msgs.try_recv() {
            apply_msg(ui, msg);
        }
        // ponytail: blocking poll on the async runtime thread; split into a
        // task + select if the agent ever needs same-thread concurrency.
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match apply_key(ui, key) {
                    Action::Quit => return Ok(()),
                    Action::Send(cmd) => cmds.send(cmd).context("agent task is gone")?,
                    Action::None => {}
                }
            }
        }
        terminal.draw(|frame| draw(frame, ui))?;
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        out.push(cur);
    }
    out
}

fn transcript_lines(ui: &Ui, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let push_text = |lines: &mut Vec<Line<'static>>, text: &str, style: Style| {
        for l in wrap(text, width) {
            lines.push(Line::from(Span::styled(l, style)));
        }
    };
    for msg in &ui.messages {
        match msg.kind {
            MsgKind::User => {
                lines.push(Line::from(Span::styled(
                    "You",
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )));
                push_text(&mut lines, &msg.text, Style::new().fg(Color::Cyan));
            }
            MsgKind::Assistant => {
                lines.push(Line::from(Span::styled(
                    "Zene",
                    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                )));
                push_text(&mut lines, &msg.text, Style::new());
            }
            MsgKind::Tool => {
                push_text(&mut lines, &msg.text, Style::new().fg(Color::DarkGray));
            }
            MsgKind::Notice => {
                push_text(&mut lines, &msg.text, Style::new().fg(Color::Yellow));
            }
            MsgKind::Error => {
                push_text(&mut lines, &msg.text, Style::new().fg(Color::Red));
            }
        }
        lines.push(Line::from(""));
    }
    if !ui.streaming.is_empty() {
        lines.push(Line::from(Span::styled(
            "Zene",
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        )));
        push_text(&mut lines, &ui.streaming, Style::new());
        lines.push(Line::from(""));
    }
    lines
}

fn draw(frame: &mut Frame, ui: &Ui) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(frame.area());

    let status = if ui.busy {
        format!("zene · {} · ⚙ working…", ui.model)
    } else {
        format!("zene · {} · {}", ui.model, ui.workdir)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            status,
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        ))),
        chunks[0],
    );

    let width = chunks[1].width.saturating_sub(1) as usize;
    let all = transcript_lines(ui, width);
    let visible = chunks[1].height as usize;
    let max_scroll = all.len().saturating_sub(visible) as u16;
    let scroll = if ui.follow {
        max_scroll
    } else {
        ui.scroll.min(max_scroll)
    };
    frame.render_widget(
        Paragraph::new(all)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        chunks[1],
    );

    // Popup command menu over the transcript bottom, open while the input is
    // still just a slash word.
    if let Some(query) = menu_query(&ui.input) {
        let matches = menu_matches(query);
        if !matches.is_empty() {
            draw_menu(frame, chunks[1], ui, &matches);
        }
    }

    // Provider/key wizard panel.
    if let Some(wizard) = &ui.wizard {
        draw_wizard(frame, chunks[1], ui, wizard);
    }

    // Input line: masked while typing a secret.
    let input_text = if ui
        .wizard
        .as_ref()
        .is_some_and(|w| w.stage == WizardStage::EnterKey)
    {
        "•".repeat(ui.input.chars().count())
    } else {
        ui.input.clone()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::new().fg(Color::Green)),
            Span::raw(input_text),
            Span::styled("▌", Style::new().fg(Color::Green)),
        ])),
        chunks[2],
    );

    let hints = if ui.wizard.is_some() {
        "enter next · esc cancel"
    } else if menu_query(&ui.input).is_some() {
        "↑↓ select · Tab/Enter complete · Esc close"
    } else if ui.busy {
        "working… Esc quit"
    } else {
        "Enter send · / commands · PgUp/PgDn scroll · Esc quit"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hints,
            Style::new().fg(Color::DarkGray),
        ))),
        chunks[3],
    );
}

/// Floating command menu anchored to the bottom of the transcript area.
fn draw_menu(frame: &mut Frame, area: Rect, ui: &Ui, matches: &[&'static Command]) {
    let shown = matches.len().min(8) as u16;
    let height = shown + 2;
    let width = area.width.min(56);
    let rect = Rect {
        x: area.x,
        y: area.bottom().saturating_sub(height),
        width,
        height,
    };
    let lines: Vec<Line<'static>> = matches
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, cmd)| {
            let selected = i == ui.menu_sel;
            let style = if selected {
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            Line::from(Span::styled(
                format!(" /{:<6} {}", cmd.name, cmd.hint),
                style,
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title("commands")
                .border_style(Style::new().fg(Color::Green)),
        ),
        rect,
    );
}

/// pi.dev-style provider wizard panel.
fn draw_wizard(frame: &mut Frame, area: Rect, ui: &Ui, wizard: &Wizard) {
    let width = area.width.min(64);
    let mut lines: Vec<Line<'static>> = Vec::new();
    match wizard.stage {
        WizardStage::PickProvider => {
            lines.push(Line::from(Span::styled(
                "Select provider to configure:",
                Style::new().fg(Color::Green),
            )));
            lines.push(Line::from(format!(" > {}▌", wizard.query)));
            lines.push(Line::from(""));
            for (i, p) in wizard.matches().iter().take(8).enumerate() {
                let selected = i == wizard.sel;
                let status = if p.env.is_empty() {
                    "manual"
                } else if std::env::var(p.env).is_ok_and(|v| !v.is_empty()) {
                    "configured"
                } else {
                    "not configured"
                };
                let arrow = if selected { "→ " } else { "  " };
                let name_style = if selected {
                    Style::new()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                lines.push(Line::from(vec![
                    Span::styled(arrow, name_style),
                    Span::styled(format!("{:<18}", p.name), name_style),
                    Span::styled(format!("· {status}"), Style::new().fg(Color::DarkGray)),
                ]));
            }
        }
        WizardStage::EnterBaseUrl => {
            let name = wizard.provider.map(|p| p.name).unwrap_or("Custom");
            lines.push(Line::from(Span::styled(
                format!("Base URL for {name}:"),
                Style::new().fg(Color::Green),
            )));
            lines.push(Line::from(format!(" > {}▌", ui.input)));
        }
        WizardStage::EnterKey => {
            let name = wizard.provider.map(|p| p.name).unwrap_or("provider");
            lines.push(Line::from(Span::styled(
                format!("Enter API key for {name}:"),
                Style::new().fg(Color::Green),
            )));
            lines.push(Line::from(format!(
                " > {}▌",
                "•".repeat(ui.input.chars().count())
            )));
        }
        WizardStage::PickModel => {
            lines.push(Line::from(Span::styled(
                format!("Select model for {}:", short_family(&ui.base_url)),
                Style::new().fg(Color::Green),
            )));
            lines.push(Line::from(format!(" > {}▌", wizard.query)));
            lines.push(Line::from(""));
            let q = wizard.query.to_lowercase();
            for (i, m) in wizard
                .models
                .iter()
                .filter(|m| m.to_lowercase().contains(&q))
                .take(10)
                .enumerate()
            {
                let selected = i == wizard.sel;
                let style = if selected {
                    Style::new()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                let current = if *m == ui.model { "  • current" } else { "" };
                lines.push(Line::from(vec![
                    Span::styled(if selected { "→ " } else { "  " }, style),
                    Span::styled(format!("{:<24}", m), style),
                    Span::styled(current, Style::new().fg(Color::DarkGray)),
                ]));
            }
        }
        WizardStage::EnterModel => {
            lines.push(Line::from(Span::styled(
                "Model name (not in the list):",
                Style::new().fg(Color::Green),
            )));
            lines.push(Line::from(format!(" > {}▌", ui.input)));
        }
    }
    let height = lines.len() as u16 + 2;
    let rect = Rect {
        x: area.x,
        y: area.bottom().saturating_sub(height),
        width,
        height,
    };
    let title = match wizard.stage {
        WizardStage::PickModel | WizardStage::EnterModel => "model",
        _ => "api key",
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(Color::Green)),
        ),
        rect,
    );
}

/// Short family label for picker headers.
fn short_family(base_url: &str) -> &'static str {
    let b = base_url.to_lowercase();
    if b.contains("deepseek") {
        "deepseek"
    } else if b.contains("bigmodel") || b.contains("zhipu") {
        "zhipu"
    } else if b.contains("moonshot") || b.contains("kimi") {
        "moonshot"
    } else if b.contains("openai") {
        "openai"
    } else if b.contains("anthropic") {
        "anthropic"
    } else {
        "this provider"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(ui: &mut Ui, s: &str) {
        for c in s.chars() {
            apply_key(ui, key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn enter_sends_prompt_and_clears_input() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "hi");
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::Prompt(p)) => assert_eq!(p, "hi"),
            _ => panic!("Enter must send a prompt"),
        }
        assert!(ui.input.is_empty());
        assert!(ui.busy);
        // Prompt while busy is refused with a notice, not sent.
        type_str(&mut ui, "again");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::None
        ));
        assert!(matches!(ui.messages.last().unwrap().kind, MsgKind::Notice));
    }

    #[test]
    fn menu_opens_on_slash_and_filters() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/");
        assert_eq!(
            menu_matches("/".trim_start_matches('/')).len(),
            COMMANDS.len()
        );
        type_str(&mut ui, "mo");
        assert!(menu_query(&ui.input).is_some());
        let matches = menu_matches("mo");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "model");
    }

    #[test]
    fn menu_down_up_navigates() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/");
        assert_eq!(ui.menu_sel, 0);
        apply_key(&mut ui, key(KeyCode::Down));
        assert_eq!(ui.menu_sel, 1);
        apply_key(&mut ui, key(KeyCode::Up));
        assert_eq!(ui.menu_sel, 0);
        apply_key(&mut ui, key(KeyCode::Up));
        assert_eq!(ui.menu_sel, 0, "Up at top must clamp");
    }

    #[test]
    fn menu_enter_opens_model_wizard_and_quit_runs() {
        // /model from the menu opens the model wizard (unknown family here:
        // Ui::new has no base_url, so free-text entry stage).
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/model");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::None
        ));
        assert!(ui.input.is_empty());
        let wizard = ui.wizard.as_ref().expect("model wizard must open");
        assert_eq!(wizard.stage, WizardStage::EnterModel);
        // /quit runs immediately.
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/quit");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::Quit
        ));
    }

    #[test]
    fn menu_esc_closes_without_quitting() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/key");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Esc)),
            Action::None
        ));
        assert!(ui.input.is_empty());
        // With no menu open, Esc quits.
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Esc)),
            Action::Quit
        ));
    }

    #[test]
    fn slash_key_opens_wizard() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/key");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::None
        ));
        let wizard = ui.wizard.as_ref().expect("wizard must open");
        assert_eq!(wizard.stage, WizardStage::PickProvider);
    }

    #[test]
    fn wizard_picks_provider_and_submits_key() {
        let mut ui = Ui::new("m".into(), "w".into());
        ui.wizard = Some(Wizard::new());
        // Filter to DeepSeek, pick it.
        type_str(&mut ui, "deep");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::None
        ));
        let wizard = ui.wizard.as_ref().unwrap();
        assert_eq!(wizard.stage, WizardStage::EnterKey);
        assert_eq!(wizard.provider.unwrap().name, "DeepSeek");
        // Type the key, Enter sends Configure with provider fields filled.
        type_str(&mut ui, "sk-secret-1234");
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::Configure(cfg)) => {
                assert_eq!(cfg.api_key, "sk-secret-1234");
                assert_eq!(cfg.provider.as_deref(), Some("openai"));
                assert_eq!(cfg.base_url.as_deref(), Some("https://api.deepseek.com"));
                assert_eq!(cfg.model.as_deref(), Some("deepseek-chat"));
            }
            _ => panic!("key stage must send Configure"),
        }
        assert!(ui.wizard.is_none());
        // Transcript echo is masked.
        let echo = ui.messages.last().unwrap();
        assert!(!echo.text.contains("secret-1234"), "leaked: {}", echo.text);
    }

    #[test]
    fn wizard_custom_provider_asks_base_url() {
        let mut ui = Ui::new("m".into(), "w".into());
        ui.wizard = Some(Wizard::new());
        type_str(&mut ui, "custom");
        apply_key(&mut ui, key(KeyCode::Enter));
        assert_eq!(ui.wizard.as_ref().unwrap().stage, WizardStage::EnterBaseUrl);
        type_str(&mut ui, "https://llm.example.com/v1");
        apply_key(&mut ui, key(KeyCode::Enter));
        assert_eq!(ui.wizard.as_ref().unwrap().stage, WizardStage::EnterKey);
        type_str(&mut ui, "tok");
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::Configure(cfg)) => {
                assert_eq!(cfg.base_url.as_deref(), Some("https://llm.example.com/v1"));
                assert_eq!(cfg.model, None, "custom keeps the current model");
            }
            _ => panic!("must send Configure"),
        }
    }

    #[test]
    fn slash_model_opens_family_picker_and_selects() {
        let mut ui = Ui::new("deepseek-chat".into(), "w".into());
        ui.base_url = "https://api.deepseek.com".into();
        type_str(&mut ui, "/model");
        apply_key(&mut ui, key(KeyCode::Enter));
        let wizard = ui.wizard.as_ref().expect("model picker must open");
        assert_eq!(wizard.stage, WizardStage::PickModel);
        assert_eq!(wizard.models, model_candidates(&ui.base_url, ""));
        // ↓ selects the second candidate (deepseek-reasoner).
        apply_key(&mut ui, key(KeyCode::Down));
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::Configure(cfg)) => {
                assert_eq!(cfg.model.as_deref(), Some("deepseek-reasoner"));
                assert!(
                    cfg.provider.is_none(),
                    "model switch must not touch provider"
                );
            }
            _ => panic!("Enter must send Configure with the picked model"),
        }
        assert_eq!(ui.model, "deepseek-reasoner");
        assert!(ui.wizard.is_none());
        assert!(matches!(ui.messages.last().unwrap().kind, MsgKind::User));
    }

    #[test]
    fn model_picker_filters_and_free_text_on_empty() {
        // Filter to zero matches → Enter falls back to free-text entry.
        let mut ui = Ui::new("deepseek-chat".into(), "w".into());
        ui.base_url = "https://api.deepseek.com".into();
        type_str(&mut ui, "/model");
        apply_key(&mut ui, key(KeyCode::Enter));
        type_str(&mut ui, "zzz");
        apply_key(&mut ui, key(KeyCode::Enter));
        assert_eq!(ui.wizard.as_ref().unwrap().stage, WizardStage::EnterModel);
        // Type a custom model name and submit.
        type_str(&mut ui, "my-model");
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::Configure(cfg)) => {
                assert_eq!(cfg.model.as_deref(), Some("my-model"));
            }
            _ => panic!("free-text model must send Configure"),
        }
        assert_eq!(ui.model, "my-model");
    }

    #[test]
    fn model_family_detection() {
        assert!(family_models("https://api.deepseek.com").contains(&"deepseek-chat"));
        assert!(family_models("https://open.bigmodel.cn/api/paas/v4").contains(&"glm-4.5"));
        assert!(family_models("https://api.anthropic.com").contains(&"claude-sonnet-4-5"));
        // Unknown endpoints get no candidates → free-text picker.
        assert!(family_models("https://llm.corp.example").is_empty());
    }

    #[test]
    fn wizard_esc_cancels() {
        let mut ui = Ui::new("m".into(), "w".into());
        ui.wizard = Some(Wizard::new());
        type_str(&mut ui, "deep");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Esc)),
            Action::None
        ));
        assert!(ui.wizard.is_none());
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Esc)),
            Action::Quit
        ));
    }

    #[test]
    fn slash_key_with_token_sets_key_directly() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/key sk-secret-token-12345");
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::Configure(cfg)) => assert_eq!(cfg.api_key, "sk-secret-token-12345"),
            _ => panic!("/key <token> must configure"),
        }
        let echo = ui.messages.last().unwrap();
        assert!(matches!(echo.kind, MsgKind::User));
        assert!(
            !echo.text.contains("secret-token"),
            "token leaked: {}",
            echo.text
        );
        assert!(echo.text.ends_with("45"));
    }

    #[test]
    fn notice_and_done_settle_busy() {
        let mut ui = Ui::new("m".into(), "w".into());
        ui.busy = true;
        apply_msg(&mut ui, UiMsg::Notice("configured".into()));
        assert!(!ui.busy);
        ui.busy = true;
        apply_msg(&mut ui, UiMsg::Stream("tok".into()));
        assert!(ui.busy, "streaming must not settle the turn");
        apply_msg(&mut ui, UiMsg::Done("answer".into()));
        assert!(!ui.busy);
        assert!(ui.streaming.is_empty());
    }

    #[test]
    fn slash_clear_and_unknown() {
        let mut ui = Ui::new("m".into(), "w".into());
        ui.push(MsgKind::User, "hello".into());
        type_str(&mut ui, "/clear");
        apply_key(&mut ui, key(KeyCode::Enter));
        assert!(ui.messages.is_empty());
        type_str(&mut ui, "/bogus");
        apply_key(&mut ui, key(KeyCode::Enter));
        assert!(matches!(ui.messages[0].kind, MsgKind::Error));
    }

    #[test]
    fn wrap_respects_width() {
        let lines = wrap("aaa bbb ccc", 7);
        assert_eq!(lines, vec!["aaa bbb", "ccc"]);
    }
}
