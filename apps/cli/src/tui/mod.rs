mod app;
mod diff;
mod markdown;
mod ui;

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use zene_config::ZeneConfig;
use zene_core::{Agent, AgentEvent, EventHandler, PermissionGate, PermissionMode, PromptChoice, PromptOptions};

use app::{App, PermissionPrompt, RunState};
use crate::Cli;

enum UiMessage {
    AgentEvent(AgentEvent),
    Permission(PermissionPrompt),
    PromptFinished(Result<()>),
}

pub async fn run(agent: Agent, config: &ZeneConfig, cli: &Cli) -> Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alt screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let session_id = agent.session().meta.id.clone();
    let model = config.model.clone();
    let permission_mode = if cli.yolo {
        PermissionMode::Yolo
    } else {
        PermissionMode::parse(&config.permission_mode)
    };

    let mut app = App::new(session_id, model, permission_mode);
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiMessage>();

    let perm_ui_tx = ui_tx.clone();
    let permission_gate = PermissionGate::with_prompter(
        permission_mode,
        Box::new(move |tool_name, preview| {
            let (response_tx, response_rx) = std::sync::mpsc::channel();
            perm_ui_tx
                .send(UiMessage::Permission(PermissionPrompt {
                    tool_name: tool_name.to_string(),
                    args_preview: preview.to_string(),
                    response_tx,
                }))
                .map_err(|err| io::Error::new(io::ErrorKind::BrokenPipe, err))?;
            response_rx
                .recv()
                .map_err(|err| io::Error::new(io::ErrorKind::BrokenPipe, err))
        }),
    );

    let mut agent = agent;
    agent.set_permission_gate(permission_gate);

    let agent = Arc::new(AsyncMutex::new(agent));
    let mut active_cancel: Option<CancellationToken> = None;

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if app.permission.is_some() {
                    handle_permission_key(&mut app, key.code);
                    continue;
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(cancel) = active_cancel.take() {
                            cancel.cancel();
                        } else {
                            app.should_quit = true;
                        }
                    }
                    KeyCode::Esc => {
                        let now = Instant::now();
                        if app.last_esc.is_some_and(|t| now.duration_since(t) < Duration::from_millis(500))
                        {
                            app.should_quit = true;
                        } else {
                            app.last_esc = Some(now);
                        }
                    }
                    KeyCode::Enter if app.run_state == RunState::Idle => {
                        let input = app.input.trim().to_string();
                        if input.is_empty() {
                            continue;
                        }
                        app.input.clear();
                        app.lines.push(app::ChatLine::User(input.clone()));
                        app.scroll_to_bottom();

                        let cancel = CancellationToken::new();
                        active_cancel = Some(cancel.clone());

                        let agent = Arc::clone(&agent);
                        let stream = !cli.no_stream;

                        let tx_events = ui_tx.clone();
                        let tx_done = ui_tx.clone();

                        let event_handler: EventHandler = Arc::new(move |event| {
                            let _ = tx_events.send(UiMessage::AgentEvent(event));
                        });

                        tokio::spawn(async move {
                            let result = {
                                let mut agent = agent.lock().await;
                                agent
                                    .prompt(
                                        &input,
                                        PromptOptions {
                                            stream,
                                            cancel: Some(cancel),
                                            event_handler: Some(event_handler),
                                            quiet: true,
                                        },
                                    )
                                    .await
                                    .map(|_| ())
                            };
                            let _ = tx_done.send(UiMessage::PromptFinished(result));
                        });
                    }
                    KeyCode::Char(c) if app.run_state == RunState::Idle => {
                        app.input.push(c);
                    }
                    KeyCode::Backspace if app.run_state == RunState::Idle => {
                        app.input.pop();
                    }
                    KeyCode::PageUp => {
                        app.scroll_page_up();
                    }
                    KeyCode::PageDown => {
                        app.scroll_page_down(app.max_scroll);
                    }
                    _ => {}
                }
            }
        }

        while let Ok(msg) = ui_rx.try_recv() {
            match msg {
                UiMessage::AgentEvent(event) => app.handle_agent_event(event),
                UiMessage::Permission(prompt) => app.permission = Some(prompt),
                UiMessage::PromptFinished(res) => {
                    active_cancel = None;
                    if let Err(err) = res {
                        app.lines.push(app::ChatLine::Error(err.to_string()));
                        app.run_state = RunState::Idle;
                    }
                    if !cli.quiet_usage {
                        let usage = agent.lock().await.turn_usage().clone();
                        app.update_usage(&usage);
                    }
                    app.scroll_to_bottom();
                }
            }
        }
    }

    disable_raw_mode().context("disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).context("leave alt screen")?;
    terminal.show_cursor().context("show cursor")?;

    let mut agent = agent.lock().await;
    agent.shutdown().await.context("agent shutdown")?;

    Ok(())
}

fn handle_permission_key(app: &mut App, code: KeyCode) {
    let Some(prompt) = app.permission.take() else {
        return;
    };
    let choice = match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => PromptChoice::AllowOnce,
        KeyCode::Char('a') | KeyCode::Char('A') => PromptChoice::AllowSession,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => PromptChoice::Deny,
        _ => {
            app.permission = Some(prompt);
            return;
        }
    };
    let _ = prompt.response_tx.send(choice);
}

#[cfg(test)]
mod tests {
    use super::app::format_tool_summary;

    #[test]
    fn tool_summary_extracts_path() {
        assert_eq!(
            format_tool_summary("Read", r#"{"path":"src/main.rs"}"#),
            "src/main.rs"
        );
    }
}
