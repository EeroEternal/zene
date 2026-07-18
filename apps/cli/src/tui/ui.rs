use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};

use super::app::{App, ChatLine, RunState};
use super::markdown::{render_markdown, wrap_line_spans};

/// Input box grows with the draft up to this many text rows (absolute cap).
const MAX_INPUT_ROWS: u16 = 8;
const MIN_CHAT_HEIGHT: u16 = 6;
const STATUS_HEIGHT: u16 = 1;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let terminal_height = frame.area().height;
    let max_input_rows = terminal_height
        .saturating_sub(MIN_CHAT_HEIGHT)
        .saturating_sub(STATUS_HEIGHT)
        .saturating_sub(2)
        .clamp(1, MAX_INPUT_ROWS);

    // Estimate wrap width before layout (inner width unknown until area assigned).
    let est_inner = frame.area().width.saturating_sub(4).max(1);
    app.input_wrap_width = est_inner;
    let content_rows = app.input.display_line_count(est_inner) as u16;
    let input_rows = content_rows.clamp(1, max_input_rows);
    let input_height = input_rows + 2; // borders

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(MIN_CHAT_HEIGHT),
            Constraint::Length(input_height),
            Constraint::Length(STATUS_HEIGHT),
        ])
        .split(frame.area());

    draw_chat(frame, chunks[0], app);
    draw_input(frame, chunks[1], app, input_rows);
    draw_status(frame, chunks[2], app);

    if let Some(perm) = &app.permission {
        draw_permission_overlay(frame, perm);
    } else if let Some(selector) = &app.model_selector {
        draw_model_selector_overlay(frame, selector);
    }
}

fn draw_chat(frame: &mut Frame, area: Rect, app: &mut App) {
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    app.chat_viewport_height = area.height;

    let mut lines: Vec<Line> = Vec::new();

    for line in &app.lines {
        match line {
            ChatLine::User(text) => {
                lines.extend(wrap_line_spans(
                    vec![
                        Span::styled("> ", Style::default().fg(Color::Cyan)),
                        Span::raw(text.clone()),
                    ],
                    inner_width,
                ));
                lines.push(Line::from(""));
            }
            ChatLine::Assistant(text) => {
                lines.extend(render_markdown(text, inner_width));
                lines.push(Line::from(""));
            }
            ChatLine::Tool {
                header,
                body,
                is_error,
                running,
            } => {
                let color = if *is_error {
                    Color::Red
                } else if *running {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };
                let display_header = if *running {
                    format!("{} {}", app.spinner(), header)
                } else {
                    header.clone()
                };
                lines.extend(wrap_line_spans(
                    vec![Span::styled(display_header, Style::default().fg(color))],
                    inner_width,
                ));
                if let Some(body) = body {
                    lines.extend(render_diff_lines(body, inner_width));
                }
                lines.push(Line::from(""));
            }
            ChatLine::Error(text) => {
                lines.extend(wrap_line_spans(
                    vec![Span::styled(
                        format!("Error: {text}"),
                        Style::default().fg(Color::Red),
                    )],
                    inner_width,
                ));
                lines.push(Line::from(""));
            }
            ChatLine::Status(text) => {
                lines.extend(wrap_line_spans(
                    vec![Span::styled(
                        text.clone(),
                        Style::default().fg(Color::DarkGray),
                    )],
                    inner_width,
                ));
            }
        }
    }

    if let Some(streaming) = &app.streaming {
        lines.extend(render_markdown(streaming, inner_width));
        lines.push(Line::from(""));
    }

    let total_lines = lines.len() as u16;
    let viewport = area.height.saturating_sub(2).max(1);
    let max_scroll = total_lines.saturating_sub(viewport);
    app.max_scroll = max_scroll;

    if app.stick_to_bottom {
        app.scroll = max_scroll;
    } else {
        app.scroll = app.scroll.min(max_scroll);
    }

    let base_title = match app.run_state {
        RunState::Running => {
            if app.activity.is_empty() {
                format!("Chat {} running…", app.spinner())
            } else {
                format!("Chat {} {}", app.spinner(), app.activity)
            }
        }
        RunState::Cancelling => format!("Chat {} cancelling…", app.spinner()),
        RunState::Idle => "Chat".to_string(),
    };
    let title = if max_scroll > 0 && !app.stick_to_bottom {
        format!("{base_title} — wheel/PgUp·PgDn scroll · PgDn→bottom resumes follow")
    } else {
        base_title
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        )
        .scroll((app.scroll, 0));

    frame.render_widget(paragraph, area);

    if max_scroll > 0 {
        let mut scrollbar_state = ScrollbarState::new(total_lines as usize)
            .viewport_content_length(viewport as usize)
            .position(app.scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .thumb_symbol("█")
            .track_symbol(Some("│"))
            .style(Style::default().fg(Color::DarkGray));
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_diff_lines(diff: &str, width: usize) -> Vec<Line<'static>> {
    diff.lines()
        .flat_map(|line| {
            let style = if line.starts_with('+') && !line.starts_with("+++") {
                Style::default().fg(Color::Green)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Style::default().fg(Color::Red)
            } else if line.starts_with('@') {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            wrap_line_spans(vec![Span::styled(format!("  {line}"), style)], width)
        })
        .collect()
}

fn draw_input(frame: &mut Frame, area: Rect, app: &mut App, visible_rows: u16) {
    let inner_width = area.width.saturating_sub(2).max(1);
    app.input_wrap_width = inner_width;

    let title = match app.run_state {
        RunState::Idle => {
            if app.pending_count() > 0 {
                format!(
                    "Input (Enter send · {} queued waiting)",
                    app.pending_count()
                )
            } else {
                "Input (Enter send · Alt+Enter newline · ↑↓ history)".to_string()
            }
        }
        RunState::Running => {
            let pending = app.pending_count();
            let pending_note = if pending > 0 {
                format!(" · {pending} queued")
            } else {
                String::new()
            };
            if app.activity.is_empty() {
                format!(
                    "Input {} running — Enter queue{pending_note} · /steer · Esc cancel",
                    app.spinner()
                )
            } else {
                format!(
                    "Input {} {} — Enter queue{pending_note} · /steer · Esc cancel",
                    app.spinner(),
                    app.activity
                )
            }
        }
        RunState::Cancelling => {
            format!(
                "Input {} cancelling… — Enter queue · Esc/Ctrl+C retry",
                app.spinner()
            )
        }
    };

    let layout = app.input.layout(inner_width);
    let styled_lines: Vec<Line> = layout
        .lines
        .iter()
        .map(|full| {
            if let Some(body) = full.strip_prefix("> ") {
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Cyan)),
                    Span::raw(body.to_string()),
                ])
            } else if let Some(body) = full.strip_prefix("  ") {
                Line::from(vec![
                    Span::styled("  ", Style::default().fg(Color::Cyan)),
                    Span::raw(body.to_string()),
                ])
            } else {
                Line::from(Span::raw(full.clone()))
            }
        })
        .collect();

    let (cursor_row, cursor_col) = app.input.cursor_display_position(inner_width);
    let scroll_y = if cursor_row >= visible_rows as usize {
        (cursor_row - visible_rows as usize + 1) as u16
    } else {
        0
    };

    let input = Paragraph::new(styled_lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .scroll((scroll_y, 0));

    frame.render_widget(input, area);

    let has_overlay = app.permission.is_some() || app.model_selector.is_some();
    if !has_overlay {
        let cursor_x = area.x + 1 + cursor_col;
        let cursor_y = area.y + 1 + cursor_row.saturating_sub(scroll_y as usize) as u16;
        let max_y = area.bottom().saturating_sub(2);
        frame.set_cursor_position((
            cursor_x.min(area.right().saturating_sub(1)),
            cursor_y.min(max_y),
        ));
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let mode = match app.permission_mode {
        zene_core::PermissionMode::Yolo | zene_core::PermissionMode::BypassPermissions => "bypass",
        zene_core::PermissionMode::Manual | zene_core::PermissionMode::Default => "default",
        zene_core::PermissionMode::AcceptEdits => "accept_edits",
        zene_core::PermissionMode::DontAsk => "dont_ask",
    };
    let state = match app.run_state {
        RunState::Idle => "ready",
        RunState::Running => "running",
        RunState::Cancelling => "cancelling",
    };
    let activity = if app.activity.is_empty() {
        String::new()
    } else {
        format!(" | {}", app.activity)
    };
    let spinner = if app.run_state == RunState::Running || app.run_state == RunState::Cancelling {
        format!("{} ", app.spinner())
    } else {
        String::new()
    };
    let mut status = format!(
        "{spinner}{} | session {} | ctx {}% | in {} / out {} ({}) | {} | {}{} ",
        app.model,
        &app.session_id[..8.min(app.session_id.len())],
        app.context_usage_percent,
        app.usage.prompt_tokens,
        app.usage.completion_tokens,
        app.usage.total_tokens,
        mode,
        state,
        activity,
    );
    if app.permission.is_some() {
        status.push_str("| [y]/[a]/[n] approve ");
    } else if app.run_state == RunState::Running || app.run_state == RunState::Cancelling {
        status.push_str("| Enter queue · Esc cancel | wheel/PgUp·PgDn scroll ");
    }
    if app.pending_count() > 0 && app.run_state == RunState::Idle {
        status.push_str(&format!("| {} queued ", app.pending_count()));
    }
    if !app.mouse_scroll_enabled {
        status.push_str("| mouse off (Shift+select · Ctrl+Shift+M toggle) ");
    }
    let paragraph = Paragraph::new(status).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

fn draw_permission_overlay(frame: &mut Frame, perm: &super::app::PermissionPrompt) {
    let area = centered_rect(70, 40, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Confirm")
        .style(Style::default().fg(Color::White).bg(Color::Black));
    frame.render_widget(block.clone(), area);

    let inner = block.inner(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let body = vec![
        Line::from(Span::styled(
            "Permission required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tool: ", Style::default().fg(Color::DarkGray)),
            Span::styled(perm.tool_name.clone(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Args: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                truncate_for_width(&perm.args_preview, chunks[0].width.saturating_sub(6)),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: true }),
        chunks[0],
    );

    let footer = Line::from(Span::styled(
        "[y] allow once  [n] deny  [a] approve for session",
        Style::default().fg(Color::Yellow),
    ));
    frame.render_widget(Paragraph::new(footer), chunks[1]);
}

fn truncate_for_width(s: &str, max_cols: u16) -> String {
    let max = max_cols as usize;
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn draw_model_selector_overlay(frame: &mut Frame, selector: &super::app::ModelSelector) {
    use crate::model_config::{preset_base_url, preset_models, PROVIDER_PRESETS};

    let area = centered_rect(65, 55, frame.area());
    frame.render_widget(Clear, area);

    if let Some(input_model) = &selector.input_model {
        let provider = selector.selected_provider();
        let text = vec![
            Line::from(Span::styled(
                "Model Name",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                Span::styled(provider.display_name, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Model ID: ", Style::default().fg(Color::Yellow)),
                Span::styled(input_model, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "[Enter] confirm  [Esc] back",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .title("Edit Model Name")
            .style(Style::default().fg(Color::White).bg(Color::Black));

        frame.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: true }), area);

        let cursor_x = area.x + 11 + input_model.chars().count() as u16;
        let cursor_y = area.y + 5;
        frame.set_cursor_position((cursor_x.min(area.right() - 2), cursor_y));
    } else if let Some(input_key) = &selector.input_key {
        let provider = selector.selected_provider();
        let model = selector.effective_model_name();
        let base_url = preset_base_url(provider);
        let text = vec![
            Line::from(Span::styled(
                "API Key Configuration",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                Span::styled(provider.display_name, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Model:    ", Style::default().fg(Color::DarkGray)),
                Span::styled(model, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Base URL: ", Style::default().fg(Color::DarkGray)),
                Span::styled(base_url, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Enter API Key: ", Style::default().fg(Color::Yellow)),
                Span::styled(input_key, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "[Enter] save key  [Esc] back",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .title("API Key")
            .style(Style::default().fg(Color::White).bg(Color::Black));

        frame.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: true }), area);

        let cursor_x = area.x + 16 + input_key.chars().count() as u16;
        let cursor_y = area.y + 8;
        frame.set_cursor_position((cursor_x.min(area.right() - 2), cursor_y));
    } else {
        use super::app::{ModelSelectorFlow, ModelSelectorMode};

        let provider = selector.selected_provider();
        let base_url = preset_base_url(provider);
        let models = preset_models(provider);

        match selector.mode {
            ModelSelectorMode::SelectModel => {
                let step = if selector.flow == ModelSelectorFlow::Wizard {
                    "Step 2/2 — Select Model"
                } else {
                    "Select Model"
                };
                let mut lines = vec![
                    Line::from(Span::styled(
                        step,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(provider.display_name, Style::default().fg(Color::White)),
                    ]),
                    Line::from(""),
                ];

                for (i, variant) in models.iter().enumerate() {
                    let prefix = if i == selector.selected_model_index {
                        "> "
                    } else {
                        "  "
                    };
                    let style = if i == selector.selected_model_index {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    lines.push(Line::from(Span::styled(
                        format!("{prefix}{} ({})", variant.display_name, variant.model_id),
                        style,
                    )));
                }

                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("Base URL: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(base_url, Style::default().fg(Color::White)),
                ]));
                lines.push(Line::from(""));
                let footer = if selector.flow == ModelSelectorFlow::Wizard {
                    "[↑/↓] model  [m] custom  [Enter] apply  [Esc] back to providers"
                } else {
                    "[↑/↓] model  [m] custom  [k] API key  [Enter] apply  [Esc] close"
                };
                lines.push(Line::from(Span::styled(
                    footer,
                    Style::default().fg(Color::DarkGray),
                )));

                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("Models")
                    .style(Style::default().fg(Color::White).bg(Color::Black));

                frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
            }
            ModelSelectorMode::SelectProvider => {
                let title = if selector.flow == ModelSelectorFlow::Wizard {
                    "Step 1/2 — Select Provider"
                } else {
                    "Select Provider"
                };
                let mut lines = vec![
                    Line::from(Span::styled(
                        title,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                ];

                for (i, item) in PROVIDER_PRESETS.iter().enumerate() {
                    let prefix = if i == selector.selected_provider_index {
                        "> "
                    } else {
                        "  "
                    };
                    let style = if i == selector.selected_provider_index {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    lines.push(Line::from(Span::styled(
                        format!("{prefix}{}", item.display_name),
                        style,
                    )));
                }

                lines.push(Line::from(""));
                if selector.flow == ModelSelectorFlow::Wizard {
                    lines.push(Line::from(vec![
                        Span::styled("Base URL: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(base_url, Style::default().fg(Color::White)),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "[↑/↓] provider  [Enter] choose model  [Esc] close",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("Base URL: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(base_url, Style::default().fg(Color::White)),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "Default models after setup:",
                        Style::default().fg(Color::DarkGray),
                    )));
                    for variant in models.iter().take(8) {
                        lines.push(Line::from(format!(
                            "  {} ({})",
                            variant.display_name, variant.model_id
                        )));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "[↑/↓] provider  [k] API key  [Enter] apply  [Esc] close",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("Provider")
                    .style(Style::default().fg(Color::White).bg(Color::Black));

                frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
            }
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
