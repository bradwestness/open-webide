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
    let parsed = crate::editor::SyntaxDocument::new(language).and_then(|mut document| {
        document.update(source, || true);
        document.highlight_lines()
    });
    #[cfg(not(feature = "editor-parser"))]
    let parsed: Option<Vec<Vec<Token>>> = None;
    let mut lines = parsed.unwrap_or_else(|| highlight_lines(source, language));
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
                        assert!(
                            tokens
                                .iter()
                                .all(|part| part.token.kind == TokenKind::Comment)
                        );
                    }
                }
            }
        }
        let tokens = paint.line(false, 1, vec![DiffChunk::Unchanged("/*".into())]);
        assert_eq!(tokens[0].token.kind, TokenKind::Comment);
    }
}
