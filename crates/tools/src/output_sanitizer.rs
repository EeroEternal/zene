/// Output filter for tool executions to reduce noise and conserve context tokens.
pub struct OutputSanitizer;

impl OutputSanitizer {
    /// Sanitize terminal command outputs (e.g., cargo test, npm test, pytest)
    pub fn sanitize_exec_output(
        command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> String {
        let is_test_command = command.contains("cargo test")
            || command.contains("npm test")
            || command.contains("pytest")
            || command.contains("vitest")
            || command.contains("jest");

        if is_test_command && exit_code == 0 {
            return Self::summarize_passed_tests(stdout, stderr);
        }

        if is_test_command && exit_code != 0 {
            return Self::filter_failed_tests(stdout, stderr);
        }

        let mut content = String::new();
        if !stdout.is_empty() {
            content.push_str(stdout);
        }
        if !stderr.is_empty() {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(stderr);
        }
        if content.is_empty() {
            content = format!("exit code {exit_code}");
        }

        Self::truncate_excessive_lines(&content, 300)
    }

    fn summarize_passed_tests(stdout: &str, _stderr: &str) -> String {
        let mut summary_lines = Vec::new();
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("test result:")
                || trimmed.starts_with("Tests:")
                || trimmed.starts_with("Ran ")
                || trimmed.contains("passed")
                || trimmed.contains("passed;")
            {
                summary_lines.push(trimmed);
            }
        }

        if summary_lines.is_empty() {
            "All tests passed successfully.".to_string()
        } else {
            format!("All tests passed.\n{}", summary_lines.join("\n"))
        }
    }

    fn filter_failed_tests(stdout: &str, stderr: &str) -> String {
        let mut result = Vec::new();
        let combined = format!("{stdout}\n{stderr}");

        let mut in_failure_section = false;
        for line in combined.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("failures:")
                || trimmed.starts_with("FAIL ")
                || trimmed.starts_with("FAILED ")
                || trimmed.contains("FAILED")
            {
                in_failure_section = true;
            }

            if in_failure_section
                || (trimmed.starts_with("test ") && trimmed.ends_with("... FAILED"))
            {
                result.push(line);
            }
        }

        if result.is_empty() {
            Self::truncate_excessive_lines(&combined, 200)
        } else {
            result.join("\n")
        }
    }

    fn truncate_excessive_lines(text: &str, max_lines: usize) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= max_lines {
            return text.to_string();
        }

        let head = &lines[..max_lines / 2];
        let tail = &lines[lines.len() - (max_lines / 2)..];
        let omitted = lines.len() - head.len() - tail.len();

        format!(
            "{}\n\n[... omitted {} lines to save context tokens ...]\n\n{}",
            head.join("\n"),
            omitted,
            tail.join("\n")
        )
    }

    /// DeepSeek Harness style tool-result pruning:
    /// For very large tool result bodies (> 8192 chars), preserve the head (4096 chars)
    /// and tail (1024 chars), replacing the middle with an omission marker.
    /// This prevents single verbose tool calls from blowing the context window
    /// while keeping critical tool exit / output details.
    pub fn prune_tool_result(
        text: &str,
        threshold: usize,
        head_len: usize,
        tail_len: usize,
    ) -> String {
        let char_count = text.chars().count();
        if char_count <= threshold || char_count <= head_len + tail_len {
            return text.to_string();
        }

        let head: String = text.chars().take(head_len).collect();
        let tail: String = text.chars().skip(char_count - tail_len).collect();
        let omitted = char_count - head_len - tail_len;

        format!(
            "{head}\n\n[... tool output pruned {omitted} characters to preserve prefix cache ...]\n\n{tail}"
        )
    }

    /// Default pruning configuration matching DeepSeek Harness standards:
    /// threshold: 8192 chars, head: 4096 chars, tail: 1024 chars.
    pub fn prune_default(text: &str) -> String {
        Self::prune_tool_result(text, 8192, 4096, 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitizer_passed_cargo_test() {
        let stdout = r#"
running 28 tests
test line_endings::tests::detect_lf ... ok
test line_endings::tests::detect_crlf ... ok
test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
        "#;
        let out = OutputSanitizer::sanitize_exec_output("cargo test -p zene-tools", stdout, "", 0);
        assert!(out.contains("All tests passed."));
        assert!(out.contains("28 passed"));
        assert!(!out.contains("detect_lf ... ok"));
    }

    #[test]
    fn test_sanitizer_truncates_long_output() {
        let long_output = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = OutputSanitizer::sanitize_exec_output("cat big_file.txt", &long_output, "", 0);
        assert!(out.contains("omitted"));
        assert!(out.contains("line 0"));
        assert!(out.contains("line 499"));
    }

    #[test]
    fn test_prune_tool_result_preserves_head_and_tail() {
        let text = (0..10_000).map(|_| "a").collect::<String>();
        let pruned = OutputSanitizer::prune_default(&text);
        assert!(pruned.len() < 6000);
        assert!(pruned.contains("tool output pruned"));
        assert!(pruned.starts_with(&"a".repeat(100)));
        assert!(pruned.ends_with(&"a".repeat(100)));

        // Text below threshold is untouched
        let short = "short output";
        assert_eq!(OutputSanitizer::prune_default(short), short);
    }
}
