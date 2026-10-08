//! `zene tui` — full terminal chat over the same agent core as `zene acp`.
//!
//! Chat-style transcript with streaming output, a Claude-Code-style popup
//! command menu (type `/` to float candidates, ↑↓ select, Tab/Enter complete,
//! Esc close), and tool activity lines. The agent runs in a spawned task and
//! talks to the UI over channels so the terminal stays responsive.

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
    /// True when the command takes an argument (menu completion fills a space).
    takes_arg: bool,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "key",
        hint: "set the API key for this session",
        takes_arg: true,
    },
    Command {
        name: "model",
        hint: "show current model",
        takes_arg: true,
    },
    Command {
        name: "help",
        hint: "list commands",
        takes_arg: false,
    },
    Command {
        name: "clear",
        hint: "clear transcript",
        takes_arg: false,
    },
    Command {
        name: "quit",
        hint: "exit",
        takes_arg: false,
    },
];

pub(crate) async fn run(workdir: &Path) -> Result<()> {
    let config = ZeneConfig::load(workdir).map_err(|err| anyhow!(err.to_string()))?;
    let model = config.model.clone();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<UiMsg>();
    tokio::spawn(agent_loop(workdir.to_path_buf(), config, cmd_rx, msg_tx));

    let mut ui = Ui::new(model, workdir.display().to_string());
    let terminal = ratatui::init();
    let result = ui_loop(terminal, &mut ui, &cmd_tx, &mut msg_rx);
    ratatui::restore();
    let _ = cmd_tx.send(Cmd::Quit);
    result
}

/// Messages into the agent task.
enum Cmd {
    Prompt(String),
    SwitchModel(String),
    SetKey(String),
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
            Cmd::SwitchModel(model) => match agent.switch_model(&model, None, None, None).await {
                Ok(()) => {
                    let _ = tx.send(UiMsg::Notice(format!("model → {model}")));
                }
                Err(err) => {
                    let _ = tx.send(UiMsg::Error(format!("model switch failed: {err:#}")));
                }
            },
            Cmd::SetKey(key) => {
                let model = agent.config().model.clone();
                match agent.switch_model(&model, None, None, Some(key)).await {
                    Ok(()) => {
                        let _ = tx.send(UiMsg::Notice(
                            "api key set for this session — persist it in ~/.zene/config.toml (api_key) or DEEPSEEK_API_KEY"
                                .into(),
                        ));
                    }
                    Err(err) => {
                        let _ = tx.send(UiMsg::Error(format!("api key rejected: {err:#}")));
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

struct Ui {
    messages: Vec<Msg>,
    streaming: String,
    input: String,
    scroll: u16,
    follow: bool,
    busy: bool,
    model: String,
    workdir: String,
    /// Popup menu selection index.
    menu_sel: usize,
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
    // Popup navigation first: the menu owns arrows/Tab/Enter/Esc while open.
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
    if cmd.takes_arg {
        ui.input = format!("/{} ", cmd.name);
        ui.menu_sel = 0;
        Action::None
    } else {
        let input = format!("/{}", cmd.name);
        ui.input.clear();
        ui.menu_sel = 0;
        slash_command(ui, &input).unwrap_or(Action::None)
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
                .map(|c| {
                    if c.takes_arg {
                        format!("/{} <arg>  — {}", c.name, c.hint)
                    } else {
                        format!("/{}  — {}", c.name, c.hint)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            ui.push(MsgKind::Notice, help);
            Some(Action::None)
        }
        "model" if arg.is_empty() => {
            ui.push(MsgKind::Notice, format!("model: {}", ui.model));
            Some(Action::None)
        }
        "model" => {
            ui.push(MsgKind::User, input.to_string());
            ui.model = arg.to_string();
            ui.busy = true;
            Some(Action::Send(Cmd::SwitchModel(arg.to_string())))
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
            Some(Action::Send(Cmd::SetKey(arg.to_string())))
        }
        "key" | "api-key" => {
            ui.push(MsgKind::Notice, "usage: /key <token>".into());
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
        // Notice is also the completion signal for SetKey/SwitchModel
        // commands (and startup "ready"): always settle busy.
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

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::new().fg(Color::Green)),
            Span::raw(ui.input.as_str()),
            Span::styled("▌", Style::new().fg(Color::Green)),
        ])),
        chunks[2],
    );

    let hints = if menu_query(&ui.input).is_some() {
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
            let arg = if cmd.takes_arg { "<arg>" } else { "" };
            Line::from(Span::styled(
                format!(" /{:<6}{:<7} {}", cmd.name, arg, cmd.hint),
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
    fn menu_enter_fills_arg_commands_and_runs_plain_ones() {
        // /model takes an arg: Enter fills "/model " instead of executing.
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/model");
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::None
        ));
        assert_eq!(ui.input, "/model ");
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
    fn slash_key_masks_token_and_sends() {
        let mut ui = Ui::new("m".into(), "w".into());
        type_str(&mut ui, "/key sk-secret-token-12345");
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(Cmd::SetKey(k)) => assert_eq!(k, "sk-secret-token-12345"),
            _ => panic!("/key must send the key"),
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
    fn notice_and_done_settle_busy() {
        let mut ui = Ui::new("m".into(), "w".into());
        ui.busy = true;
        apply_msg(&mut ui, UiMsg::Notice("api key set".into()));
        assert!(!ui.busy);
        ui.busy = true;
        apply_msg(&mut ui, UiMsg::Stream("tok".into()));
        assert!(ui.busy, "streaming must not settle the turn");
        apply_msg(&mut ui, UiMsg::Done("answer".into()));
        assert!(!ui.busy);
        assert!(ui.streaming.is_empty());
    }

    #[test]
    fn wrap_respects_width() {
        let lines = wrap("aaa bbb ccc", 7);
        assert_eq!(lines, vec!["aaa bbb", "ccc"]);
    }
}
