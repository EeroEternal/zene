use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Render assistant text with basic markdown (**bold**, `code`, ```fenced blocks```)
/// and word-wrap to `width` columns.
pub fn render_markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::from("")];
    }

    let mut out = Vec::new();
    let parts = split_code_blocks(text);

    for part in parts {
        match part {
            BlockPart::Code(code) => {
                for line in code.lines() {
                    let wrapped = wrap_plain_line(line, width);
                    for w in wrapped {
                        out.push(Line::from(Span::styled(
                            w,
                            Style::default()
                                .fg(Color::Cyan)
                                .bg(Color::DarkGray),
                        )));
                    }
                }
                if code.is_empty() {
                    out.push(Line::from(Span::styled(
                        " ",
                        Style::default().bg(Color::DarkGray),
                    )));
                }
            }
            BlockPart::Text(text) => {
                for paragraph in text.split('\n') {
                    if paragraph.is_empty() {
                        out.push(Line::from(""));
                        continue;
                    }
                    let spans = parse_inline_markdown(paragraph);
                    out.extend(wrap_spans(spans, width));
                }
            }
        }
    }

    if out.is_empty() {
        out.push(Line::from(""));
    }

    out
}

enum BlockPart {
    Text(String),
    Code(String),
}

fn split_code_blocks(input: &str) -> Vec<BlockPart> {
    let mut parts = Vec::new();
    let mut rest = input;

    while let Some(start) = rest.find("```") {
        if start > 0 {
            parts.push(BlockPart::Text(rest[..start].to_string()));
        }
        rest = &rest[start + 3..];
        if let Some(lang_end) = rest.find('\n') {
            rest = &rest[lang_end + 1..];
        }
        if let Some(end) = rest.find("```") {
            parts.push(BlockPart::Code(rest[..end].to_string()));
            rest = &rest[end + 3..];
        } else {
            parts.push(BlockPart::Code(rest.to_string()));
            return parts;
        }
    }

    if !rest.is_empty() {
        parts.push(BlockPart::Text(rest.to_string()));
    }

    parts
}

fn parse_inline_markdown(input: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = input;

    while !rest.is_empty() {
        if let Some(idx) = rest.find("**") {
            if idx > 0 {
                spans.push(Span::raw(rest[..idx].to_string()));
            }
            rest = &rest[idx + 2..];
            if let Some(end) = rest.find("**") {
                spans.push(Span::styled(
                    rest[..end].to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                rest = &rest[end + 2..];
            } else {
                spans.push(Span::raw("**".to_string()));
                spans.push(Span::raw(rest.to_string()));
                return spans;
            }
        } else if let Some(idx) = rest.find('`') {
            if idx > 0 {
                spans.push(Span::raw(rest[..idx].to_string()));
            }
            rest = &rest[idx + 1..];
            if let Some(end) = rest.find('`') {
                spans.push(Span::styled(
                    rest[..end].to_string(),
                    Style::default().fg(Color::Yellow),
                ));
                rest = &rest[end + 1..];
            } else {
                spans.push(Span::raw("`".to_string()));
                spans.push(Span::raw(rest.to_string()));
                return spans;
            }
        } else {
            spans.push(Span::raw(rest.to_string()));
            break;
        }
    }

    spans
}

fn wrap_plain_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    wrap_spans(vec![Span::raw(line.to_string())], width)
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

/// Word-wrap styled spans to `width` columns (one display row per returned [`Line`]).
pub fn wrap_line_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    wrap_spans(spans, width)
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_text = String::new();
    let mut current_style = Style::default();
    let mut col = 0usize;

    for span in spans {
        for ch in span.content.chars() {
            if col >= width {
                if !current_text.is_empty() {
                    current_spans.push(Span::styled(
                        std::mem::take(&mut current_text),
                        current_style,
                    ));
                }
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                col = 0;
            }
            if current_text.is_empty() {
                current_style = span.style;
                current_text.push(ch);
            } else if current_style == span.style {
                current_text.push(ch);
            } else {
                current_spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    current_style,
                ));
                current_style = span.style;
                current_text.push(ch);
            }
            col += 1;
        }
    }

    if !current_text.is_empty() {
        current_spans.push(Span::styled(current_text, current_style));
    }
    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_bold_and_code() {
        let lines = render_markdown("hello **world** and `code`", 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("hello "));
        assert!(text.contains("world"));
        assert!(text.contains("code"));
    }

    #[test]
    fn wraps_long_lines() {
        let lines = render_markdown("abcdefghijklmnopqrstuvwxyz", 10);
        assert!(lines.len() >= 2);
    }
}
