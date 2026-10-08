//! Source-relative lexical state shared by complete and yielding structure scans.
use super::{
    Language, LexicalStructure, MAX_BRACKETS, MAX_STRUCTURE_BYTES, RegionKind, closing,
    regex_position, supports_brackets, supports_quote, yaml_scalar_end,
};

#[derive(Default)]
enum Mode {
    #[default]
    Code,
    Line {
        start: usize,
    },
    Block {
        start: usize,
        open: &'static str,
        close: &'static str,
        nested: bool,
        depth: usize,
    },
    RawPrefix {
        start: usize,
        hashes: usize,
    },
    Raw {
        start: usize,
        hashes: usize,
        matched: Option<usize>,
    },
    Quote {
        start: usize,
        quote: u8,
        width: usize,
        escapes: bool,
    },
    Regex {
        start: usize,
        class: bool,
    },
    Template {
        start: usize,
        continuation: bool,
    },
}

/// The driver owns immutable source for the duration of a scan. No borrowed node
/// or text survives a batch. UTF-8 characters and fixed delimiters are atomic;
/// YAML scalar lookahead remains a complete-region operation.
pub(in crate::editor) struct LexicalScan {
    language: Language,
    position: usize,
    last_nonwhite_end: usize,
    mode: Mode,
    metadata: LexicalStructure,
    stack: Vec<usize>,
    template_holes: Vec<usize>,
    complete: bool,
    failed: bool,
}
impl LexicalScan {
    pub(in crate::editor) fn new(language: Language) -> Self {
        Self {
            language,
            position: 0,
            last_nonwhite_end: 0,
            mode: Mode::Code,
            metadata: LexicalStructure::default(),
            stack: Vec::new(),
            template_holes: Vec::new(),
            complete: false,
            failed: language == Language::Plain,
        }
    }
    pub(in crate::editor) fn advance(&mut self, text: &str, max_bytes: usize) -> Option<bool> {
        if self.failed || text.len() > MAX_STRUCTURE_BYTES {
            self.failed = true;
            return None;
        }
        if self.complete {
            return Some(true);
        }
        let initial = self.position;
        while self.position < text.len() && self.position - initial < max_bytes {
            let before = self.position;
            if self.step(text).is_none() {
                self.failed = true;
                return None;
            }
            for (offset, ch) in text[before..self.position].char_indices() {
                if !ch.is_whitespace() {
                    self.last_nonwhite_end = before + offset + ch.len_utf8();
                }
            }
        }
        if self.position == text.len() {
            match std::mem::take(&mut self.mode) {
                Mode::Line { start } => self.protect(start, false, RegionKind::LineComment),
                Mode::Block { start, .. } => self.protect(start, false, RegionKind::BlockComment),
                Mode::Raw { start, .. } | Mode::Quote { start, .. } => {
                    self.protect(start, false, RegionKind::String);
                }
                Mode::Regex { start, .. } => self.protect(start, false, RegionKind::Regex),
                Mode::Template {
                    start,
                    continuation,
                } => self.template(start, false, continuation),
                Mode::Code | Mode::RawPrefix { .. } => {}
            }
            self.complete = true;
        }
        Some(self.complete)
    }
    #[cfg(test)]
    pub(in crate::editor) fn position(&self) -> usize {
        self.position
    }
    pub(in crate::editor) fn finish(self) -> Option<LexicalStructure> {
        (self.complete && !self.failed).then_some(self.metadata)
    }
    fn protect(&mut self, start: usize, closed: bool, kind: RegionKind) {
        self.metadata
            .protected
            .push((start..self.position, closed, kind));
    }
    fn template(&mut self, start: usize, closed: bool, continuation: bool) {
        if start < self.position {
            if continuation {
                self.metadata.opaque_starts.push(start);
            }
            self.protect(start, closed, RegionKind::Template);
        }
    }
    fn step(&mut self, text: &str) -> Option<()> {
        let rest = &text[self.position..];
        let ch = rest.chars().next().unwrap();
        match std::mem::take(&mut self.mode) {
            Mode::Code => return self.code(text),
            Mode::Line { start } => {
                if ch == '\n' {
                    self.protect(start, false, RegionKind::LineComment);
                } else {
                    self.position += ch.len_utf8();
                    self.mode = Mode::Line { start };
                }
            }
            Mode::Block {
                start,
                open,
                close,
                nested,
                mut depth,
            } => {
                if nested && rest.starts_with(open) {
                    depth += 1;
                    self.position += open.len();
                } else if rest.starts_with(close) {
                    depth -= 1;
                    self.position += close.len();
                } else {
                    self.position += ch.len_utf8();
                }
                if depth == 0 {
                    self.protect(start, true, RegionKind::BlockComment);
                } else {
                    self.mode = Mode::Block {
                        start,
                        open,
                        close,
                        nested,
                        depth,
                    };
                }
            }
            Mode::RawPrefix { start, mut hashes } => {
                if ch == '#' {
                    hashes += 1;
                    self.position += 1;
                    self.mode = Mode::RawPrefix { start, hashes };
                } else if ch == '"' {
                    self.position += 1;
                    self.mode = Mode::Raw {
                        start,
                        hashes,
                        matched: None,
                    };
                }
            }
            Mode::Raw {
                start,
                hashes,
                matched,
            } => {
                if let Some(count) = matched {
                    if ch == '#' {
                        self.position += 1;
                        if count + 1 == hashes {
                            self.protect(start, true, RegionKind::String);
                        } else {
                            self.mode = Mode::Raw {
                                start,
                                hashes,
                                matched: Some(count + 1),
                            };
                        }
                    } else {
                        self.mode = Mode::Raw {
                            start,
                            hashes,
                            matched: None,
                        };
                    }
                } else {
                    self.position += ch.len_utf8();
                    if ch == '"' && hashes == 0 {
                        self.protect(start, true, RegionKind::String);
                    } else {
                        self.mode = Mode::Raw {
                            start,
                            hashes,
                            matched: (ch == '"').then_some(0),
                        };
                    }
                }
            }
            Mode::Quote {
                start,
                quote,
                width,
                escapes,
            } => {
                if rest
                    .as_bytes()
                    .get(..width)
                    .is_some_and(|bytes| bytes.iter().all(|byte| *byte == quote))
                {
                    self.position += width;
                    self.protect(start, true, RegionKind::String);
                } else {
                    self.position += ch.len_utf8();
                    if ch == '\\' && escapes {
                        self.escape(text);
                    }
                    self.mode = Mode::Quote {
                        start,
                        quote,
                        width,
                        escapes,
                    };
                }
            }
            Mode::Regex { start, mut class } => {
                if ch == '\n' {
                    self.protect(start, false, RegionKind::Regex);
                } else {
                    self.position += ch.len_utf8();
                    match ch {
                        '\\' => self.escape(text),
                        '[' => class = true,
                        ']' => class = false,
                        '/' if !class => {
                            self.protect(start, true, RegionKind::Regex);
                            return Some(());
                        }
                        _ => {}
                    }
                    self.mode = Mode::Regex { start, class };
                }
            }
            Mode::Template {
                start,
                continuation,
            } => {
                if rest.starts_with("${") {
                    self.template(start, false, continuation);
                    self.template_holes.push(self.position + 1);
                    self.position += 1;
                } else {
                    self.position += ch.len_utf8();
                    if ch == '`' {
                        self.template(start, true, continuation);
                    } else {
                        if ch == '\\' {
                            self.escape(text);
                        }
                        self.mode = Mode::Template {
                            start,
                            continuation,
                        };
                    }
                }
            }
        }
        Some(())
    }
    fn escape(&mut self, text: &str) {
        if let Some(ch) = text[self.position..].chars().next() {
            self.position += ch.len_utf8();
        }
    }
    fn code(&mut self, text: &str) -> Option<()> {
        let i = self.position;
        let rest = &text[i..];
        let language = self.language;
        let hash_comments = matches!(
            language,
            Language::Python
                | Language::Shell
                | Language::Toml
                | Language::Yaml
                | Language::Ini
                | Language::Php
        );
        let slash_comments = super::super::line_comment(language) == Some("//");
        if (hash_comments && rest.starts_with('#'))
            || (slash_comments && rest.starts_with("//"))
            || (language == Language::Sql && rest.starts_with("--"))
        {
            self.mode = Mode::Line { start: i };
            return Some(());
        }
        let block = if (slash_comments || matches!(language, Language::Css | Language::Sql))
            && rest.starts_with("/*")
        {
            Some(("/*", "*/", language == Language::Rust))
        } else if matches!(
            language,
            Language::Html | Language::Xml | Language::Markdown | Language::MarkdownInline
        ) && rest.starts_with("<!--")
        {
            Some(("<!--", "-->", false))
        } else {
            None
        };
        if let Some((open, close, nested)) = block {
            self.position += open.len();
            self.mode = Mode::Block {
                start: i,
                open,
                close,
                nested,
                depth: 1,
            };
            return Some(());
        }
        if language == Language::Rust && (rest.starts_with('r') || rest.starts_with("br")) {
            self.position += if rest.starts_with("br") { 2 } else { 1 };
            self.mode = Mode::RawPrefix {
                start: i,
                hashes: 0,
            };
            return Some(());
        }
        if matches!(
            language,
            Language::JavaScript | Language::TypeScript | Language::Jsx | Language::Tsx
        ) && rest.starts_with('/')
            && regex_position(&text[..self.last_nonwhite_end])
        {
            self.position += 1;
            self.mode = Mode::Regex {
                start: i,
                class: false,
            };
            return Some(());
        }
        let ch = rest.chars().next().unwrap();
        if language == Language::Yaml
            && matches!(ch, '|' | '>')
            && let Some(end) = yaml_scalar_end(text, i)
        {
            self.position = end;
            self.protect(i, true, RegionKind::String);
            return Some(());
        }
        let lifetime = language == Language::Rust && ch == '\'' && {
            let after = &rest[1..];
            after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && !after.chars().nth(1).is_some_and(|c| c == '\'')
        };
        if ch == '`'
            && matches!(
                language,
                Language::JavaScript | Language::TypeScript | Language::Jsx | Language::Tsx
            )
        {
            self.position += 1;
            self.mode = Mode::Template {
                start: i,
                continuation: false,
            };
            return Some(());
        }
        if supports_quote(language, ch) && !lifetime {
            let quote = u8::try_from(u32::from(ch)).expect("supported quote is ASCII");
            let width = if language == Language::Python
                && rest
                    .as_bytes()
                    .get(..3)
                    .is_some_and(|bytes| bytes.iter().all(|byte| *byte == quote))
            {
                3
            } else {
                1
            };
            self.position += width;
            self.mode = Mode::Quote {
                start: i,
                quote,
                width,
                escapes: !(language == Language::Go && ch == '`'),
            };
            return Some(());
        }
        if supports_brackets(language) && matches!(ch, '(' | '[' | '{' | ')' | ']' | '}') {
            if self.metadata.brackets.len() == MAX_BRACKETS {
                return None;
            }
            let index = self.metadata.brackets.len();
            self.metadata.brackets.push((i, ch, None));
            if matches!(ch, '(' | '[' | '{') {
                self.stack.push(index);
            } else if let Some(&open) = self.stack.last()
                && closing(self.metadata.brackets[open].1) == Some(ch)
            {
                self.stack.pop();
                self.metadata.brackets[open].2 = Some(i);
                self.metadata.brackets[index].2 = Some(self.metadata.brackets[open].0);
                if self.template_holes.last() == Some(&self.metadata.brackets[open].0) {
                    self.template_holes.pop();
                    self.position += 1;
                    self.mode = Mode::Template {
                        start: self.position,
                        continuation: true,
                    };
                    return Some(());
                }
            }
        }
        self.position += ch.len_utf8();
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Case {
        language: Language,
        source: String,
        metadata: serde_json::Value,
    }
    #[test]
    fn scan_limits_and_incomplete_results_survive_yields() {
        let source = "(".repeat(MAX_BRACKETS + 1);
        let mut scan = LexicalScan::new(Language::Rust);
        assert_eq!(scan.advance(&source, 0), Some(false));
        assert_eq!(scan.position(), 0);
        let mut batches = 0;
        while scan.advance(&source, 31) == Some(false) {
            batches += 1;
        }
        assert!(batches > 1_000);
        assert_eq!(scan.metadata.brackets.len(), MAX_BRACKETS);
        assert!(
            scan.finish().is_none(),
            "no partial brackets on limit failure"
        );
        let mut oversized = LexicalScan::new(Language::Rust);
        assert!(
            oversized
                .advance(&"x".repeat(MAX_STRUCTURE_BYTES + 1), 1)
                .is_none()
        );
        assert!(oversized.finish().is_none());
        let mut incomplete = LexicalScan::new(Language::Rust);
        assert_eq!(incomplete.advance("\"incomplete\"", 1), Some(false));
        assert!(
            incomplete.finish().is_none(),
            "a pending literal is not published"
        );
    }

    #[test]
    fn every_batch_boundary_matches_the_original_scanner_oracle() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("scanning_cases.json")).unwrap();
        assert_eq!(cases.len(), 864);
        for case in cases {
            for budget in [1, 2, 3, 7, 64, usize::MAX] {
                let mut scan = LexicalScan::new(case.language);
                loop {
                    let before = scan.position();
                    let result = scan.advance(&case.source, budget);
                    if case.language != Language::Yaml && result.is_some() {
                        assert!(scan.position() - before <= budget.saturating_add(4));
                    }
                    if result != Some(false) {
                        break;
                    }
                    assert!(scan.position() > before, "pending batches make progress");
                    assert!(case.source.is_char_boundary(scan.position()));
                }
                let metadata = scan.finish().map(|value| serde_json::json!({"protected":value.protected,"opaque_starts":value.opaque_starts,"brackets":value.brackets})).unwrap_or(serde_json::Value::Null);
                assert_eq!(
                    metadata, case.metadata,
                    "{:?}, {budget}, {:?}",
                    case.language, case.source
                );
            }
        }
    }
}
