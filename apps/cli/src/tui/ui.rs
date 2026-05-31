use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{App, ChatLine, RunState};
use super::markdown::render_markdown;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_chat(frame, chunks[0], app);
    draw_input(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);

    if let Some(perm) = &app.permission {
        draw_permission_overlay(frame, perm);
    } else if let Some(prompt) = &app.api_key_prompt {
        draw_api_key_prompt_overlay(frame, prompt);
    }
}

fn draw_chat(frame: &mut Frame, area: Rect, app: &mut App) {
    let inner_width = area.width.saturating_sub(2) as usize;
    app.chat_viewport_height = area.height;

    let mut lines: Vec<Line> = Vec::new();

    for line in &app.lines {
        match line {
            ChatLine::User(text) => {
                lines.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Cyan)),
                    Span::raw(text.clone()),
                ]));
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
            } => {
                let color = if *is_error {
                    Color::Red
                } else {
                    Color::DarkGray
                };
                lines.push(Line::from(Span::styled(
                    header.clone(),
                    Style::default().fg(color),
                )));
                if let Some(body) = body {
                    lines.extend(render_diff_lines(body));
                }
                lines.push(Line::from(""));
            }
            ChatLine::Error(text) => {
                lines.push(Line::from(Span::styled(
                    format!("Error: {text}"),
                    Style::default().fg(Color::Red),
                )));
                lines.push(Line::from(""));
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

    let title = match app.run_state {
        RunState::Running => "Chat (running…)",
        RunState::Idle => "Chat",
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        )
        .wrap(Wrap { trim: false })
        .scroll((0, app.scroll));

    frame.render_widget(paragraph, area);
}

fn render_diff_lines(diff: &str) -> Vec<Line<'static>> {
    diff.lines()
        .map(|line| {
            let style = if line.starts_with('+') && !line.starts_with("+++") {
                Style::default().fg(Color::Green)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Style::default().fg(Color::Red)
            } else if line.starts_with('@') {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(format!("  {line}"), style))
        })
        .collect()
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let prompt = if app.run_state == RunState::Running {
        "…"
    } else {
        "> "
    };
    let input = Paragraph::new(format!("{prompt}{}", app.input))
        .block(Block::default().borders(Borders::ALL).title("Input"))
        .wrap(Wrap { trim: false });

    frame.render_widget(input, area);

    if app.run_state == RunState::Idle && app.permission.is_none() && app.api_key_prompt.is_none() {
        let cursor_x = area.x + 1 + prompt.len() as u16 + app.input.chars().count() as u16;
        let cursor_y = area.y + 1;
        frame.set_cursor_position((cursor_x.min(area.right() - 1), cursor_y));
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let mode = match app.permission_mode {
        zene_core::PermissionMode::Yolo => "yolo",
        zene_core::PermissionMode::Manual => "manual",
    };
    let status = format!(
        " {} | session {} | in {} / out {} ({}) | {} ",
        app.model,
        &app.session_id[..8.min(app.session_id.len())],
        app.usage.prompt_tokens,
        app.usage.completion_tokens,
        app.usage.total_tokens,
        mode,
    );
    let paragraph = Paragraph::new(status).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

fn draw_permission_overlay(frame: &mut Frame, perm: &super::app::PermissionPrompt) {
    let area = centered_rect(70, 30, frame.area());
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        frame.area(),
    );

    let text = vec![
        Line::from(Span::styled(
            "Permission required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Tool: {}", perm.tool_name)),
        Line::from(format!("Args: {}", perm.args_preview)),
        Line::from(""),
        Line::from("[y] allow once  [n] deny  [a] approve for session"),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Confirm")
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));

    frame.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: true }), area);
}

fn draw_api_key_prompt_overlay(frame: &mut Frame, prompt: &super::app::ApiKeyPrompt) {
    let area = centered_rect(75, 30, frame.area());
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        frame.area(),
    );

    let text = vec![
        Line::from(Span::styled(
            "API Key Configuration Required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Model:    {}", prompt.model)),
        Line::from(format!("Provider: {}", prompt.provider)),
        Line::from(format!("Base URL: {}", prompt.base_url.as_deref().unwrap_or("default"))),
        Line::from(""),
        Line::from(format!("Enter API Key: {}", prompt.input_key)),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] submit  [Esc] cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title("API Key Configuration")
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));

    frame.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: true }), area);

    let cursor_x = area.x + 16 + prompt.input_key.chars().count() as u16;
    let cursor_y = area.y + 7;
    frame.set_cursor_position((cursor_x.min(area.right() - 2), cursor_y));
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
