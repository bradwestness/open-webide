//! Lexical structure for editing commands. Strings and comments are opaque;
//! unsupported languages retain plain-text editing rather than guessing syntax.
use std::ops::Range;
use std::sync::Arc;

use crate::highlight::Language;

/// Bound synchronous structural commands until incremental parsing is available.
pub const MAX_STRUCTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_BRACKETS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RegionKind {
    String,
    Template,
    #[cfg(feature = "editor-parser")]
    Text,
    LineComment,
    BlockComment,
    Regex,
}

impl RegionKind {
    fn is_literal(self) -> bool {
        match self {
            Self::String | Self::Template | Self::Regex => true,
            #[cfg(feature = "editor-parser")]
            Self::Text => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Structure {
    source: Arc<str>,
    language: Language,
    pub(super) scopes: Vec<(Range<usize>, Language)>,
    selection_ranges: Vec<Range<usize>>,
    pub(super) opaque_starts: Vec<usize>,
    available: bool,
    pub(super) protected: Vec<(Range<usize>, bool, RegionKind)>,
    pub brackets: Vec<(usize, char, Option<usize>)>,
}

impl Structure {
    pub fn new(text: &str, language: Language) -> Self {
        if language == Language::Plain || text.len() > MAX_STRUCTURE_BYTES {
            return Self::unavailable();
        }
        let mut protected = Vec::new();
        let mut opaque_starts = Vec::new();
        let mut template_holes = Vec::new();
        let mut brackets: Vec<(usize, char, Option<usize>)> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            let hash_comments = matches!(
                language,
                Language::Python
                    | Language::Shell
                    | Language::Toml
                    | Language::Yaml
                    | Language::Php
            );
            let slash_comments = super::line_comment(language) == Some("//");
            let line_comment = (hash_comments && rest.starts_with('#'))
                || (slash_comments && rest.starts_with("//"))
                || (language == Language::Sql && rest.starts_with("--"));
            if line_comment {
                let end = rest.find('\n').map_or(text.len(), |n| i + n);
                protected.push((i..end, false, RegionKind::LineComment));
                i = end;
                continue;
            }
            let block = if (slash_comments || matches!(language, Language::Css | Language::Sql))
                && rest.starts_with("/*")
            {
                Some(("/*", "*/", language == Language::Rust))
            } else if matches!(language, Language::Html | Language::Markdown)
                && rest.starts_with("<!--")
            {
                Some(("<!--", "-->", false))
            } else {
                None
            };
            if let Some((open, close, nested)) = block {
                let start = i;
                i += open.len();
                let mut depth = 1;
                while i < text.len() && depth > 0 {
                    if nested && text[i..].starts_with(open) {
                        depth += 1;
                        i += open.len();
                    } else if text[i..].starts_with(close) {
                        depth -= 1;
                        i += close.len();
                    } else {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                }
                protected.push((start..i, depth == 0, RegionKind::BlockComment));
                continue;
            }
            // Rust raw strings use a matching number of hashes, not backslash escapes.
            if language == Language::Rust && (rest.starts_with('r') || rest.starts_with("br")) {
                let prefix = if rest.starts_with("br") { 2 } else { 1 };
                let hashes = rest[prefix..].bytes().take_while(|b| *b == b'#').count();
                if rest.as_bytes().get(prefix + hashes) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let start = i;
                    i += prefix + hashes + 1;
                    let found = text[i..].find(&close);
                    i = found.map_or(text.len(), |n| i + n + close.len());
                    protected.push((start..i, found.is_some(), RegionKind::String));
                    continue;
                }
            }
            // A slash in expression-start position can begin a JavaScript regex.
            // Character classes and escaped slashes do not end the literal.
            if matches!(
                language,
                Language::JavaScript | Language::TypeScript | Language::Jsx | Language::Tsx
            ) && rest.starts_with('/')
                && regex_position(&text[..i])
            {
                let start = i;
                i += 1;
                let mut class = false;
                let mut closed = false;
                while i < text.len() && text.as_bytes()[i] != b'\n' {
                    let ch = text[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                    match ch {
                        '\\' => {
                            if i < text.len() {
                                i += text[i..].chars().next().unwrap().len_utf8();
                            }
                        }
                        '[' => class = true,
                        ']' => class = false,
                        '/' if !class => {
                            closed = true;
                            break;
                        }
                        _ => {}
                    }
                }
                protected.push((start..i, closed, RegionKind::Regex));
                continue;
            }
            let ch = rest.chars().next().unwrap();
            if language == Language::Yaml
                && matches!(ch, '|' | '>')
                && let Some(end) = yaml_scalar_end(text, i)
            {
                protected.push((i..end, true, RegionKind::String));
                i = end;
                continue;
            }
            let quotes = supports_quote(language, ch);
            // An apostrophe before a Rust identifier is a lifetime unless a closing
            // apostrophe follows its single character (possibly an escape).
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
                let (end, closed, hole) = template_fragment(text, i, true);
                protected.push((i..end, closed, RegionKind::Template));
                if let Some(hole) = hole {
                    template_holes.push(hole);
                    i = hole;
                } else {
                    i = end;
                }
                continue;
            }
            if quotes && !lifetime {
                let start = i;
                let triple =
                    language == Language::Python && rest.starts_with(&ch.to_string().repeat(3));
                let delimiter = ch.to_string().repeat(if triple { 3 } else { 1 });
                i += delimiter.len();
                let mut closed = false;
                while i < text.len() {
                    if text[i..].starts_with(&delimiter) {
                        i += delimiter.len();
                        closed = true;
                        break;
                    }
                    if text.as_bytes()[i] == b'\\' && !(language == Language::Go && ch == '`') {
                        i += 1;
                        if i < text.len() {
                            i += text[i..].chars().next().unwrap().len_utf8();
                        }
                    } else {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                }
                protected.push((start..i, closed, RegionKind::String));
                continue;
            }
            if supports_brackets(language) && matches!(ch, '(' | '[' | '{' | ')' | ']' | '}') {
                if brackets.len() == MAX_BRACKETS {
                    return Self::unavailable();
                }
                let index = brackets.len();
                brackets.push((i, ch, None));
                if matches!(ch, '(' | '[' | '{') {
                    stack.push(index);
                } else if let Some(&open) = stack.last()
                    && closing(brackets[open].1) == Some(ch)
                {
                    stack.pop();
                    brackets[open].2 = Some(i);
                    brackets[index].2 = Some(brackets[open].0);
                    if template_holes.last() == Some(&brackets[open].0) {
                        template_holes.pop();
                        let start = i + ch.len_utf8();
                        let (end, closed, hole) = template_fragment(text, start, false);
                        if start < end {
                            opaque_starts.push(start);
                            protected.push((start..end, closed, RegionKind::Template));
                        }
                        if let Some(hole) = hole {
                            template_holes.push(hole);
                            i = hole;
                        } else {
                            i = end;
                        }
                        continue;
                    }
                }
            }
            i += ch.len_utf8();
        }
        Self {
            source: text.into(),
            language,
            scopes: Vec::new(),
            selection_ranges: Vec::new(),
            opaque_starts,
            available: true,
            protected,
            brackets,
        }
    }

    fn unavailable() -> Self {
        Self {
            source: "".into(),
            language: Language::Plain,
            scopes: Vec::new(),
            selection_ranges: Vec::new(),
            opaque_starts: Vec::new(),
            available: false,
            protected: Vec::new(),
            brackets: Vec::new(),
        }
    }

    pub fn matches_source(&self, text: &str) -> bool {
        self.source.as_ref() == text
    }

    pub fn opens_interpolation(&self, position: usize, ch: char) -> bool {
        if ch != '{'
            || !matches!(
                self.language_at(position),
                Language::JavaScript | Language::TypeScript | Language::Jsx | Language::Tsx
            )
        {
            return false;
        }
        let Some(prefix) = self
            .source
            .get(..position)
            .and_then(|prefix| prefix.strip_suffix('$'))
        else {
            return false;
        };
        prefix.chars().rev().take_while(|ch| *ch == '\\').count() % 2 == 0
            && self
                .region_at(position)
                .is_some_and(|(range, closed, kind)| {
                    *kind == RegionKind::Template
                        && range.start < position
                        && (range.contains(&position) || !closed && range.end == position)
                })
    }

    /// Next character within the current language body.
    pub fn next_character(&self, position: usize) -> Option<char> {
        if self.scopes.iter().any(|(range, _)| range.end == position) {
            return None;
        }
        self.source.get(position..)?.chars().next()
    }

    /// The language of an insertion, including the end of an embedded body.
    pub fn language_at(&self, position: usize) -> Language {
        self.scopes
            .iter()
            .find(|(range, _)| range.start <= position && position <= range.end)
            .map_or(self.language, |(_, language)| *language)
    }

    /// Separate embedded bodies cannot share indentation or bracket ancestry,
    /// even when both use the same language.
    pub fn same_language_body(&self, first: usize, second: usize) -> bool {
        let body = |position| {
            self.scopes
                .iter()
                .position(|(range, _)| range.contains(&position))
        };
        body(first) == body(second)
    }

    /// The bounds of the embedded language containing a caret. Root-language
    /// commands retain the document bounds.
    pub fn language_body(&self, position: usize) -> Range<usize> {
        self.scopes
            .iter()
            .find(|(range, _)| range.start <= position && position <= range.end)
            .map_or(0..self.source.len(), |(range, _)| range.clone())
    }

    #[cfg(feature = "editor-parser")]
    pub(super) fn parsed(
        text: Arc<str>,
        language: Language,
        mut protected: Vec<(Range<usize>, bool, RegionKind)>,
        scopes: Vec<(Range<usize>, Language)>,
        mut opaque_starts: Vec<usize>,
        mut selection_ranges: Vec<Range<usize>>,
    ) -> Option<Self> {
        protected.sort_by_key(|(range, _, _)| (range.start, range.end));
        let mut end = 0;
        for (range, _, _) in &protected {
            if range.start < end
                || range.start > range.end
                || !text.is_char_boundary(range.start)
                || !text.is_char_boundary(range.end)
            {
                return None;
            }
            end = range.end;
        }
        if selection_ranges.iter().any(|range| {
            range.start >= range.end
                || !text.is_char_boundary(range.start)
                || !text.is_char_boundary(range.end)
        }) {
            return None;
        }
        selection_ranges.sort_by_key(|range| (range.start, range.end));
        selection_ranges.dedup();
        opaque_starts.sort_unstable();
        opaque_starts.dedup();
        let mut result = Self {
            source: text.clone(),
            language,
            scopes,
            selection_ranges,
            opaque_starts,
            available: true,
            protected,
            brackets: Vec::new(),
        };
        let mut stack = Vec::<usize>::new();
        let mut last_scope = None;
        let mut scope_index = 0;
        for (position, ch) in text.char_indices() {
            while result
                .scopes
                .get(scope_index)
                .is_some_and(|(range, _)| range.end <= position)
            {
                scope_index += 1;
            }
            let scope = result
                .scopes
                .get(scope_index)
                .filter(|(range, _)| range.contains(&position))
                .map(|_| scope_index);
            let current_language = scope.map_or(language, |index| result.scopes[index].1);
            if scope != last_scope {
                stack.clear();
                last_scope = scope;
            }
            if !supports_brackets(current_language)
                || result
                    .region_at(position)
                    .is_some_and(|(range, _, _)| range.contains(&position))
            {
                continue;
            }
            if closing(ch).is_some() {
                if result.brackets.len() == MAX_BRACKETS {
                    return None;
                }
                stack.push(result.brackets.len());
                result.brackets.push((position, ch, None));
            } else if matches!(ch, ')' | ']' | '}') {
                if result.brackets.len() == MAX_BRACKETS {
                    return None;
                }
                let index = result.brackets.len();
                result.brackets.push((position, ch, None));
                if let Some(open) = stack.last().copied()
                    && closing(result.brackets[open].1) == Some(ch)
                {
                    stack.pop();
                    result.brackets[open].2 = Some(position);
                    result.brackets[index].2 = Some(result.brackets[open].0);
                }
            }
        }
        Some(result)
    }

    pub(super) fn selection_ranges(&self) -> impl Iterator<Item = &Range<usize>> {
        self.selection_ranges.iter()
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub(super) fn literals(&self) -> impl Iterator<Item = &Range<usize>> {
        self.protected.iter().filter_map(|(range, _, kind)| {
            matches!(
                kind,
                RegionKind::String | RegionKind::Template | RegionKind::BlockComment
            )
            .then_some(range)
        })
    }

    pub(super) fn is_literal(&self, position: usize) -> bool {
        self.region_at(position)
            .is_some_and(|(range, _, kind)| kind.is_literal() && range.contains(&position))
    }

    pub(super) fn is_opaque_body(&self, position: usize) -> bool {
        self.region_at(position).is_some_and(|(range, _, _)| {
            (range.start < position || self.opaque_starts.binary_search(&position).is_ok())
                && range.contains(&position)
        })
    }

    pub(super) fn is_line_comment(&self, position: usize) -> bool {
        self.region_at(position).is_some_and(|(range, _, kind)| {
            *kind == RegionKind::LineComment && range.contains(&position)
        })
    }

    pub(super) fn is_comment(&self, position: usize) -> bool {
        self.region_at(position).is_some_and(|(range, _, kind)| {
            matches!(kind, RegionKind::LineComment | RegionKind::BlockComment)
                && range.contains(&position)
        })
    }

    fn region_at(&self, position: usize) -> Option<&(Range<usize>, bool, RegionKind)> {
        let end = self
            .protected
            .partition_point(|(range, _, _)| range.start <= position);
        end.checked_sub(1)
            .and_then(|index| self.protected.get(index))
    }

    pub fn is_code(&self, position: usize) -> bool {
        self.available
            && !self.region_at(position).is_some_and(|(range, closed, _)| {
                (position > range.start || self.opaque_starts.binary_search(&position).is_ok())
                    && (position < range.end || (!closed && position == range.end))
            })
    }

    pub fn allows_newline_indent(&self, position: usize) -> bool {
        self.is_code(position)
            || self.protected.iter().any(|(range, _, kind)| {
                *kind == RegionKind::LineComment && position > range.start && position <= range.end
            })
    }

    pub fn quote_closes_at(&self, position: usize) -> bool {
        self.protected.iter().any(|(range, closed, kind)| {
            matches!(kind, RegionKind::String | RegionKind::Template)
                && *closed
                && range.end == position + 1
        })
    }
    pub fn empty_quote_pair(&self, position: usize) -> bool {
        self.protected.iter().any(|(range, closed, _)| {
            *closed && range.start + 1 == position && range.end == position + 1
        })
    }

    pub fn last_code(&self, text: &str, range: Range<usize>) -> Option<(usize, char)> {
        if !self.available {
            return None;
        }
        text[range.clone()]
            .char_indices()
            .rev()
            .find_map(|(offset, ch)| {
                let position = range.start + offset;
                (!ch.is_whitespace()
                    && !self
                        .region_at(position)
                        .is_some_and(|(range, _, _)| range.contains(&position)))
                .then_some((position, ch))
            })
    }
}

// YAML block-scalar contents are literal even when they resemble comments or
// code. Dedentation ends the value; blank rows do not.
fn yaml_scalar_end(text: &str, offset: usize) -> Option<usize> {
    let start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let before = &text[start..offset];
    if !before.trim_end().ends_with([':', '-']) {
        return None;
    }
    let newline = text[offset..].find('\n').map(|newline| offset + newline)?;
    let indicator = text[offset + 1..newline].split('#').next()?.trim();
    if !indicator
        .chars()
        .all(|ch| matches!(ch, '+' | '-' | '1'..='9'))
    {
        return None;
    }
    let depth = before
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count();
    let mut end = newline + 1;
    for line in text[end..].split_inclusive('\n') {
        let body = line.trim();
        let indent = line
            .chars()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .count();
        if !body.is_empty() && indent <= depth {
            break;
        }
        end += line.len();
    }
    Some(end)
}

/// Scan literal text up to a closing backtick or an interpolation opening brace.
fn template_fragment(text: &str, start: usize, opening: bool) -> (usize, bool, Option<usize>) {
    let mut position = start + usize::from(opening);
    while position < text.len() {
        let rest = &text[position..];
        if rest.starts_with("${") {
            return (position, false, Some(position + 1));
        }
        let ch = rest.chars().next().unwrap();
        position += ch.len_utf8();
        if ch == '`' {
            return (position, true, None);
        }
        if ch == '\\' && position < text.len() {
            position += text[position..].chars().next().unwrap().len_utf8();
        }
    }
    (position, false, None)
}

fn regex_position(before: &str) -> bool {
    let before = before.trim_end();
    before.is_empty()
        || before.chars().last().is_some_and(|ch| {
            matches!(
                ch,
                '=' | '(' | '[' | '{' | ',' | ':' | ';' | '!' | '?' | '&' | '|'
            )
        })
        || before
            .rsplit(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .next()
            .is_some_and(|word| matches!(word, "return" | "throw" | "case" | "yield" | "await"))
}

pub fn supports_quote(language: Language, ch: char) -> bool {
    match language {
        Language::Plain | Language::Markdown => false,
        Language::Json => ch == '"',
        Language::JavaScript
        | Language::TypeScript
        | Language::Jsx
        | Language::Tsx
        | Language::Go
        | Language::Shell => {
            matches!(ch, '\'' | '"' | '`')
        }
        _ => matches!(ch, '\'' | '"'),
    }
}

pub fn supports_brackets(language: Language) -> bool {
    matches!(
        language,
        Language::Rust
            | Language::Python
            | Language::JavaScript
            | Language::TypeScript
            | Language::Jsx
            | Language::Tsx
            | Language::Java
            | Language::CSharp
            | Language::Php
            | Language::Json
            | Language::C
            | Language::Cpp
            | Language::Go
            | Language::Css
    )
}

pub fn closing(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lexical_structure_ignores_multiline_comments_raw_strings_and_lifetimes() {
        let text = "fn f<'a>() { /* { /* } */ } */ let x = r##\"}\"##; '\\''; }";
        let syntax = Structure::new(text, Language::Rust);
        let braces: Vec<_> = syntax
            .brackets
            .iter()
            .filter(|(_, ch, _)| matches!(ch, '{' | '}'))
            .collect();
        assert_eq!(braces.len(), 2);
        assert_eq!(braces[0].2, Some(braces[1].0));
        assert!(!Structure::new("\"unterminated", Language::Rust).is_code(13));
        assert!(Structure::new("\"closed\"", Language::Rust).is_code(8));
    }
    #[test]
    fn regex_literals_and_go_escapes_are_opaque_and_work_is_bounded() {
        let syntax = Structure::new(
            "const r = /[{}\\/]/; const x = 1 / 2; {}",
            Language::JavaScript,
        );
        assert_eq!(syntax.brackets.len(), 2);
        let syntax = Structure::new("s := \"escaped \\\" } \"; {}", Language::Go);
        assert_eq!(syntax.brackets.len(), 2);
        assert!(!Structure::new(&"x".repeat(MAX_STRUCTURE_BYTES + 1), Language::Rust).available());
        assert!(!Structure::new(&"{".repeat(MAX_BRACKETS + 1), Language::Rust).available());
    }

    #[test]
    fn python_triple_strings_and_unsupported_text_do_not_supply_brackets() {
        let syntax = Structure::new("'''\n{ }\n'''\nx = [1] # }", Language::Python);
        assert_eq!(syntax.brackets.len(), 2);
        assert!(
            Structure::new("{text}", Language::Plain)
                .brackets
                .is_empty()
        );
    }
}
