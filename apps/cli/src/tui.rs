//! `zene tui` — minimal terminal chat over the same agent core as `zene acp`.
//!
//! Deliberately small: transcript pane + input line. The agent runs in a
//! spawned task and talks to the UI over channels so the terminal stays
//! responsive during long turns.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc;
use zene_config::ZeneConfig;
use zene_core::{Agent, PromptOptions};

pub(crate) async fn run(workdir: &Path) -> Result<()> {
    let config = ZeneConfig::load(workdir).map_err(|err| anyhow!(err.to_string()))?;
    let (prompt_tx, prompt_rx) = mpsc::unbounded_channel::<String>();
    let (resp_tx, mut resp_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(agent_loop(
        workdir.to_path_buf(),
        config,
        prompt_rx,
        resp_tx,
    ));

    let terminal = ratatui::init();
    let result = ui_loop(terminal, &prompt_tx, &mut resp_rx);
    ratatui::restore();
    result
}

/// Owns the agent across turns (session continuity lives here).
async fn agent_loop(
    workdir: PathBuf,
    config: ZeneConfig,
    mut prompts: mpsc::UnboundedReceiver<String>,
    responses: mpsc::UnboundedSender<String>,
) {
    let mut agent = match Agent::builder(&workdir)
        .config(config)
        .core_tools()
        .build()
        .await
    {
        Ok(agent) => agent,
        Err(err) => {
            let _ = responses.send(format!("[agent setup failed] {err:#}"));
            return;
        }
    };
    let _ = responses.send("[ready]".to_string());
    while let Some(prompt) = prompts.recv().await {
        let reply = agent
            .prompt(
                &prompt,
                PromptOptions {
                    quiet: true,
                    ..Default::default()
                },
            )
            .await;
        let text = match reply {
            Ok(text) => text,
            Err(err) => format!("[turn failed] {err:#}"),
        };
        if responses.send(text).is_err() {
            break;
        }
    }
}

#[derive(Default)]
struct Ui {
    transcript: Vec<String>,
    input: String,
    scroll: u16,
    follow: bool,
    busy: bool,
}

enum Action {
    Quit,
    Send(String),
    None,
}

/// Terminal state transition for one key press (pure; the IO loop below only
/// dispatches).
fn apply_key(ui: &mut Ui, key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }
    match key.code {
        KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Enter if !ui.input.trim().is_empty() && !ui.busy => {
            let prompt = std::mem::take(&mut ui.input);
            ui.transcript.push(format!("> {prompt}"));
            ui.busy = true;
            ui.follow = true;
            Action::Send(prompt)
        }
        KeyCode::Backspace => {
            ui.input.pop();
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

fn ui_loop(
    mut terminal: DefaultTerminal,
    prompts: &mpsc::UnboundedSender<String>,
    responses: &mut mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let mut ui = Ui {
        follow: true,
        ..Default::default()
    };
    loop {
        while let Ok(text) = responses.try_recv() {
            ui.transcript.push(text);
            ui.busy = false;
            ui.follow = true;
        }
        // ponytail: blocking poll on the async runtime thread; split into a
        // task + select if the agent ever needs same-thread concurrency.
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match apply_key(&mut ui, key) {
                    Action::Quit => return Ok(()),
                    Action::Send(prompt) => prompts.send(prompt).context("agent task is gone")?,
                    Action::None => {}
                }
            }
        }
        terminal.draw(|frame| draw(frame, &ui))?;
    }
}

fn draw(frame: &mut Frame, ui: &Ui) {
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).split(frame.area());
    let text = ui.transcript.join("\n");
    let inner_height = chunks[0].height.saturating_sub(2) as usize;
    let max_scroll = (text.lines().count().max(1)).saturating_sub(inner_height) as u16;
    let scroll = if ui.follow {
        max_scroll
    } else {
        ui.scroll.min(max_scroll)
    };
    let title = if ui.busy {
        "zene (working…)"
    } else {
        "zene (Enter send · PgUp/PgDn scroll · Esc quit)"
    };
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(Block::default().borders(Borders::ALL).title(title)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(ui.input.as_str())
            .block(Block::default().borders(Borders::ALL).title("input")),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_sends_and_clears_input() {
        let mut ui = Ui::default();
        for c in "hi".chars() {
            apply_key(&mut ui, key(KeyCode::Char(c)));
        }
        match apply_key(&mut ui, key(KeyCode::Enter)) {
            Action::Send(prompt) => assert_eq!(prompt, "hi"),
            _ => panic!("Enter must send"),
        }
        assert!(ui.input.is_empty());
        assert!(ui.busy);
        // Enter while busy must not send again.
        assert!(matches!(
            apply_key(&mut ui, key(KeyCode::Enter)),
            Action::None
        ));
    }

    #[test]
    fn backspace_edits_and_ctrl_c_quits() {
        let mut ui = Ui::default();
        apply_key(&mut ui, key(KeyCode::Char('a')));
        apply_key(&mut ui, key(KeyCode::Backspace));
        assert!(ui.input.is_empty());
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(apply_key(&mut ui, ctrl_c), Action::Quit));
    }
}
