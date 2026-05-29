use similar::TextDiff;

/// Compact unified diff preview (first `max_lines` lines).
pub fn compact_unified_diff(path: &str, old: &str, new: &str, max_lines: usize) -> String {
    if old == new {
        return String::new();
    }

    let diff = TextDiff::from_lines(old, new);
    let full = format!(
        "{}",
        diff.unified_diff()
            .context_radius(2)
            .header(&format!("a/{path}"), &format!("b/{path}"))
    );
    full.lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_unified_diff() {
        let diff = compact_unified_diff("foo.rs", "fn old()\n", "fn new()\n", 20);
        assert!(diff.contains("-fn old()"));
        assert!(diff.contains("+fn new()"));
    }
}
