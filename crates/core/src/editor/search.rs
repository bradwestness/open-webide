//! Bounded source search and atomic replacement, independent of DOM and transport.
use std::ops::Range;

use regex::{Regex, RegexBuilder};

use super::{Document, Edit, EditError, Selection};

const MAX_MATCHES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            whole_word: false,
            regex: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchError {
    Pattern(String),
    InvalidScope,
    ChangedDocument,
    ReadOnly,
    TooLarge,
    TooManyMatches,
    OutputTooLarge,
    Edit(EditError),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pattern(error) => write!(f, "Invalid search pattern: {error}"),
            Self::InvalidScope => f.write_str("Search selection is no longer valid"),
            Self::ChangedDocument => f.write_str("The document changed; run the search again"),
            Self::ReadOnly => f.write_str("This document cannot be edited right now"),
            Self::TooLarge => f.write_str("Search is limited to files up to 2 MiB"),
            Self::TooManyMatches => f.write_str("Search exceeds 100,000 matches; narrow the query"),
            Self::OutputTooLarge => {
                f.write_str("Replacement would exceed the 32 MiB editing limit")
            }
            Self::Edit(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for SearchError {}
impl From<EditError> for SearchError {
    fn from(value: EditError) -> Self {
        Self::Edit(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub range: Range<usize>,
    pub line: usize,
}

/// Compile once per query/options. Empty input means no search, while a regex
/// that can match an empty string retains its finite Unicode-safe matches.
#[derive(Clone, Debug)]
pub struct SearchPattern {
    compiled: Option<Regex>,
    options: SearchOptions,
    capture_names: std::collections::HashMap<String, usize>,
}

impl SearchPattern {
    pub fn new(query: &str, options: SearchOptions) -> Result<Self, SearchError> {
        if query.len() > 64 * 1024 {
            return Err(SearchError::Pattern(
                "Search patterns are limited to 64 KiB".into(),
            ));
        }
        let pattern = if options.regex {
            query.to_string()
        } else {
            regex::escape(query)
        };
        let compiled = if query.is_empty() {
            None
        } else {
            Some(
                RegexBuilder::new(&pattern)
                    .case_insensitive(!options.case_sensitive)
                    .multi_line(true)
                    .crlf(true)
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(1024 * 1024)
                    .nest_limit(128)
                    .build()
                    .map_err(|error| SearchError::Pattern(error.to_string()))?,
            )
        };
        let capture_names = compiled
            .as_ref()
            .map(|pattern| {
                pattern
                    .capture_names()
                    .enumerate()
                    .filter_map(|(index, name)| name.map(|name| (name.to_string(), index)))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            compiled,
            options,
            capture_names,
        })
    }

    fn scope(text: &str, scope: Option<Range<usize>>) -> Result<Range<usize>, SearchError> {
        if text.len() > super::MAX_STRUCTURE_BYTES {
            return Err(SearchError::TooLarge);
        }
        let scope = scope.unwrap_or(0..text.len());
        if scope.start > scope.end
            || scope.end > text.len()
            || !text.is_char_boundary(scope.start)
            || !text.is_char_boundary(scope.end)
        {
            return Err(SearchError::InvalidScope);
        }
        Ok(scope)
    }

    fn includes(&self, text: &str, scope: &Range<usize>, range: &Range<usize>) -> bool {
        if range.start < scope.start || range.end > scope.end {
            return false;
        }
        // Word boundaries are Unicode-aware, including combining marks. Testing
        // the full source avoids inventing boundaries at selection edges.
        !self.options.whole_word || {
            static WORD: std::sync::LazyLock<Regex> =
                std::sync::LazyLock::new(|| Regex::new(r"\w").expect("constant word pattern"));
            let before = text[..range.start].chars().next_back();
            let after = text[range.end..].chars().next();
            let word = |ch: char| WORD.is_match(ch.encode_utf8(&mut [0; 4]));
            !before.is_some_and(word) && !after.is_some_and(word)
        }
    }

    pub fn find(
        &self,
        text: &str,
        scope: Option<Range<usize>>,
    ) -> Result<Vec<SearchMatch>, SearchError> {
        let scope = Self::scope(text, scope)?;
        let Some(pattern) = &self.compiled else {
            return Ok(Vec::new());
        };
        let mut results = Vec::new();
        let mut byte = 0;
        let mut line = 1;
        for matched in pattern.find_iter(text) {
            let range = matched.range();
            if !self.includes(text, &scope, &range) {
                continue;
            }
            if results.len() == MAX_MATCHES {
                return Err(SearchError::TooManyMatches);
            }
            line += text[byte..range.start]
                .bytes()
                .filter(|ch| *ch == b'\n')
                .count();
            byte = range.start;
            results.push(SearchMatch { range, line });
        }
        Ok(results)
    }

    fn replacements(
        &self,
        text: &str,
        scope: Option<Range<usize>>,
        replacement: &str,
        index: Option<usize>,
    ) -> Result<Vec<Edit>, SearchError> {
        let scope = Self::scope(text, scope)?;
        if replacement.len() > super::MAX_DOCUMENT_BYTES {
            return Err(SearchError::OutputTooLarge);
        }
        let Some(pattern) = &self.compiled else {
            return Ok(Vec::new());
        };
        let mut edits = Vec::new();
        let mut count = 0;
        let mut output = text.len();
        for captures in pattern.captures_iter(text) {
            let matched = captures.get(0).expect("capture zero always exists");
            let range = matched.range();
            if !self.includes(text, &scope, &range) {
                continue;
            }
            if count == MAX_MATCHES {
                return Err(SearchError::TooManyMatches);
            }
            let selected = index.is_none_or(|index| index == count);
            count += 1;
            if !selected {
                continue;
            }
            let mut inserted = String::new();
            if self.options.regex {
                let mut exceeded = false;
                regex_automata::util::interpolate::string(
                    replacement,
                    |index, output| {
                        if let Some(capture) = captures.get(index) {
                            if output.len().saturating_add(capture.len())
                                > super::MAX_DOCUMENT_BYTES
                            {
                                exceeded = true;
                            } else if !exceeded {
                                output.push_str(capture.as_str());
                            }
                        }
                    },
                    |name| self.capture_names.get(name).copied(),
                    &mut inserted,
                );
                if exceeded {
                    return Err(SearchError::OutputTooLarge);
                }
            } else {
                inserted.push_str(replacement);
            }
            output = output
                .saturating_sub(range.len())
                .saturating_add(inserted.len());
            if output > super::MAX_DOCUMENT_BYTES {
                return Err(SearchError::OutputTooLarge);
            }
            edits.push(Edit::replace(range, inserted));
            if index.is_some() {
                break;
            }
        }
        Ok(edits)
    }
}

impl Document {
    /// Index selects a current match; None replaces all. All validation and
    /// expansion finish before mutation, so failures preserve history and text.
    pub fn replace_search(
        &mut self,
        pattern: &SearchPattern,
        scope: Option<Range<usize>>,
        replacement: &str,
        index: Option<usize>,
    ) -> Result<usize, SearchError> {
        let edits = pattern.replacements(&self.text, scope, replacement, index)?;
        let count = edits.len();
        if edits.is_empty() {
            return Ok(0);
        }
        let caret = edits[0].range.start + edits[0].text.len();
        self.apply(edits, vec![Selection::caret(caret)], None)?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn regex(query: &str) -> SearchPattern {
        SearchPattern::new(
            query,
            SearchOptions {
                regex: true,
                ..Default::default()
            },
        )
        .unwrap()
    }
    #[test]
    fn case_word_scope_and_unicode_offsets_use_full_source() {
        let pattern = SearchPattern::new(
            "CAFÉ",
            SearchOptions {
                case_sensitive: false,
                whole_word: true,
                regex: false,
            },
        )
        .unwrap();
        let text = "😀 café caféine _café café\u{301}\r\ncafé";
        let results = pattern.find(text, None).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].range, 5..10);
        assert_eq!(results[1].line, 2);
        assert!(pattern.find(text, Some(11..16)).unwrap().is_empty());
        assert_eq!(
            pattern.find(text, Some(1..10)),
            Err(SearchError::InvalidScope)
        );
    }
    #[test]
    fn captures_replace_all_in_one_undo_step_with_source_line_endings() {
        let mut doc = Document::new("x=文\r\ny=😀\r\n");
        doc.set_selections(vec![Selection { anchor: 5, head: 0 }])
            .unwrap();
        let before = doc.clone();
        assert_eq!(
            doc.replace_search(&regex(r"(?P<key>\w)=(.+)"), None, "${key}: $2", None)
                .unwrap(),
            2
        );
        assert_eq!(doc.text(), "x: 文\r\ny: 😀\r\n");
        assert!(doc.undo());
        assert_eq!(doc.text(), before.text());
        assert_eq!(doc.selections(), before.selections());
        assert!(doc.redo());
        assert_eq!(doc.text(), "x: 文\r\ny: 😀\r\n");
    }
    #[test]
    fn zero_width_is_finite_and_does_not_split_unicode_or_crlf() {
        let pattern = regex("^");
        let mut doc = Document::new("文😀\r\nx\r\n");
        assert_eq!(
            pattern
                .find(doc.text(), None)
                .unwrap()
                .iter()
                .map(|m| m.range.clone())
                .collect::<Vec<_>>(),
            vec![0..0, 9..9, 12..12]
        );
        assert_eq!(doc.replace_search(&pattern, None, ">", None).unwrap(), 3);
        assert_eq!(doc.text(), ">文😀\r\n>x\r\n>");
        assert!(doc.undo());
        assert_eq!(doc.text(), "文😀\r\nx\r\n");
    }
    #[test]
    fn next_scope_literal_dollars_and_failures_are_atomic() {
        let pattern = SearchPattern::new("one", SearchOptions::default()).unwrap();
        let mut doc = Document::new("one one one");
        assert_eq!(
            doc.replace_search(&pattern, Some(4..11), "$1", Some(1))
                .unwrap(),
            1
        );
        assert_eq!(doc.text(), "one one $1");
        let before = doc.clone();
        assert_eq!(
            doc.replace_search(&pattern, Some(99..99), "x", None),
            Err(SearchError::InvalidScope)
        );
        assert_eq!(doc, before);
        assert!(
            SearchPattern::new(
                "[",
                SearchOptions {
                    regex: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(
            SearchPattern::new("", SearchOptions::default())
                .unwrap()
                .find("x", None)
                .unwrap()
                .is_empty()
        );
        let many = "a".repeat(MAX_MATCHES + 1);
        assert_eq!(
            regex("a").find(&many, None),
            Err(SearchError::TooManyMatches)
        );
    }
    #[test]
    fn expansion_limits_and_selection_anchors_do_not_mutate_history_on_failure() {
        let mut doc = Document::new("a".repeat(1024));
        let before = doc.clone();
        assert_eq!(
            doc.replace_search(&regex("(.+)"), None, &"$1".repeat(40_000), None),
            Err(SearchError::OutputTooLarge)
        );
        assert_eq!(doc, before);
        assert!(regex("^a").find("xa", Some(1..2)).unwrap().is_empty());
        assert_eq!(regex("a$").find("ax", Some(0..1)).unwrap(), Vec::new());
    }
}
