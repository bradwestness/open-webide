//! Full-document syntax colors intersected with word-level change boundaries.
use super::{DiffChunk, FileDiff};
use crate::highlight::{Token, TokenKind, highlight_lines, language_from_path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffChange {
    Unchanged,
    Deleted,
    Inserted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffPaintToken {
    pub token: Token,
    pub change: DiffChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffPaintLine {
    pub marker: char,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub tokens: Vec<DiffPaintToken>,
    pub ending_note: Option<&'static str>,
}

/// Full context keeps source numbering accurate even for compact change previews.
pub fn paint_inline_diff(diff: &FileDiff) -> Vec<DiffPaintLine> {
    let paint = DiffPaint::new(diff);
    let mut old_number = 0;
    let mut new_number = 0;
    super::diff_inline_full(diff)
        .into_iter()
        .map(|line| {
            let old = (line.marker != '+').then(|| {
                old_number += 1;
                old_number
            });
            let new = (line.marker != '-').then(|| {
                new_number += 1;
                new_number
            });
            DiffPaintLine {
                marker: line.marker,
                old_number: old,
                new_number: new,
                tokens: paint.line(
                    line.marker == '-',
                    if line.marker == '-' {
                        old_number
                    } else {
                        new_number
                    },
                    line.chunks,
                ),
                ending_note: line.ending_note,
            }
        })
        .collect()
}

/// Parse each side once; displayed rows select colors by original line number.
pub struct DiffPaint {
    old: Vec<Vec<Token>>,
    new: Vec<Vec<Token>>,
}

impl DiffPaint {
    pub fn new(diff: &FileDiff) -> Self {
        Self {
            old: paint_source(diff.old.as_deref().unwrap_or_default(), &diff.path),
            new: paint_source(&diff.new, &diff.path),
        }
    }

    /// One-based source row, independent of alignment gaps and removed rows.
    /// Missing or mismatched colors fall back without changing the diff text.
    pub fn line(&self, old: bool, number: usize, chunks: Vec<DiffChunk>) -> Vec<DiffPaintToken> {
        let lines = if old { &self.old } else { &self.new };
        let tokens = number.checked_sub(1).and_then(|index| lines.get(index));
        intersect(chunks, tokens.map(Vec::as_slice).unwrap_or_default())
    }
}

fn paint_source(source: &str, path: &str) -> Vec<Vec<Token>> {
    let language = language_from_path(path);
    #[cfg(feature = "editor-parser")]
    let mut lines = crate::editor::SyntaxDocument::new(language)
        .and_then(|mut document| {
            document.update(source, || true);
            document.highlight_lines()
        })
        .unwrap_or_else(|| highlight_lines(source, language));
    #[cfg(not(feature = "editor-parser"))]
    let mut lines = highlight_lines(source, language);
    // DiffLine excludes CRLF terminators. A final bare CR is source content.
    for (line, source) in lines.iter_mut().zip(source.split_inclusive('\n')) {
        if source.ends_with("\r\n")
            && let Some(last) = line.last_mut()
            && last.text.ends_with('\r')
        {
            last.text.pop();
        }
    }
    lines
}

fn intersect(chunks: Vec<DiffChunk>, tokens: &[Token]) -> Vec<DiffPaintToken> {
    let text: String = chunks.iter().map(DiffChunk::text).collect();
    let painted: String = tokens.iter().map(|token| token.text.as_str()).collect();
    let valid = text == painted;
    let mut result = Vec::new();
    let mut token_index = 0;
    let mut token_offset = 0;
    for chunk in chunks {
        let change = match &chunk {
            DiffChunk::Unchanged(_) => DiffChange::Unchanged,
            DiffChunk::Deleted(_) => DiffChange::Deleted,
            DiffChunk::Inserted(_) => DiffChange::Inserted,
        };
        let mut remaining = chunk.text();
        if !valid {
            result.push(DiffPaintToken {
                token: Token {
                    kind: TokenKind::Plain,
                    text: remaining.into(),
                },
                change,
            });
            continue;
        }
        while !remaining.is_empty() {
            while tokens
                .get(token_index)
                .is_some_and(|token| token_offset == token.text.len())
            {
                token_index += 1;
                token_offset = 0;
            }
            let token = &tokens[token_index];
            let count = remaining.len().min(token.text.len() - token_offset);
            // Both boundaries come from valid UTF-8 token/chunk strings sharing
            // the same source; their intersection is a valid source boundary.
            result.push(DiffPaintToken {
                token: Token {
                    kind: token.kind,
                    text: remaining[..count].into(),
                },
                change,
            });
            remaining = &remaining[count..];
            token_offset += count;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_boundaries_preserve_unicode_and_syntax_categories() {
        let tokens = vec![Token {
            kind: TokenKind::String,
            text: "café 😀".into(),
        }];
        let paint = intersect(
            vec![
                DiffChunk::Unchanged("café ".into()),
                DiffChunk::Inserted("😀".into()),
            ],
            &tokens,
        );
        assert_eq!(
            paint
                .iter()
                .map(|part| part.token.text.as_str())
                .collect::<String>(),
            "café 😀"
        );
        assert!(
            paint
                .iter()
                .all(|part| part.token.kind == TokenKind::String)
        );
        assert_eq!(paint[1].change, DiffChange::Inserted);
        let fallback = intersect(vec![DiffChunk::Deleted("other".into())], &tokens);
        assert_eq!(fallback[0].token.text, "other");
        assert_eq!(fallback[0].token.kind, TokenKind::Plain);
        assert_eq!(fallback[0].change, DiffChange::Deleted);
    }

    #[test]
    fn source_numbers_survive_alignment_gaps_and_final_bare_cr() {
        let diff = FileDiff {
            path: "fixture.txt".into(),
            old: Some("keep\r\nold\nlast\r".into()),
            new: "keep\r\nnew\nextra\nlast\r".into(),
            old_unavailable: false,
            backup_path: None,
        };
        let lines = paint_inline_diff(&diff);
        let old: Vec<_> = lines
            .iter()
            .filter_map(|line| {
                line.old_number.map(|number| {
                    (
                        number,
                        line.tokens
                            .iter()
                            .map(|part| part.token.text.as_str())
                            .collect::<String>(),
                    )
                })
            })
            .collect();
        let new: Vec<_> = lines
            .iter()
            .filter_map(|line| {
                line.new_number.map(|number| {
                    (
                        number,
                        line.tokens
                            .iter()
                            .map(|part| part.token.text.as_str())
                            .collect::<String>(),
                    )
                })
            })
            .collect();
        assert_eq!(
            old,
            vec![(1, "keep".into()), (2, "old".into()), (3, "last\r".into())]
        );
        assert_eq!(
            new,
            vec![
                (1, "keep".into()),
                (2, "new".into()),
                (3, "extra".into()),
                (4, "last\r".into())
            ]
        );
        let empty = FileDiff {
            path: "empty".into(),
            old: None,
            new: String::new(),
            old_unavailable: false,
            backup_path: None,
        };
        assert!(paint_inline_diff(&empty).is_empty());
    }

    #[test]
    fn complete_sides_keep_multiline_comments_and_line_endings() {
        let diff = FileDiff {
            path: "fixture.rs".into(),
            old: Some("/*\r\nold café\r\n*/\r\nfn call() {}\r\n".into()),
            new: "/*\r\nnew 😀\r\n*/\r\nfn call() {}\r\n".into(),
            old_unavailable: false,
            backup_path: None,
        };
        let paint = DiffPaint::new(&diff);
        let expected_kind = if cfg!(feature = "editor-parser") {
            TokenKind::Comment
        } else {
            TokenKind::Plain
        };
        let rows = super::super::diff_side_by_side_detailed(&diff);
        let mut old_number = 0;
        let mut new_number = 0;
        for (left, right) in rows {
            for (old, line, number) in [
                (true, left, &mut old_number),
                (false, right, &mut new_number),
            ] {
                if let Some(line) = line {
                    *number += 1;
                    let tokens = paint.line(old, *number, line.chunks);
                    assert_eq!(
                        tokens
                            .iter()
                            .map(|part| part.token.text.as_str())
                            .collect::<String>(),
                        line.content
                    );
                    if *number == 2 {
                        assert!(tokens.iter().all(|part| part.token.kind == expected_kind));
                    }
                }
            }
        }
        let tokens = paint.line(false, 1, vec![DiffChunk::Unchanged("/*".into())]);
        assert_eq!(tokens[0].token.kind, expected_kind);
    }
}

/// Paint a bounded Git patch using the same tokens and gutters as editor diffs.
/// Hunk headers supply original line numbers; metadata never consumes source rows.
pub fn paint_unified_diff(patch: &str, path: &str) -> Vec<DiffPaintLine> {
    let mut rows = Vec::new();
    let (mut old, mut new) = (0, 0);
    let mut hunk = false;
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("@@ -") {
            let ranges = header.split_whitespace().take(2).collect::<Vec<_>>();
            let number = |range: &str| {
                range
                    .trim_start_matches('+')
                    .split(',')
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
            };
            if let [left, right] = ranges.as_slice()
                && let (Some(left), Some(right)) = (number(left), number(right))
            {
                old = left;
                new = right;
                hunk = true;
            }
        } else if line.starts_with("diff --git ") {
            hunk = false;
        }
        let marker = line
            .chars()
            .next()
            .filter(|_| hunk && !line.starts_with("@@ "))
            .unwrap_or('m');
        let (old_number, new_number, text) = match marker {
            '+' => {
                let number = new;
                new += 1;
                (None, Some(number), &line[1..])
            }
            '-' => {
                let number = old;
                old += 1;
                (Some(number), None, &line[1..])
            }
            ' ' => {
                let numbers = (old, new);
                old += 1;
                new += 1;
                (Some(numbers.0), Some(numbers.1), &line[1..])
            }
            _ => (None, None, line),
        };
        rows.push((marker, old_number, new_number, text));
    }
    let old_source = rows
        .iter()
        .filter(|(_, old, _, _)| old.is_some())
        .map(|(_, _, _, text)| *text)
        .collect::<Vec<_>>()
        .join("\n");
    let new_source = rows
        .iter()
        .filter(|(_, _, new, _)| new.is_some())
        .map(|(_, _, _, text)| *text)
        .collect::<Vec<_>>()
        .join("\n");
    let (old_paint, new_paint) = (
        paint_source(&old_source, path),
        paint_source(&new_source, path),
    );
    let (mut old_index, mut new_index) = (0, 0);
    rows.into_iter()
        .map(|(marker, old_number, new_number, text)| {
            let tokens = if marker == '-' {
                old_paint.get(old_index)
            } else if new_number.is_some() {
                new_paint.get(new_index)
            } else {
                None
            };
            let tokens = tokens
                .filter(|tokens| {
                    tokens
                        .iter()
                        .map(|token| token.text.as_str())
                        .collect::<String>()
                        == text
                })
                .cloned()
                .unwrap_or_else(|| {
                    vec![Token {
                        kind: TokenKind::Plain,
                        text: text.into(),
                    }]
                });
            if old_number.is_some() {
                old_index += 1;
            }
            if new_number.is_some() {
                new_index += 1;
            }
            DiffPaintLine {
                marker: if old_number.is_none() && new_number.is_none() {
                    ' '
                } else {
                    marker
                },
                old_number,
                new_number,
                tokens: tokens
                    .into_iter()
                    .map(|token| DiffPaintToken {
                        token,
                        change: DiffChange::Unchanged,
                    })
                    .collect(),
                ending_note: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod patch_tests {
    use super::*;
    #[test]
    fn patch_headers_and_multiple_hunks_preserve_source_numbers_and_text() {
        let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -40,2 +50,2 @@\n-old\n+new\n context\n@@ -100 +120 @@\n-last\n+next\n\\ No newline at end of file";
        let lines = paint_unified_diff(patch, "a.rs");
        assert!(lines[..4].iter().all(|line| line.old_number.is_none()
            && line.new_number.is_none()
            && line.marker == ' '));
        assert_eq!((lines[4].old_number, lines[4].new_number), (Some(40), None));
        assert_eq!((lines[5].old_number, lines[5].new_number), (None, Some(50)));
        assert_eq!(
            (lines[6].old_number, lines[6].new_number),
            (Some(41), Some(51))
        );
        assert_eq!(
            (lines[8].old_number, lines[9].new_number),
            (Some(100), Some(120))
        );
        assert_eq!(
            lines[5]
                .tokens
                .iter()
                .map(|token| token.token.text.as_str())
                .collect::<String>(),
            "new"
        );
        assert!(lines[10].new_number.is_none());
    }
}
