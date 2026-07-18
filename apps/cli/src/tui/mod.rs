mod app;
mod diff;
mod input_line;
mod markdown;
mod ui;

use crate::model_config;
use crate::Cli;

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::sync::mpsc;
use tokio::runtime::Runtime;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use zene_config::ZeneConfig;
use zene_core::{Agent, AgentEvent, EventHandler, PermissionGate, PermissionMode, PromptChoice, PromptOptions, SteerBuffer};

use app::{App, ModelSelectorFlow, ModelSelectorMode, PermissionPrompt, RunState};

enum UiMessage {
    AgentEvent(AgentEvent),
    Permission(PermissionPrompt),
    ModelSwitchFinished(Result<String>),
}

pub async fn run(agent: Agent, config: &ZeneConfig, cli: &Cli) -> Result<()> {
    let config = config.clone();
    let no_stream = cli.no_stream;
    let yolo = cli.yolo;
    let quiet_usage = cli.quiet_usage;
    tokio::task::spawn_blocking(move || {
        run_tui_sync(agent, &config, no_stream, yolo, quiet_usage)
    })
    .await
    .context("tui thread join")?
}

/// Runs on a dedicated OS thread so crossterm I/O does not starve the agent tokio runtime.
fn run_tui_sync(
    agent: Agent,
    config: &ZeneConfig,
    no_stream: bool,
    yolo: bool,
    quiet_usage: bool,
) -> Result<()> {
    let agent_rt = Runtime::new().context("create agent tokio runtime")?;

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context("enter alt screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let session_id = agent.session().meta.id.clone();
    let model = config.model.clone();
    let permission_mode = if yolo {
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
            // Avoid blocking a tokio worker while the TUI main loop processes the prompt.
            std::thread::spawn(move || response_rx.recv())
                .join()
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "permission thread panicked"))?
                .map_err(|err| io::Error::new(io::ErrorKind::BrokenPipe, err))
        }),
    );

    let mut agent = agent;
    agent.set_permission_gate(permission_gate);
    let steer_buffer = agent.steer_buffer();

    let agent = Arc::new(AsyncMutex::new(agent));
    let mut active_cancel: Option<CancellationToken> = None;
    let mut prompt_done_rx: Option<mpsc::Receiver<Result<String>>> = None;

    loop {
        app.tick();
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if app.should_quit {
            break;
        }

        while let Ok(msg) = ui_rx.try_recv() {
            match msg {
                UiMessage::AgentEvent(event) => app.handle_agent_event(event),
                UiMessage::Permission(prompt) => {
                    app.lines.push(app::ChatLine::Status(format!(
                        "Approve {}? [y] once  [a] session  [n] deny",
                        prompt.tool_name
                    )));
                    app.permission = Some(prompt);
                    app.scroll_to_bottom();
                }
                UiMessage::ModelSwitchFinished(res) => {
                    match res {
                        Ok(new_model) => {
                            app.model = new_model;
                            app.lines.push(app::ChatLine::Assistant(format!(
                                "Successfully switched to model (saved): {}",
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

        if let Some(rx) = prompt_done_rx.as_ref() {
            if let Ok(result) = rx.try_recv() {
                prompt_done_rx = None;
                active_cancel = None;
                match result {
                    Ok(text) => {
                        app.push_final_assistant(&text);
                        app.lines
                            .push(app::ChatLine::Status("Turn finished.".to_string()));
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        if msg.contains("aborted") {
                            app.mark_pending_tools_cancelled();
                            app.lines
                                .push(app::ChatLine::Status("Turn cancelled.".to_string()));
                        } else if !app.lines.iter().any(|line| {
                            matches!(line, app::ChatLine::Error(s) if s == &msg)
                        }) {
                            app.lines.push(app::ChatLine::Error(msg));
                        }
                    }
                }
                app.finish_turn();
                if !quiet_usage {
                    let usage = agent_rt
                        .block_on(async { *agent.lock().await.turn_usage() });
                    app.update_usage(&usage);
                }
                app.scroll_to_bottom();
                try_start_next_pending(
                    &mut app,
                    &agent_rt,
                    &agent,
                    no_stream,
                    &ui_tx,
                    &mut active_cancel,
                    &mut prompt_done_rx,
                );
            }
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Paste(text)
                    if app.permission.is_none() && app.model_selector.is_none() =>
                {
                    app.input.insert_str(&text);
                }
                Event::Mouse(mouse)
                    if app.model_selector.is_none() && app.mouse_scroll_enabled =>
                {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.scroll_lines_up(3),
                        MouseEventKind::ScrollDown => {
                            app.scroll_lines_down(3, app.max_scroll)
                        }
                        _ => {}
                    }
                }
                Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    continue;
                }

                if app.permission.is_some() {
                    handle_permission_key(&mut app, key.code);
                    continue;
                }

                if app.model_selector.is_some() {
                    handle_model_selector_key(
                        &mut app,
                        &agent_rt,
                        &agent,
                        key.code,
                        ui_tx.clone(),
                    );
                    continue;
                }

                match key.code {
                    KeyCode::Char('m')
                        if key
                            .modifiers
                            .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                    {
                        let enabled = app.toggle_mouse_scroll();
                        sync_mouse_capture(terminal.backend_mut(), enabled)?;
                        let msg = if enabled {
                            "Mouse scroll enabled (chat wheel scroll on)."
                        } else {
                            "Mouse scroll disabled — Shift+drag to select terminal text."
                        };
                        app.lines.push(app::ChatLine::Status(msg.to_string()));
                        app.scroll_to_bottom();
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if cancel_turn(&mut app, &mut active_cancel) {
                            // turn cancel requested
                        } else {
                            app.should_quit = true;
                        }
                    }
                    KeyCode::Esc if app.run_state == RunState::Running
                        || app.run_state == RunState::Cancelling =>
                    {
                        cancel_turn(&mut app, &mut active_cancel);
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
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                        if matches!(
                            app.run_state,
                            RunState::Idle | RunState::Running | RunState::Cancelling
                        ) {
                            app.input.insert_newline();
                        }
                    }
                    KeyCode::Enter => {
                        if app.run_state == RunState::Running
                            || app.run_state == RunState::Cancelling
                        {
                            handle_running_enter(&mut app, &steer_buffer, &mut active_cancel);
                            continue;
                        }
                        let input = app.input.trimmed().to_string();
                        if input.is_empty() {
                            continue;
                        }
                        app.push_history(&input);
                        app.input.clear();
                        app.lines.push(app::ChatLine::User(input.clone()));
                        app.scroll_to_bottom();

                        // Slash command handling
                        if input == "/exit" || input == "/quit" {
                            app.should_quit = true;
                            continue;
                        }
                        if input == "/models" || input == "/model" {
                            let agent_guard = agent_rt.block_on(async { agent.lock().await });
                            open_model_picker(&mut app, agent_guard.config());
                            continue;
                        }
                        if input == "/provider" || input == "/providers" {
                            let agent_guard = agent_rt.block_on(async { agent.lock().await });
                            app.model_selector =
                                Some(app::ModelSelector::for_providers(agent_guard.config()));
                            continue;
                        }
                        if let Some(args_str) = input.strip_prefix("/model ") {
                            let args_str = args_str.trim();
                            if args_str.is_empty() {
                                let agent_guard =
                                    agent_rt.block_on(async { agent.lock().await });
                                open_model_picker(&mut app, agent_guard.config());
                                continue;
                            }
                            let parts: Vec<&str> = args_str.split_whitespace().collect();
                            let resolved = model_config::resolve_model_args(parts[0], &parts);
                                let agent_guard =
                                    agent_rt.block_on(async { agent.lock().await });
                            let base_url = resolved
                                .base_url
                                .clone()
                                .unwrap_or_else(|| agent_guard.config().base_url.clone());
                            let has_key = resolved.api_key.is_some()
                                || model_config::has_api_key_for_openai_provider(
                                    agent_guard.config(),
                                    &base_url,
                                )
                                || !model_config::model_requires_key(&resolved.model);
                            drop(agent_guard);

                                if has_key {
                                    let agent_clone = Arc::clone(&agent);
                                    match agent_rt.block_on(async {
                                        let mut agent_guard = agent_clone.lock().await;
                                        agent_guard
                                            .switch_model(
                                                &resolved.model,
                                                resolved.provider,
                                                resolved.base_url,
                                                resolved.api_key,
                                            )
                                            .await
                                    }) {
                                        Ok(_) => {
                                            app.model = agent_rt.block_on(async {
                                                agent_clone.lock().await.config().model.clone()
                                            });
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
                                app.model_selector = Some(app::ModelSelector::for_model_with_api_key(
                                    &resolved.model,
                                ));
                            }
                            app.scroll_to_bottom();
                            continue;
                        }
                        if input == "/plan" {
                            agent_rt.block_on(async {
                                agent.lock().await.enter_plan_mode();
                            });
                            app.lines.push(app::ChatLine::Assistant(
                                "Entered plan mode (read-only tools + ExitPlanMode).".to_string()
                            ));
                            app.scroll_to_bottom();
                            continue;
                        }

                        let cancel = CancellationToken::new();
                        start_agent_turn(
                            &mut app,
                            &agent_rt,
                            &agent,
                            no_stream,
                            &ui_tx,
                            &mut active_cancel,
                            &mut prompt_done_rx,
                            input,
                            cancel,
                        );
                    }
                    KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.scroll_to_top();
                    }
                    KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.scroll_to_bottom();
                    }
                    KeyCode::Home => {
                        app.input.move_home_wrapped(app.input_wrap_width);
                    }
                    KeyCode::End => {
                        app.input.move_end_wrapped(app.input_wrap_width);
                    }
                    KeyCode::Char(_)
                    | KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::Left
                    | KeyCode::Right => {
                        app.input.handle_key(key.code, key.modifiers);
                    }
                    KeyCode::Up => {
                        if !app.input.move_up_wrapped(app.input_wrap_width) {
                            app.history_prev();
                        }
                    }
                    KeyCode::Down => {
                        if !app.input.move_down_wrapped(app.input_wrap_width) {
                            app.history_next();
                        }
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
                _ => {}
            }
        }
    }

    disable_raw_mode().context("disable raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context("leave alt screen")?;
    terminal.show_cursor().context("show cursor")?;

    agent_rt
        .block_on(async {
            let mut agent = agent.lock().await;
            agent.shutdown().await
        })
        .context("agent shutdown")?;

    Ok(())
}

fn sync_mouse_capture(
    stdout: &mut impl io::Write,
    enabled: bool,
) -> Result<()> {
    if enabled {
        execute!(stdout, EnableMouseCapture).context("enable mouse capture")?;
    } else {
        execute!(stdout, DisableMouseCapture).context("disable mouse capture")?;
    }
    Ok(())
}

fn start_agent_turn(
    app: &mut App,
    agent_rt: &Runtime,
    agent: &Arc<AsyncMutex<Agent>>,
    no_stream: bool,
    ui_tx: &mpsc::Sender<UiMessage>,
    active_cancel: &mut Option<CancellationToken>,
    prompt_done_rx: &mut Option<mpsc::Receiver<Result<String>>>,
    input: String,
    cancel: CancellationToken,
) {
    *active_cancel = Some(cancel.clone());
    app.run_state = RunState::Running;
    app.activity = "Starting…".to_string();

    let stream = !no_stream;
    let tx_events = ui_tx.clone();
    let event_handler: EventHandler = Arc::new(move |event| {
        let _ = tx_events.send(UiMessage::AgentEvent(event));
    });

    *prompt_done_rx = Some(spawn_prompt(
        agent_rt,
        Arc::clone(agent),
        input,
        stream,
        cancel,
        event_handler,
    ));
}

fn try_start_next_pending(
    app: &mut App,
    agent_rt: &Runtime,
    agent: &Arc<AsyncMutex<Agent>>,
    no_stream: bool,
    ui_tx: &mpsc::Sender<UiMessage>,
    active_cancel: &mut Option<CancellationToken>,
    prompt_done_rx: &mut Option<mpsc::Receiver<Result<String>>>,
) {
    if app.run_state != RunState::Idle || prompt_done_rx.is_some() {
        return;
    }
    while let Some(input) = app.pending_prompts.pop_front() {
        if input.trim().is_empty() {
            continue;
        }
        let remaining = app.pending_prompts.len();
        let status = if remaining == 0 {
            "Starting queued prompt…".to_string()
        } else {
            format!("Starting queued prompt ({remaining} more waiting)…")
        };
        app.lines.push(app::ChatLine::Status(status));
        app.scroll_to_bottom();
        let cancel = CancellationToken::new();
        start_agent_turn(
            app,
            agent_rt,
            agent,
            no_stream,
            ui_tx,
            active_cancel,
            prompt_done_rx,
            input,
            cancel,
        );
        return;
    }
}

fn spawn_prompt(
    agent_rt: &Runtime,
    agent: Arc<AsyncMutex<Agent>>,
    input: String,
    stream: bool,
    cancel: CancellationToken,
    event_handler: EventHandler,
) -> mpsc::Receiver<Result<String>> {
    let (tx, rx) = mpsc::channel();
    agent_rt.spawn(async move {
        let result = async {
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
        }
        .await;
        let _ = tx.send(result);
    });
    rx
}

fn cancel_turn(app: &mut App, active_cancel: &mut Option<CancellationToken>) -> bool {
    if matches!(
        app.run_state,
        RunState::Running | RunState::Cancelling
    ) {
        if let Some(cancel) = active_cancel.as_ref() {
            cancel.cancel();
            app.request_cancel();
            app.activity = "Cancelling…".to_string();
            app.lines
                .push(app::ChatLine::Status("Cancelling current turn… (Esc/Ctrl+C again)".to_string()));
            app.scroll_to_bottom();
            return true;
        }
    }
    false
}

fn handle_running_enter(
    app: &mut App,
    steer_buffer: &Arc<Mutex<SteerBuffer>>,
    active_cancel: &mut Option<CancellationToken>,
) {
    let input = app.input.trimmed().to_string();
    if input.is_empty() {
        return;
    }
    app.push_history(&input);
    app.input.clear();

    if input == "/cancel" {
        if cancel_turn(app, active_cancel) {
            return;
        }
        app.lines
            .push(app::ChatLine::Status("No turn in progress.".to_string()));
        app.scroll_to_bottom();
        return;
    }

    if let Some(msg) = input.strip_prefix("/steer ") {
        let msg = msg.trim();
        if msg.is_empty() {
            app.lines.push(app::ChatLine::Status(
                "Usage: /steer <message>".to_string(),
            ));
            app.scroll_to_bottom();
            return;
        }
        let mut buffer = steer_buffer.lock();
        buffer.push(msg.to_string());
        app.lines.push(app::ChatLine::Status(format!(
            "Steer queued: {msg}"
        )));
        app.scroll_to_bottom();
        return;
    }

    if input.starts_with('/') {
        app.lines.push(app::ChatLine::Status(
            "Commands unavailable while running — Esc to cancel, or queue plain text with Enter."
                .to_string(),
        ));
        app.scroll_to_bottom();
        return;
    }

    app.lines.push(app::ChatLine::User(input.clone()));
    let pending = app.queue_prompt(input.clone());
    let preview = preview_text(&input);
    app.lines.push(app::ChatLine::Status(format!(
        "Queued ({pending} pending): {preview}"
    )));
    app.scroll_to_bottom();
}

fn preview_text(text: &str) -> String {
    let one_line = text.lines().next().unwrap_or(text);
    if one_line.chars().count() <= 60 {
        one_line.to_string()
    } else {
        format!("{}…", one_line.chars().take(59).collect::<String>())
    }
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

fn open_model_picker(app: &mut App, config: &ZeneConfig) {
    app.model_selector = Some(app::ModelSelector::for_setup(config));
}

fn handle_model_selector_key(
    app: &mut App,
    agent_rt: &Runtime,
    agent: &Arc<AsyncMutex<Agent>>,
    code: KeyCode,
    ui_tx: std::sync::mpsc::Sender<UiMessage>,
) {
    let Some(ref mut selector) = app.model_selector else {
        return;
    };

    if let Some(ref mut input_model) = selector.input_model {
        match code {
            KeyCode::Char(c) => input_model.push(c),
            KeyCode::Backspace => {
                input_model.pop();
            }
            KeyCode::Esc => {
                selector.input_model = None;
            }
            KeyCode::Enter => {
                let name = input_model.trim().to_string();
                if !name.is_empty() {
                    selector.model_name_override = Some(name);
                }
                selector.input_model = None;
            }
            _ => {}
        }
        return;
    }

    if selector.input_key.is_some() {
        match code {
            KeyCode::Char(c) => {
                if let Some(input_key) = &mut selector.input_key {
                    input_key.push(c);
                }
            }
            KeyCode::Backspace => {
                if let Some(input_key) = &mut selector.input_key {
                    input_key.pop();
                }
            }
            KeyCode::Esc => {
                selector.input_key = None;
            }
            KeyCode::Enter => {
                let key = selector.input_key.take().unwrap_or_default();
                let flow = selector.flow;
                let mode = selector.mode;
                let provider = selector.selected_provider();
                let model = selector.effective_model_name();
                let provider_kind = provider.provider.to_string();
                let base_url = model_config::preset_base_url(provider).to_string();
                let provider_index = selector.selected_provider_index;

                match mode {
                    ModelSelectorMode::SelectModel => {
                        let config = agent_rt.block_on(async { agent.lock().await.config().clone() });
                        let (provider_arg, base_url_arg) = if flow == ModelSelectorFlow::Wizard {
                            (Some(provider_kind), Some(base_url))
                        } else {
                            model_switch_endpoint_args_for_index(&config, provider_index)
                        };
                        app.model_selector = None;
                        spawn_model_switch(
                            agent_rt,
                            Arc::clone(agent),
                            ui_tx,
                            model,
                            provider_arg,
                            base_url_arg,
                            Some(key),
                        );
                    }
                    ModelSelectorMode::SelectProvider => {
                        app.model_selector = None;
                        spawn_model_switch(
                            agent_rt,
                            Arc::clone(agent),
                            ui_tx,
                            model,
                            Some(provider_kind),
                            Some(base_url),
                            Some(key),
                        );
                    }
                }
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Up => match selector.mode {
            ModelSelectorMode::SelectModel => {
                let provider = selector.selected_provider();
                let len = model_config::preset_models(provider).len();
                if len > 0 {
                    selector.selected_model_index =
                        (selector.selected_model_index + len - 1) % len;
                    selector.model_name_override = None;
                }
            }
            ModelSelectorMode::SelectProvider => {
                let len = model_config::PROVIDER_PRESETS.len();
                selector.selected_provider_index =
                    (selector.selected_provider_index + len - 1) % len;
                selector.selected_model_index = 0;
                selector.model_name_override = None;
            }
        },
        KeyCode::Down => match selector.mode {
            ModelSelectorMode::SelectModel => {
                let provider = selector.selected_provider();
                let len = model_config::preset_models(provider).len();
                if len > 0 {
                    selector.selected_model_index =
                        (selector.selected_model_index + 1) % len;
                    selector.model_name_override = None;
                }
            }
            ModelSelectorMode::SelectProvider => {
                let len = model_config::PROVIDER_PRESETS.len();
                selector.selected_provider_index =
                    (selector.selected_provider_index + 1) % len;
                selector.selected_model_index = 0;
                selector.model_name_override = None;
            }
        },
        KeyCode::Esc => {
            if selector.mode == ModelSelectorMode::SelectModel
                && selector.flow == ModelSelectorFlow::Wizard
            {
                selector.mode = ModelSelectorMode::SelectProvider;
                selector.input_model = None;
                selector.model_name_override = None;
            } else {
                app.model_selector = None;
                app.lines.push(app::ChatLine::Assistant(
                    "Selection cancelled.".to_string(),
                ));
                app.scroll_to_bottom();
            }
        }
        KeyCode::Char('m') | KeyCode::Char('M')
            if selector.mode == ModelSelectorMode::SelectModel =>
        {
            selector.input_model = Some(selector.effective_model_name());
        }
        KeyCode::Char('k') | KeyCode::Char('K')
            if selector.mode == ModelSelectorMode::SelectModel
                || (selector.mode == ModelSelectorMode::SelectProvider
                    && selector.flow == ModelSelectorFlow::ProviderOnly) =>
        {
            let provider = selector.selected_provider();
            if provider.requires_key {
                selector.input_key = Some(String::new());
            }
        }
        KeyCode::Enter => {
                let flow = selector.flow;
                let mode = selector.mode;
                let provider = selector.selected_provider();
                let mut model = selector.effective_model_name();
                let provider_kind = provider.provider.to_string();
                let base_url = model_config::preset_base_url(provider).to_string();
                let requires_key = provider.requires_key;

                if mode == ModelSelectorMode::SelectProvider
                    && flow == ModelSelectorFlow::Wizard
                {
                    let current = agent_rt
                        .block_on(async { agent.lock().await.config().model.clone() });
                    let provider_index = selector.selected_provider_index;
                    let model_index = model_config::model_index_for_provider(
                        &model_config::PROVIDER_PRESETS[provider_index],
                        &current,
                    );
                    selector.mode = ModelSelectorMode::SelectModel;
                    selector.selected_model_index = model_index;
                    selector.model_name_override = None;
                    return;
                }

                if mode == ModelSelectorMode::SelectProvider {
                    let current = agent_rt
                        .block_on(async { agent.lock().await.config().model.clone() });
                    if model.is_empty()
                        || !model_config::preset_models(provider)
                            .iter()
                            .any(|v| v.model_id == model)
                    {
                        model = model_config::default_model_for_provider(provider, &current);
                    }
                }

                if model.is_empty() {
                    selector.input_model = Some(String::new());
                    return;
                }

                match mode {
                    ModelSelectorMode::SelectModel => {
                        if requires_key {
                            let has_key = {
                                let agent_guard =
                                    agent_rt.block_on(async { agent.lock().await });
                                model_config::has_api_key_for_provider(
                                    agent_guard.config(),
                                    provider,
                                )
                            };
                            if !has_key {
                                selector.input_key = Some(String::new());
                                return;
                            }
                        }
                        let config =
                            agent_rt.block_on(async { agent.lock().await.config().clone() });
                        let (provider_arg, base_url_arg) =
                            if flow == ModelSelectorFlow::Wizard {
                                (
                                    Some(provider_kind),
                                    Some(base_url),
                                )
                            } else {
                                model_switch_endpoint_args(&config, selector)
                            };
                        app.model_selector = None;
                        spawn_model_switch(
                            agent_rt,
                            Arc::clone(agent),
                            ui_tx,
                            model,
                            provider_arg,
                            base_url_arg,
                            None,
                        );
                    }
                    ModelSelectorMode::SelectProvider => {
                        if requires_key {
                            let has_key = {
                                let agent_guard =
                                    agent_rt.block_on(async { agent.lock().await });
                                model_config::has_api_key_for_provider(
                                    agent_guard.config(),
                                    provider,
                                )
                            };

                            if has_key {
                                app.model_selector = None;
                                spawn_model_switch(
                                    agent_rt,
                                    Arc::clone(agent),
                                    ui_tx,
                                    model,
                                    Some(provider_kind),
                                    Some(base_url),
                                    None,
                                );
                            } else {
                                selector.input_key = Some(String::new());
                            }
                        } else {
                            app.model_selector = None;
                            spawn_model_switch(
                                agent_rt,
                                Arc::clone(agent),
                                ui_tx,
                                model,
                                Some(provider_kind),
                                Some(base_url),
                                Some("ollama".to_string()),
                            );
                        }
                    }
                }
            }
        _ => {}
    }
}

fn model_switch_endpoint_args(
    config: &ZeneConfig,
    selector: &app::ModelSelector,
) -> (Option<String>, Option<String>) {
    model_switch_endpoint_args_for_index(config, selector.selected_provider_index)
}

fn model_switch_endpoint_args_for_index(
    config: &ZeneConfig,
    selected_provider_index: usize,
) -> (Option<String>, Option<String>) {
    if model_config::provider_index_for_config(config) == Some(selected_provider_index) {
        return (None, None);
    }
    let provider = &model_config::PROVIDER_PRESETS[selected_provider_index];
    (
        Some(provider.provider.to_string()),
        Some(model_config::preset_base_url(provider).to_string()),
    )
}

fn spawn_model_switch(
    agent_rt: &Runtime,
    agent: Arc<AsyncMutex<Agent>>,
    tx_done: std::sync::mpsc::Sender<UiMessage>,
    model: String,
    provider: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
) {
    agent_rt.spawn(async move {
        let mut agent_guard = agent.lock().await;
        let res = agent_guard
            .switch_model(&model, provider, base_url, api_key)
            .await;
        let msg = res.map(|_| agent_guard.config().model.clone());
        let _ = tx_done.send(UiMessage::ModelSwitchFinished(msg));
    });
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
