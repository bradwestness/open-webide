//! Source-relative lexical state shared by complete and yielding structure scans.
use super::{
    Language, LexicalStructure, MAX_BRACKETS, MAX_STRUCTURE_BYTES, RegionKind, closing,
    regex_position, supports_brackets, supports_quote,
};

#[derive(Clone, Copy)]
struct Context {
    last_nonwhite_end: usize,
    line_start: usize,
    indent: usize,
    leading: bool,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            last_nonwhite_end: 0,
            line_start: 0,
            indent: 0,
            leading: true,
        }
    }
}
impl Context {
    fn consume(&mut self, position: usize, ch: char) {
        if !ch.is_whitespace() {
            self.last_nonwhite_end = position + ch.len_utf8();
        }
        if ch == '\n' {
            self.line_start = position + 1;
            self.indent = 0;
            self.leading = true;
        } else if self.leading && matches!(ch, ' ' | '\t') {
            self.indent += 1;
        } else {
            self.leading = false;
        }
    }
}
#[derive(Clone, Copy)]
enum YamlSuffix {
    Leading,
    Flags,
    Trailing,
    Comment,
}

#[derive(Default)]
enum Mode {
    #[default]
    Code,
    YamlHeader {
        start: usize,
        depth: usize,
        restore: Context,
        suffix: YamlSuffix,
    },
    YamlBody {
        start: usize,
        depth: usize,
        row: Context,
        admitted: bool,
    },
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
/// tentative YAML rows replay through the same scanner when they are not literal.
pub(in crate::editor) struct LexicalScan {
    language: Language,
    position: usize,
    context: Context,
    worked: usize,
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
            context: Context::default(),
            worked: 0,
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
        let initial = self.worked;
        loop {
            // A header without a newline is not a scalar. Replay its suffix as code.
            if self.position == text.len() {
                if let Mode::YamlHeader { start, restore, .. } = self.mode {
                    self.position = start + 1;
                    self.context = restore;
                    self.mode = Mode::Code;
                    continue;
                } else {
                    match std::mem::take(&mut self.mode) {
                        Mode::Line { start } => self.protect(start, false, RegionKind::LineComment),
                        Mode::Block { start, .. } => {
                            self.protect(start, false, RegionKind::BlockComment);
                        }
                        Mode::Raw { start, .. } | Mode::Quote { start, .. } => {
                            self.protect(start, false, RegionKind::String);
                        }
                        Mode::Regex { start, .. } => self.protect(start, false, RegionKind::Regex),
                        Mode::Template {
                            start,
                            continuation,
                        } => self.template(start, false, continuation),
                        Mode::YamlBody { start, .. } => {
                            self.protect(start, true, RegionKind::String);
                        }
                        Mode::Code | Mode::RawPrefix { .. } => {}
                        Mode::YamlHeader { .. } => unreachable!(),
                    }
                    self.complete = true;
                    break;
                }
            }
            if self.worked - initial >= max_bytes {
                break;
            }
            let before = self.position;
            if self.step(text).is_none() {
                self.failed = true;
                return None;
            }
            // Replaying a speculative header/row restores its context in step().
            // Charge transitions too, so arbitrarily many zero-byte steps cannot
            // monopolize a batch. Work counts visited bytes, including replay.
            let consumed = self.position.saturating_sub(before);
            self.worked = self.worked.saturating_add(consumed.max(1));
            if self.position >= before {
                for (offset, ch) in text[before..self.position].char_indices() {
                    self.context.consume(before + offset, ch);
                }
            }
        }
        Some(self.complete)
    }
    #[cfg(test)]
    pub(in crate::editor) fn position(&self) -> usize {
        self.position
    }
    #[cfg(test)]
    fn worked(&self) -> usize {
        self.worked
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
            Mode::YamlHeader {
                start,
                depth,
                restore,
                mut suffix,
            } => {
                if ch == '\n' {
                    self.position += 1;
                    let mut row = self.context;
                    row.consume(self.position - 1, ch);
                    self.mode = Mode::YamlBody {
                        start,
                        depth,
                        row,
                        admitted: false,
                    };
                } else {
                    let valid = match suffix {
                        YamlSuffix::Comment => true,
                        _ if ch == '#' => {
                            suffix = YamlSuffix::Comment;
                            true
                        }
                        YamlSuffix::Leading if ch.is_whitespace() => true,
                        YamlSuffix::Leading | YamlSuffix::Flags
                            if matches!(ch, '+' | '-' | '1'..='9') =>
                        {
                            suffix = YamlSuffix::Flags;
                            true
                        }
                        YamlSuffix::Flags | YamlSuffix::Trailing if ch.is_whitespace() => {
                            suffix = YamlSuffix::Trailing;
                            true
                        }
                        _ => false,
                    };
                    if valid {
                        self.position += ch.len_utf8();
                        self.mode = Mode::YamlHeader {
                            start,
                            depth,
                            restore,
                            suffix,
                        };
                    } else {
                        self.position = start + 1;
                        self.context = restore;
                    }
                }
            }
            Mode::YamlBody {
                start,
                depth,
                mut row,
                mut admitted,
            } => {
                if !admitted && !ch.is_whitespace() && self.context.indent <= depth {
                    self.position = row.line_start;
                    self.context = row;
                    self.protect(start, true, RegionKind::String);
                } else {
                    self.position += ch.len_utf8();
                    if ch == '\n' {
                        row = self.context;
                        row.consume(self.position - 1, ch);
                        admitted = false;
                    } else if !ch.is_whitespace() {
                        admitted = true;
                    }
                    self.mode = Mode::YamlBody {
                        start,
                        depth,
                        row,
                        admitted,
                    };
                }
            }
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
            && regex_position(&text[..self.context.last_nonwhite_end])
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
            && self.context.last_nonwhite_end > self.context.line_start
            && text
                .as_bytes()
                .get(self.context.last_nonwhite_end - 1)
                .is_some_and(|byte| matches!(byte, b':' | b'-'))
        {
            let depth = self.context.indent;
            let mut restore = self.context;
            restore.consume(i, ch);
            self.position += 1;
            self.mode = Mode::YamlHeader {
                start: i,
                depth,
                restore,
                suffix: YamlSuffix::Leading,
            };
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
    fn long_yaml_headers_rows_and_dedentation_are_bounded() {
        for (source, expected) in [
            (
                format!(
                    "  key: | #{}\n    literal # []\nnext: true\n",
                    "文😀".repeat(149_000)
                ),
                true,
            ),
            (
                format!("key: |\n  {}\nnext: true\n", "文😀".repeat(149_000)),
                true,
            ),
            (
                format!("key: |{}+\n  value\nnext: true\n", " ".repeat(1_000_000)),
                true,
            ),
            (
                format!("key: |+{}+\n  value\n", " ".repeat(1_000_000)),
                false,
            ),
            (format!("key: |{}", " ".repeat(1_000_000)), false),
            (
                format!("key: |\n{}\n  value\nnext: true\n", " ".repeat(1_000_000)),
                true,
            ),
            (
                format!(
                    "key: |\n  value\n{}next: true\n",
                    "\u{2003}".repeat(300_000)
                ),
                true,
            ),
        ] {
            let mut scan = LexicalScan::new(Language::Yaml);
            let mut batches = 0;
            loop {
                let before = scan.worked();
                let result = scan.advance(&source, 8_192);
                assert!(scan.worked() - before <= 8_196);
                if result == Some(true) {
                    break;
                }
                assert_eq!(result, Some(false));
                assert!(scan.worked() > before);
                batches += 1;
                assert!(batches < 1_000);
            }
            assert!(batches > 100);
            let metadata = scan.finish().unwrap();
            assert_eq!(metadata.protected.len(), usize::from(expected));
            if expected {
                let (range, closed, kind) = &metadata.protected[0];
                assert!(*closed);
                assert_eq!(*kind, RegionKind::String);
                assert_eq!(range.start, source.find('|').unwrap());
                assert_eq!(
                    range.end,
                    source
                        .find("\u{2003}")
                        .or_else(|| source.find("next:"))
                        .unwrap_or(source.len())
                );
            }
        }
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
        let mut cases: Vec<Case> =
            serde_json::from_str(include_str!("scanning_cases.json")).unwrap();
        assert_eq!(cases.len(), 864);
        let yaml: Vec<Case> = serde_json::from_str(include_str!("yaml_cases.json")).unwrap();
        assert_eq!(yaml.len(), 192);
        cases.extend(yaml);
        for case in cases {
            for budget in [1, 2, 3, 7, 64, usize::MAX] {
                let mut scan = LexicalScan::new(case.language);
                loop {
                    let before = scan.worked();
                    let result = scan.advance(&case.source, budget);
                    if result.is_some() {
                        assert!(scan.worked() - before <= budget.saturating_add(4));
                    }
                    if result != Some(false) {
                        break;
                    }
                    assert!(scan.worked() > before, "pending batches make progress");
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
