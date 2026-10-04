//! Incremental, lightweight terminal output with bounded line retention.

use std::collections::VecDeque;
use std::sync::Arc;

pub const LINE_LIMIT: usize = 10_000;
const LINE_CHARACTER_LIMIT: usize = 16_384;
const COLORS: [&str; 16] = [
    "term-black",
    "term-red",
    "term-green",
    "term-yellow",
    "term-blue",
    "term-magenta",
    "term-cyan",
    "term-white",
    "term-bright-black",
    "term-bright-red",
    "term-bright-green",
    "term-bright-yellow",
    "term-bright-blue",
    "term-bright-magenta",
    "term-bright-cyan",
    "term-bright-white",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    color: Option<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    pub id: u64,
    pub html: String,
}

#[derive(Debug, Default)]
enum Escape {
    #[default]
    Text,
    Start,
    Csi(String),
    IgnoreCsi,
    String,
    StringTerminator,
    Intermediate,
}

#[derive(Debug, Default)]
pub struct TerminalOutput {
    completed: VecDeque<Arc<Line>>,
    current: Vec<(char, Style)>,
    cursor: usize,
    style: Style,
    escape: Escape,
    next_id: u64,
    structure_changed: bool,
    truncated: bool,
    #[cfg(test)]
    parsed_characters: usize,
}

impl TerminalOutput {
    pub fn push(&mut self, text: &str) {
        for ch in text.chars() {
            #[cfg(test)]
            {
                self.parsed_characters += 1;
            }
            match std::mem::take(&mut self.escape) {
                Escape::Text => self.text(ch),
                Escape::Start => {
                    self.escape = match ch {
                        '[' => Escape::Csi(String::new()),
                        ']' | 'P' | 'X' | '^' | '_' => Escape::String,
                        '\x20'..='\x2f' => Escape::Intermediate,
                        _ => Escape::Text,
                    };
                }
                Escape::Csi(mut prefix) => {
                    if ('\x40'..='\x7e').contains(&ch) {
                        match ch {
                            'm' => self.sgr(&prefix),
                            'K' => self.erase(&prefix),
                            _ => {}
                        }
                    } else if prefix.len() < 256 {
                        prefix.push(ch);
                        self.escape = Escape::Csi(prefix);
                    } else {
                        self.escape = Escape::IgnoreCsi;
                    }
                }
                Escape::IgnoreCsi => {
                    if !('\x40'..='\x7e').contains(&ch) {
                        self.escape = Escape::IgnoreCsi;
                    }
                }
                Escape::String => {
                    self.escape = match ch {
                        '\x07' | '\u{9c}' => Escape::Text,
                        '\x1b' => Escape::StringTerminator,
                        _ => Escape::String,
                    };
                }
                Escape::StringTerminator => {
                    self.escape = match ch {
                        '\\' | '\x07' => Escape::Text,
                        '\x1b' => Escape::StringTerminator,
                        _ => Escape::String,
                    };
                }
                Escape::Intermediate => {
                    if !('\x30'..='\x7e').contains(&ch) {
                        self.escape = Escape::Intermediate;
                    }
                }
            }
        }
    }

    pub fn push_notice(&mut self, text: &str) {
        // Application notices must not consume a running process's split control sequence.
        let escape = std::mem::take(&mut self.escape);
        self.push(text);
        self.escape = escape;
    }

    pub fn discard_pending_escape(&mut self) {
        self.escape = Escape::Text;
    }

    fn text(&mut self, ch: char) {
        match ch {
            '\x1b' => self.escape = Escape::Start,
            '\u{9b}' => self.escape = Escape::Csi(String::new()),
            '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => self.escape = Escape::String,
            '\n' => {
                let line = Arc::new(Line {
                    id: self.next_id,
                    html: self.current_html(),
                });
                self.next_id += 1;
                self.completed.push_back(line);
                // The mutable current line also occupies one retained line.
                while self.completed.len() >= LINE_LIMIT {
                    self.completed.pop_front();
                    self.truncated = true;
                }
                self.current.clear();
                self.cursor = 0;
                self.structure_changed = true;
            }
            '\r' => self.cursor = 0,
            '\x08' => self.cursor = self.cursor.saturating_sub(1),
            '\t' => self.write(ch),
            ch if ch.is_control() => {}
            ch => self.write(ch),
        }
    }

    fn write(&mut self, ch: char) {
        if self.cursor >= LINE_CHARACTER_LIMIT {
            self.truncated = true;
            return;
        }
        while self.current.len() < self.cursor {
            self.current.push((' ', Style::default()));
        }
        if self.cursor < self.current.len() {
            self.current[self.cursor] = (ch, self.style);
        } else {
            self.current.push((ch, self.style));
        }
        self.cursor += 1;
    }

    fn erase(&mut self, prefix: &str) {
        match prefix {
            "" | "0" => self.current.truncate(self.cursor),
            "1" => {
                for cell in self.current.iter_mut().take(self.cursor + 1) {
                    *cell = (' ', self.style);
                }
            }
            "2" => self.current.clear(),
            _ => {}
        }
    }

    fn sgr(&mut self, prefix: &str) {
        let mut parameters = prefix.split(';');
        while let Some(parameter) = parameters.next() {
            let Ok(code) = parameter.parse::<u16>().or_else(|error| {
                if parameter.is_empty() {
                    Ok(0)
                } else {
                    Err(error)
                }
            }) else {
                continue;
            };
            match code {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                22 => self.style.bold = false,
                30..=37 => self.style.color = u8::try_from(code - 30).ok(),
                90..=97 => self.style.color = u8::try_from(code - 90 + 8).ok(),
                39 => self.style.color = None,
                // Extended colours are outside the existing fixed ANSI class palette.
                38 | 48 | 58 => match parameters.next() {
                    Some("5") => {
                        parameters.next();
                    }
                    Some("2") => {
                        parameters.next();
                        parameters.next();
                        parameters.next();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    pub fn current_html(&self) -> String {
        let mut html = String::new();
        let mut previous = Style::default();
        let mut span = false;
        for &(ch, style) in &self.current {
            if style != previous {
                if span {
                    html.push_str("</span>");
                }
                span = style != Style::default();
                if span {
                    html.push_str("<span class=\"");
                    if style.bold {
                        html.push_str("term-bold ");
                    }
                    if let Some(color) = style.color {
                        html.push_str(COLORS[usize::from(color)]);
                    }
                    html.push_str("\">");
                }
                previous = style;
            }
            match ch {
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '&' => html.push_str("&amp;"),
                '"' => html.push_str("&quot;"),
                '\'' => html.push_str("&#39;"),
                ch => html.push(ch),
            }
        }
        if span {
            html.push_str("</span>");
        }
        html
    }

    pub fn was_truncated(&self) -> bool {
        self.truncated
    }

    pub fn completed(&self) -> impl Iterator<Item = &Arc<Line>> {
        self.completed.iter()
    }

    pub fn take_structure_changed(&mut self) -> bool {
        std::mem::take(&mut self.structure_changed)
    }

    #[cfg(test)]
    fn parsed_characters(&self) -> usize {
        self.parsed_characters
    }

    pub fn clear(&mut self) {
        let next_id = self.next_id;
        *self = Self {
            next_id,
            structure_changed: true,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(output: &TerminalOutput) -> (Vec<(u64, String)>, String) {
        (
            output
                .completed()
                .map(|line| (line.id, line.html.clone()))
                .collect(),
            output.current_html(),
        )
    }

    #[test]
    fn every_character_split_matches_whole_stream() {
        let text = "\x1b[1;31mhé<>&\nred\x1b[22;39m plain\rOK\x1b[K\n123\x08!\x1b]title\x07\x1b[32Jx\x1bPdrop\x1b\\z";
        let mut whole = TerminalOutput::default();
        whole.push(text);
        for split in text.char_indices().map(|(index, _)| index) {
            let mut output = TerminalOutput::default();
            output.push(&text[..split]);
            output.push(&text[split..]);
            assert_eq!(snapshot(&output), snapshot(&whole));
        }
        let mut output = TerminalOutput::default();
        for ch in text.chars() {
            output.push(&ch.to_string());
        }
        assert_eq!(snapshot(&output), snapshot(&whole));
        assert_eq!(whole.parsed_characters(), text.chars().count());
        assert!(
            whole
                .completed()
                .next()
                .unwrap()
                .html
                .contains("term-bold term-red")
        );
        assert!(
            whole
                .completed()
                .next()
                .unwrap()
                .html
                .contains("hé&lt;&gt;&amp;")
        );
        assert_eq!(whole.current_html(), "12!xz");
    }

    #[test]
    fn discarding_pending_escape_preserves_scrollback_and_current_line() {
        for prefix in [
            "\x1b",
            "\x1b[31",
            "\x1b]title",
            "\x1b]title\x1b",
            "\x1bPdata",
            "\x1b(",
        ] {
            let mut output = TerminalOutput::default();
            output.push("old output\n\x1b[32mcurrent");
            let first = output.completed().next().unwrap().clone();
            output.push(prefix);
            output.discard_pending_escape();
            output.push(" continued\x1b[0m\nnotice\nnext output");
            assert!(Arc::ptr_eq(&first, output.completed().next().unwrap()));
            assert_eq!(first.html, "old output");
            assert_eq!(
                output.completed().nth(1).unwrap().html,
                "<span class=\"term-green\">current continued</span>"
            );
            assert_eq!(output.completed().nth(2).unwrap().html, "notice");
            assert_eq!(output.current_html(), "next output");
        }
    }

    #[test]
    fn split_controls_continue_after_interleaved_notices() {
        for (prefix, suffix, expected) in [
            (
                "\x1b]0;first half",
                " second half\x07after title",
                "after title",
            ),
            ("\x1b]title\x1b", "\\after title", "after title"),
            (
                "\x1b[1;3",
                "1mred",
                "<span class=\"term-bold term-red\">red</span>",
            ),
        ] {
            let mut output = TerminalOutput::default();
            output.push("visible\n");
            output.push(prefix);
            output.push_notice("\x1b[31mbusy notice\x1b[0m\n");
            output.push(suffix);
            assert_eq!(output.completed().next().unwrap().html, "visible");
            assert_eq!(
                output.completed().nth(1).unwrap().html,
                "<span class=\"term-red\">busy notice</span>"
            );
            assert_eq!(output.current_html(), expected);
        }
    }

    #[test]
    fn current_edits_styles_and_clear() {
        let mut output = TerminalOutput::default();
        output.push("\x1b[1;31mfirst\nsecond\x1b[0m!\rX\x1b[K");
        let first = output.completed().next().unwrap().clone();
        assert_eq!(output.current_html(), "X");
        output.push("\n\x1b[32mnext");
        assert!(Arc::ptr_eq(&first, output.completed().next().unwrap()));
        assert_ne!(first.id, output.completed().nth(1).unwrap().id);
        output.clear();
        output.push("plain\x1b[\x1b");
        output.clear();
        output.push("<safe>\n");
        assert_eq!(output.completed().next().unwrap().html, "&lt;safe&gt;");
        assert!(output.completed().next().unwrap().id > first.id);
    }

    #[test]
    fn erase_modes_and_sgr_resets_are_cumulative() {
        let mut output = TerminalOutput::default();
        output.push("abcdef\rXY\x1b[1K");
        assert_eq!(output.current_html(), "   def");
        output.push("\x1b[2K!");
        assert_eq!(output.current_html(), "  !");
        output.clear();
        output.push("\x1b[1;91ma\x1b[22mb\x1b[39mc\x1b[32md\x1b[me");
        assert_eq!(
            output.current_html(),
            "<span class=\"term-bold term-bright-red\">a</span><span class=\"term-bright-red\">b</span>c<span class=\"term-green\">d</span>e"
        );
    }

    #[test]
    fn controls_crlf_and_retention() {
        let mut output = TerminalOutput::default();
        output.push("abc\rX\n\x1b[31Jplain\x1b[38;5;1m!\x07");
        assert_eq!(output.completed().next().unwrap().html, "Xbc");
        assert_eq!(output.current_html(), "plain!");
        output.push("\r\n");
        assert_eq!(output.completed().nth(1).unwrap().html, "plain!");
        output.push(&"line\n".repeat(100_000));
        assert_eq!(output.completed().count(), LINE_LIMIT - 1);
        assert_eq!(output.completed().next().unwrap().id, 90_003);
        output.push(&"x".repeat(1_000_000));
        assert_eq!(output.current_html().len(), LINE_CHARACTER_LIMIT);
        output.push("\rOK");
        assert!(output.current_html().starts_with("OK"));
    }
}
