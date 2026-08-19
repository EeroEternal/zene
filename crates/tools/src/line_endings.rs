#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndingStyle {
    Lf,
    Crlf,
    Mixed,
}

#[derive(Debug, Clone)]
pub struct ModelTextView {
    pub text: String,
    pub line_ending_style: LineEndingStyle,
}

pub fn detect_line_ending_style(text: &str) -> LineEndingStyle {
    let mut has_crlf = false;
    let mut has_lf = false;
    let mut has_lone_cr = false;

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                has_crlf = true;
                chars.next();
            } else {
                has_lone_cr = true;
            }
        } else if ch == '\n' {
            has_lf = true;
        }
    }

    if has_lone_cr || (has_crlf && has_lf) {
        LineEndingStyle::Mixed
    } else if has_crlf {
        LineEndingStyle::Crlf
    } else {
        LineEndingStyle::Lf
    }
}

pub fn to_model_text_view(raw: &str) -> ModelTextView {
    let line_ending_style = detect_line_ending_style(raw);
    let text = if line_ending_style == LineEndingStyle::Crlf {
        raw.replace("\r\n", "\n")
    } else {
        raw.to_string()
    };
    ModelTextView {
        text,
        line_ending_style,
    }
}

pub fn materialize_model_text(text: &str, line_ending_style: LineEndingStyle) -> String {
    if line_ending_style != LineEndingStyle::Crlf {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

pub fn make_carriage_returns_visible(text: &str) -> String {
    text.replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_lf() {
        assert_eq!(detect_line_ending_style("a\nb\n"), LineEndingStyle::Lf);
    }

    #[test]
    fn detect_crlf() {
        assert_eq!(
            detect_line_ending_style("a\r\nb\r\n"),
            LineEndingStyle::Crlf
        );
    }

    #[test]
    fn detect_mixed() {
        assert_eq!(
            detect_line_ending_style("a\r\nb\nc"),
            LineEndingStyle::Mixed
        );
    }

    #[test]
    fn to_model_view_crlf() {
        let view = to_model_text_view("hello\r\nworld\r\n");
        assert_eq!(view.text, "hello\nworld\n");
        assert_eq!(view.line_ending_style, LineEndingStyle::Crlf);
    }

    #[test]
    fn materialize_crlf() {
        assert_eq!(
            materialize_model_text("a\nb\n", LineEndingStyle::Crlf),
            "a\r\nb\r\n"
        );
    }
}
