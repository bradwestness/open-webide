//! Lexical structure for editing commands. Strings and comments are opaque;
//! unsupported languages retain plain-text editing rather than guessing syntax.
use std::ops::Range;

use crate::highlight::Language;

/// Bound synchronous structural commands until incremental parsing is available.
pub const MAX_STRUCTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_BRACKETS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegionKind {
    String,
    LineComment,
    BlockComment,
    Regex,
}

#[derive(Clone, Debug)]
pub struct Structure {
    available: bool,
    protected: Vec<(Range<usize>, bool, RegionKind)>,
    pub brackets: Vec<(usize, char, Option<usize>)>,
}

impl Structure {
    pub fn new(text: &str, language: Language) -> Self {
        if language == Language::Plain || text.len() > MAX_STRUCTURE_BYTES {
            return Self::unavailable();
        }
        let mut protected = Vec::new();
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
                protected.push((i..end, end < text.len(), RegionKind::LineComment));
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
                }
            }
            i += ch.len_utf8();
        }
        Self {
            available: true,
            protected,
            brackets,
        }
    }

    fn unavailable() -> Self {
        Self {
            available: false,
            protected: Vec::new(),
            brackets: Vec::new(),
        }
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub(super) fn literals(&self) -> impl Iterator<Item = &Range<usize>> {
        self.protected.iter().filter_map(|(range, _, kind)| {
            matches!(kind, RegionKind::String | RegionKind::BlockComment).then_some(range)
        })
    }

    pub(super) fn is_literal(&self, position: usize) -> bool {
        self.region_at(position).is_some_and(|(range, _, kind)| {
            matches!(kind, RegionKind::String | RegionKind::Regex) && range.contains(&position)
        })
    }

    pub(super) fn is_opaque_body(&self, position: usize) -> bool {
        self.region_at(position)
            .is_some_and(|(range, _, _)| range.start < position && range.contains(&position))
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
                position > range.start
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
            *kind == RegionKind::String && *closed && range.end == position + 1
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
