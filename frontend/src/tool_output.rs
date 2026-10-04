//! Safe ANSI output rendering shared by every chat mode.
use crate::terminal_output::{LINE_LIMIT, TerminalOutput};

pub const PREVIEW_LINES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputContent {
    pub lines: Vec<String>,
    pub truncated: bool,
}

impl ToolOutputContent {
    pub fn parse(text: &str) -> Self {
        let mut output = TerminalOutput::default();
        output.push(text);
        let mut lines: Vec<_> = output.completed().map(|line| line.html.clone()).collect();
        let truncated = output.was_truncated();
        lines.push(output.current_html());
        debug_assert!(lines.len() <= LINE_LIMIT);
        Self { lines, truncated }
    }

    pub fn html(&self, expanded: bool) -> String {
        self.lines
            .iter()
            .take(if expanded {
                self.lines.len()
            } else {
                PREVIEW_LINES
            })
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_are_safe_and_long_output_expands() {
        let text = format!("\x1b[31m<script>& danger\x1b[0m\n{}", "line\n".repeat(30));
        let output = ToolOutputContent::parse(&text);
        assert!(output.html(false).contains("term-red"));
        assert!(output.html(false).contains("&lt;script&gt;&amp; danger"));
        assert!(!output.html(true).contains("<script>"));
        assert!(!output.html(true).contains('\x1b'));
        assert_eq!(output.html(false).lines().count(), PREVIEW_LINES);
        assert!(output.html(true).len() > output.html(false).len());
        assert!(!output.truncated);
    }
    #[test]
    fn controls_and_retention_follow_the_terminal_contract() {
        let output =
            ToolOutputContent::parse("old\rnew\x1b]8;;javascript:alert(1)\x07link\x1b]8;;\x07");
        assert_eq!(output.html(true), "newlink");
        let output = ToolOutputContent::parse(&"line\n".repeat(LINE_LIMIT + 10));
        assert!(output.truncated);
        assert_eq!(output.lines.len(), LINE_LIMIT);
    }
}
