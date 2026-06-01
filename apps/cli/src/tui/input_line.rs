//! Multi-line input with UTF-8 cursor (for CJK editing, arrow keys, and
//! Alt+Enter newlines). Cursor positions are tracked in Unicode scalar values.

use crossterm::event::{KeyCode, KeyModifiers};

const PROMPT: &str = "> ";
const INDENT: &str = "  ";

#[derive(Debug, Clone)]
struct DisplayLine {
    prefix: String,
    content: String,
    char_start: usize,
    char_end: usize,
    prefix_cols: u16,
}

impl DisplayLine {
    fn text(&self) -> String {
        format!("{}{}", self.prefix, self.content)
    }
}

#[derive(Debug, Clone)]
pub struct InputLayout {
    pub lines: Vec<String>,
    segments: Vec<DisplayLine>,
}

#[derive(Debug, Clone, Default)]
pub struct InputLine {
    text: String,
    cursor: usize,
}

impl InputLine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn trimmed(&self) -> &str {
        self.text.trim()
    }

    #[allow(dead_code)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.char_len();
    }

    pub fn display_line_count(&self, inner_width: u16) -> usize {
        self.layout(inner_width).lines.len().max(1)
    }

    pub fn layout(&self, inner_width: u16) -> InputLayout {
        let inner_width = inner_width.max(1) as usize;
        let mut segments = Vec::new();

        if self.text.is_empty() {
            segments.push(DisplayLine {
                prefix: PROMPT.to_string(),
                content: String::new(),
                char_start: 0,
                char_end: 0,
                prefix_cols: display_width(PROMPT),
            });
        } else {
            let mut char_offset = 0usize;
            for (logical_i, row_text) in self.text.split('\n').enumerate() {
                let prefix = if logical_i == 0 {
                    PROMPT.to_string()
                } else {
                    INDENT.to_string()
                };
                segments.extend(wrap_logical_row(
                    prefix,
                    INDENT.to_string(),
                    row_text,
                    inner_width,
                    char_offset,
                ));
                char_offset += row_text.chars().count() + 1;
            }
        }

        let lines = segments.iter().map(|s| s.text()).collect();
        InputLayout { lines, segments }
    }

    pub fn cursor_display_position(&self, inner_width: u16) -> (usize, u16) {
        let layout = self.layout(inner_width);
        let cursor = self.cursor.min(self.char_len());

        for (i, seg) in layout.segments.iter().enumerate() {
            if cursor >= seg.char_start && cursor <= seg.char_end {
                let content_before: String = self
                    .text
                    .chars()
                    .skip(seg.char_start)
                    .take(cursor.saturating_sub(seg.char_start))
                    .collect();
                return (i, seg.prefix_cols + display_width(&content_before));
            }
        }

        if let Some(last) = layout.segments.last() {
            let content: String = self.text.chars().skip(last.char_start).collect();
            return (
                layout.segments.len().saturating_sub(1),
                last.prefix_cols + display_width(&content),
            );
        }

        (0, display_width(PROMPT))
    }

    pub fn move_up_wrapped(&mut self, inner_width: u16) -> bool {
        let layout = self.layout(inner_width);
        let (row, col) = self.cursor_display_position(inner_width);
        if row == 0 {
            return false;
        }
        self.cursor = display_col_to_char(&layout.segments[row - 1], col);
        true
    }

    pub fn move_down_wrapped(&mut self, inner_width: u16) -> bool {
        let layout = self.layout(inner_width);
        let (row, col) = self.cursor_display_position(inner_width);
        if row + 1 >= layout.segments.len() {
            return false;
        }
        self.cursor = display_col_to_char(&layout.segments[row + 1], col);
        true
    }

    pub fn move_home_wrapped(&mut self, inner_width: u16) {
        let layout = self.layout(inner_width);
        let (row, _) = self.cursor_display_position(inner_width);
        if let Some(seg) = layout.segments.get(row) {
            self.cursor = seg.char_start;
        }
    }

    pub fn move_end_wrapped(&mut self, inner_width: u16) {
        let layout = self.layout(inner_width);
        let (row, _) = self.cursor_display_position(inner_width);
        if let Some(seg) = layout.segments.get(row) {
            self.cursor = seg.char_end;
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        let byte_idx = self.byte_index(self.cursor);
        self.text.insert(byte_idx, ch);
        self.cursor += 1;
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let byte_idx = self.byte_index(self.cursor);
        let char_count = s.chars().count();
        self.text.insert_str(byte_idx, s);
        self.cursor += char_count;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let start = self.byte_index(self.cursor);
        let end = self.byte_index(self.cursor + 1);
        self.text.drain(start..end);
    }

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.char_len() {
            return;
        }
        let start = self.byte_index(self.cursor);
        let end = self.byte_index(self.cursor + 1);
        self.text.drain(start..end);
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.char_len() {
            self.cursor += 1;
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return;
        }
        match code {
            KeyCode::Char(ch) => self.insert_char(ch),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            _ => {}
        }
    }

    fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }
}

fn wrap_logical_row(
    prefix: String,
    indent: String,
    row_text: &str,
    inner_width: usize,
    char_offset: usize,
) -> Vec<DisplayLine> {
    let chars: Vec<char> = row_text.chars().collect();
    let mut segments = Vec::new();
    let mut idx = 0usize;
    let mut first = true;

    if chars.is_empty() {
        segments.push(DisplayLine {
            prefix: prefix.clone(),
            content: String::new(),
            char_start: char_offset,
            char_end: char_offset,
            prefix_cols: display_width(&prefix),
        });
        return segments;
    }

    while idx < chars.len() || first {
        let seg_prefix = if first { prefix.clone() } else { indent.clone() };
        let prefix_cols = display_width(&seg_prefix);
        let avail = inner_width.saturating_sub(prefix_cols as usize).max(1);

        let line_start = char_offset + idx;
        let mut col = 0usize;
        let mut line_chars = String::new();
        let mut consumed = 0usize;

        while idx < chars.len() {
            let ch = chars[idx];
            let w = char_display_width(ch) as usize;
            if col + w > avail && !line_chars.is_empty() {
                break;
            }
            line_chars.push(ch);
            col += w;
            idx += 1;
            consumed += 1;
        }

        segments.push(DisplayLine {
            prefix: seg_prefix,
            content: line_chars,
            char_start: line_start,
            char_end: line_start + consumed,
            prefix_cols,
        });
        first = false;

        if consumed == 0 {
            break;
        }
    }

    segments
}

fn display_col_to_char(seg: &DisplayLine, target_col: u16) -> usize {
    if target_col <= seg.prefix_cols {
        return seg.char_start;
    }
    let target = target_col - seg.prefix_cols;
    let mut col = 0u16;
    for (i, ch) in seg.content.chars().enumerate() {
        if col >= target {
            return seg.char_start + i;
        }
        col += char_display_width(ch);
    }
    seg.char_end
}

fn display_width(s: &str) -> u16 {
    s.chars().map(char_display_width).sum()
}

fn char_display_width(ch: char) -> u16 {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_move_cursor() {
        let mut line = InputLine::new();
        line.insert_str("ab");
        line.move_home_wrapped(80);
        line.insert_char('中');
        assert_eq!(line.as_str(), "中ab");
        assert_eq!(line.cursor(), 1);
        line.move_right();
        assert_eq!(line.cursor(), 2);
    }

    #[test]
    fn backspace_at_cursor() {
        let mut line = InputLine::new();
        line.insert_str("a中b");
        line.move_home_wrapped(80);
        line.move_right();
        line.move_right();
        line.backspace();
        assert_eq!(line.as_str(), "ab");
    }

    #[test]
    fn paste_inserts_at_cursor() {
        let mut line = InputLine::new();
        line.insert_str("hi");
        line.move_home_wrapped(80);
        line.insert_str("你好");
        assert_eq!(line.as_str(), "你好hi");
    }

    #[test]
    fn arrow_keys_via_handle_key() {
        let mut line = InputLine::new();
        line.insert_str("abc");
        line.handle_key(KeyCode::Left, KeyModifiers::NONE);
        line.handle_key(KeyCode::Left, KeyModifiers::NONE);
        line.insert_char('X');
        assert_eq!(line.as_str(), "aXbc");
    }

    #[test]
    fn newline_and_vertical_moves() {
        let mut line = InputLine::new();
        line.insert_str("ab");
        line.insert_newline();
        line.insert_str("cde");
        assert_eq!(line.as_str(), "ab\ncde");
        assert!(line.move_up_wrapped(80));
        assert!(line.move_down_wrapped(80));
    }

    #[test]
    fn wrapped_long_line_cursor() {
        let mut line = InputLine::new();
        line.insert_str("abcdefghijklmnopqrstuvwxyz");
        let layout = line.layout(12);
        assert!(layout.lines.len() >= 2);
        line.cursor = line.char_len();
        let (row, _) = line.cursor_display_position(12);
        assert_eq!(row, layout.lines.len() - 1);
    }

    #[test]
    fn home_end_on_row() {
        let mut line = InputLine::new();
        line.insert_str("foo");
        line.insert_newline();
        line.insert_str("barbaz");
        line.move_home_wrapped(80);
        assert_eq!(line.cursor(), 4);
        line.move_end_wrapped(80);
        assert_eq!(line.cursor(), line.char_len());
    }
}
