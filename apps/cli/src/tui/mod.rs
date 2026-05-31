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
    ModelSwitchFinished(Result<String>),
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

                if app.model_selector.is_some() {
                    handle_model_selector_key(&mut app, &agent, key.code, ui_tx.clone()).await;
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

                        // Slash command handling
                        if input == "/exit" || input == "/quit" {
                            app.should_quit = true;
                            continue;
                        }
                        if input == "/models" || input == "/model" {
                            app.model_selector = Some(app::ModelSelector {
                                selected_index: 0,
                                input_key: None,
                            });
                            continue;
                        }
                        if let Some(args_str) = input.strip_prefix("/model ") {
                            let args_str = args_str.trim().to_string();
                            let parts: Vec<&str> = args_str.split_whitespace().collect();
                            if !parts.is_empty() {
                                let raw_model = parts[0];
                                let mut model = raw_model.to_string();
                                let mut provider = parts.get(1).map(|s| s.to_string());
                                let mut base_url = parts.get(2).map(|s| s.to_string());
                                let mut api_key = parts.get(3).map(|s| s.to_string());

                                // Resolve presets
                                if raw_model == "deepseek-chat" {
                                    provider = Some("openai".to_string());
                                    base_url = Some("https://api.deepseek.com".to_string());
                                } else if raw_model == "gpt-4o" {
                                    provider = Some("openai".to_string());
                                    base_url = Some("https://api.openai.com/v1".to_string());
                                } else if raw_model == "claude-3-5-sonnet" || raw_model == "claude-3-5-sonnet-20241022" {
                                    model = "claude-3-5-sonnet-20241022".to_string();
                                    provider = Some("anthropic".to_string());
                                    base_url = Some("https://api.anthropic.com".to_string());
                                } else if let Some(ollama_model) = raw_model.strip_prefix("ollama/") {
                                    model = ollama_model.to_string();
                                    provider = Some("openai".to_string());
                                    base_url = Some("http://localhost:11434/v1".to_string());
                                    if api_key.is_none() {
                                        api_key = Some("ollama".to_string()); // dummy key
                                    }
                                }

                                let has_key = {
                                    let agent_guard = agent.lock().await;
                                    let config = agent_guard.config();
                                    let provider_name = provider.clone().unwrap_or_else(|| {
                                        config.provider.clone()
                                    });
                                    api_key.is_some() || {
                                        if provider_name.trim().to_lowercase() == "anthropic" {
                                            config.anthropic_api_key.as_deref().map_or(false, |k| !k.is_empty()) || std::env::var("ANTHROPIC_API_KEY").is_ok()
                                        } else {
                                            config.api_key.as_deref().map_or(false, |k| !k.is_empty()) || 
                                                ["DEEPSEEK_API_KEY", "ZENE_API_KEY", "OPENAI_API_KEY"].iter().any(|var| std::env::var(var).is_ok())
                                        }
                                    }
                                };

                                let is_ollama = raw_model.starts_with("ollama/");

                                if has_key || is_ollama {
                                    let agent_clone = Arc::clone(&agent);
                                    let mut agent_guard = agent_clone.lock().await;
                                    match agent_guard.switch_model(&model, provider, base_url, api_key).await {
                                        Ok(_) => {
                                            app.model = agent_guard.config().model.clone();
                                            app.lines.push(app::ChatLine::Assistant(format!(
                                                "Successfully switched to model: {}",
                                                app.model
                                            )));
                                        }
                                        Err(err) => {
                                            app.lines.push(app::ChatLine::Error(format!(
                                                "Error switching model: {err:#}"
                                            )));
                                        }
                                    }
                                } else {
                                    let selected_index = app::MODEL_PRESETS
                                        .iter()
                                        .position(|p| p.model_name == model)
                                        .unwrap_or(0);

                                    app.model_selector = Some(app::ModelSelector {
                                        selected_index,
                                        input_key: Some(String::new()),
                                    });
                                }
                            }
                            app.scroll_to_bottom();
                            continue;
                        }
                        if input == "/plan" {
                            agent.lock().await.enter_plan_mode();
                            app.lines.push(app::ChatLine::Assistant(
                                "Entered plan mode (read-only tools + ExitPlanMode).".to_string()
                            ));
                            app.scroll_to_bottom();
                            continue;
                        }

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
                UiMessage::ModelSwitchFinished(res) => {
                    match res {
                        Ok(new_model) => {
                            app.model = new_model;
                            app.lines.push(app::ChatLine::Assistant(format!(
                                "Successfully switched to model and configured API key: {}",
                                app.model
                            )));
                        }
                        Err(err) => {
                            app.lines.push(app::ChatLine::Error(format!(
                                "Error configuring model and API key: {err:#}"
                            )));
                        }
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

async fn handle_model_selector_key(
    app: &mut App,
    agent: &Arc<AsyncMutex<Agent>>,
    code: KeyCode,
    ui_tx: std::sync::mpsc::Sender<UiMessage>,
) {
    let Some(ref mut selector) = app.model_selector else {
        return;
    };

    if let Some(ref mut input_key) = selector.input_key {
        match code {
            KeyCode::Char(c) => {
                input_key.push(c);
            }
            KeyCode::Backspace => {
                input_key.pop();
            }
            KeyCode::Esc => {
                selector.input_key = None;
            }
            KeyCode::Enter => {
                let preset = &app::MODEL_PRESETS[selector.selected_index];
                let model = preset.model_name.to_string();
                let provider = preset.provider.to_string();
                let base_url = preset.base_url.to_string();
                let key = input_key.clone();

                app.model_selector = None;

                let agent = Arc::clone(agent);
                let tx_done = ui_tx;
                tokio::spawn(async move {
                    let mut agent_guard = agent.lock().await;
                    let res = agent_guard
                        .switch_model(
                            &model,
                            Some(provider),
                            Some(base_url),
                            Some(key),
                        )
                        .await;
                    let msg = res.map(|_| agent_guard.config().model.clone());
                    let _ = tx_done.send(UiMessage::ModelSwitchFinished(msg));
                });
            }
            _ => {}
        }
    } else {
        match code {
            KeyCode::Up => {
                let len = app::MODEL_PRESETS.len();
                selector.selected_index = (selector.selected_index + len - 1) % len;
            }
            KeyCode::Down => {
                let len = app::MODEL_PRESETS.len();
                selector.selected_index = (selector.selected_index + 1) % len;
            }
            KeyCode::Esc => {
                app.model_selector = None;
                app.lines.push(app::ChatLine::Assistant("Model selection cancelled.".to_string()));
                app.scroll_to_bottom();
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                let preset = &app::MODEL_PRESETS[selector.selected_index];
                if preset.requires_key {
                    selector.input_key = Some(String::new());
                }
            }
            KeyCode::Enter => {
                let preset = &app::MODEL_PRESETS[selector.selected_index];
                if preset.requires_key {
                    let has_key = {
                        let agent_guard = agent.lock().await;
                        let config = agent_guard.config();
                        let provider_lower = preset.provider.trim().to_lowercase();
                        if provider_lower == "anthropic" {
                            config.anthropic_api_key.as_deref().map_or(false, |k| !k.is_empty()) || std::env::var("ANTHROPIC_API_KEY").is_ok()
                        } else {
                            config.api_key.as_deref().map_or(false, |k| !k.is_empty()) || 
                                ["DEEPSEEK_API_KEY", "ZENE_API_KEY", "OPENAI_API_KEY"].iter().any(|var| std::env::var(var).is_ok())
                        }
                    };

                    if has_key {
                        let model = preset.model_name.to_string();
                        let provider = preset.provider.to_string();
                        let base_url = preset.base_url.to_string();

                        app.model_selector = None;

                        let agent = Arc::clone(agent);
                        let tx_done = ui_tx;
                        tokio::spawn(async move {
                            let mut agent_guard = agent.lock().await;
                            let res = agent_guard
                                .switch_model(
                                    &model,
                                    Some(provider),
                                    Some(base_url),
                                    None,
                                )
                                .await;
                            let msg = res.map(|_| agent_guard.config().model.clone());
                            let _ = tx_done.send(UiMessage::ModelSwitchFinished(msg));
                        });
                    } else {
                        selector.input_key = Some(String::new());
                    }
                } else {
                    let model = preset.model_name.to_string();
                    let provider = preset.provider.to_string();
                    let base_url = preset.base_url.to_string();

                    app.model_selector = None;

                    let agent = Arc::clone(agent);
                    let tx_done = ui_tx;
                    tokio::spawn(async move {
                        let mut agent_guard = agent.lock().await;
                        let res = agent_guard
                            .switch_model(
                                &model,
                                Some(provider),
                                Some(base_url),
                                Some("ollama".to_string()),
                            )
                            .await;
                        let msg = res.map(|_| agent_guard.config().model.clone());
                        let _ = tx_done.send(UiMessage::ModelSwitchFinished(msg));
                    });
                }
            }
            _ => {}
        }
    }
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
