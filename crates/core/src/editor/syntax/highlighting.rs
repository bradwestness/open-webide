//! Shared grammar-aware paint; source bytes and line boundaries are preserved.
use super::{SyntaxDocument, visit_tree};
use crate::editor::structure::RegionKind;
use crate::highlight::{
    Language, Token, TokenKind, TokenRow, TokenRows, highlight_lines, share_token_rows,
};
use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
    sync::Arc,
};

struct PaintedPiece {
    language: Language,
    kind: Option<TokenKind>,
    rows: TokenRows,
}

struct Piece {
    end: usize,
    paint: Arc<PaintedPiece>,
}

struct PaintedRow {
    end: usize,
    pieces: Vec<(Arc<PaintedPiece>, usize)>,
    tokens: TokenRow,
}

#[derive(Default)]
pub(super) struct SyntaxPaint {
    source: Arc<str>,
    pieces: HashMap<usize, Piece>,
    rows: HashMap<usize, PaintedRow>,
    #[cfg(test)]
    repainted_pieces: usize,
}

fn previous_range(range: Range<usize>, edit: &super::InputEdit) -> Option<Range<usize>> {
    if range.end <= edit.start_byte {
        Some(range)
    } else if range.start >= edit.new_end_byte {
        Some(
            range
                .start
                .checked_sub(edit.new_end_byte)?
                .checked_add(edit.old_end_byte)?
                ..range
                    .end
                    .checked_sub(edit.new_end_byte)?
                    .checked_add(edit.old_end_byte)?,
        )
    } else {
        None
    }
}

impl SyntaxDocument {
    /// None requests the ordinary bounded lexical fallback after failed analysis.
    pub fn highlight_lines(&self) -> Option<Vec<Vec<Token>>> {
        let context = self.structure()?;
        self.highlight_with_structure(&context)
            .map(|rows| rows.iter().map(|row| row.to_vec()).collect())
    }

    pub(super) fn highlight_with_structure(
        &self,
        context: &crate::editor::Structure,
    ) -> Option<TokenRows> {
        let mut semantic = Vec::new();
        let mut visited = 0;
        for (tree, provider) in self.tree.iter().zip(self.provider).chain(
            self.embedded
                .iter()
                .filter_map(|body| body.tree.as_ref().map(|tree| (tree, body.provider))),
        ) {
            let Some(select) = provider.highlight else {
                continue;
            };
            visit_tree(tree, &mut visited, |node| {
                if let Some(kind) = select(node) {
                    let range = node.start_byte()..node.end_byte();
                    if range.start < range.end
                        && self.text.is_char_boundary(range.start)
                        && self.text.is_char_boundary(range.end)
                    {
                        semantic.push((range, kind));
                    }
                }
                Ok(())
            })
            .ok()?;
        }
        semantic.sort_by_key(|(range, _)| (range.start, range.end));
        // A selector must classify leaves or disjoint spans. Overlap is a typed
        // analysis fallback, rather than a renderer-dependent priority rule.
        if semantic
            .windows(2)
            .any(|pair| pair[0].0.end > pair[1].0.start)
        {
            return None;
        }
        let mut boundaries = BTreeSet::from([0, self.text.len()]);
        for (range, _, _) in &context.protected {
            boundaries.extend([range.start, range.end]);
        }
        for (range, _) in &context.scopes {
            boundaries.extend([range.start, range.end]);
        }
        for (range, _) in &semantic {
            boundaries.extend([range.start, range.end]);
        }
        let boundaries: Vec<_> = boundaries.into_iter().collect();
        let mut previous = self.paint.borrow_mut();
        let edit = super::input_edit(&previous.source, &self.text);
        let mut pieces = HashMap::new();
        #[cfg(test)]
        let mut repainted_pieces = 0;
        let mut lines = vec![Vec::new()];
        let mut protected = 0;
        let mut selected = 0;
        for range in boundaries.windows(2) {
            let (start, end) = (range[0], range[1]);
            if start == end {
                continue;
            }
            while context
                .protected
                .get(protected)
                .is_some_and(|(range, _, _)| range.end <= start)
            {
                protected += 1;
            }
            while semantic
                .get(selected)
                .is_some_and(|(range, _)| range.end <= start)
            {
                selected += 1;
            }
            let language = context.language_at(self.text.floor_char_boundary(end - 1));
            let literal = context
                .protected
                .get(protected)
                .filter(|(range, _, _)| range.contains(&start));
            let kind = literal
                .map(|(_, _, kind)| match kind {
                    RegionKind::LineComment | RegionKind::BlockComment => TokenKind::Comment,
                    RegionKind::Text => TokenKind::Plain,
                    RegionKind::String
                        if matches!(
                            language,
                            Language::Rust
                                | Language::Java
                                | Language::CSharp
                                | Language::C
                                | Language::Cpp
                                | Language::Go
                        ) && (self.text[start..end].starts_with('\'')
                            || self.text[start..end].starts_with("b'")) =>
                    {
                        TokenKind::Char
                    }
                    _ => TokenKind::String,
                })
                .or_else(|| {
                    semantic
                        .get(selected)
                        .filter(|(range, _)| range.contains(&start))
                        .map(|(_, kind)| *kind)
                });
            let piece = &self.text[start..end];
            let retained = previous_range(start..end, &edit).and_then(|range| {
                let part = previous.pieces.get(&range.start)?;
                (part.end == range.end
                    && part.paint.language == language
                    && part.paint.kind == kind
                    && previous.source.get(range)? == piece)
                    .then(|| part.paint.clone())
            });
            let paint = retained.unwrap_or_else(|| {
                #[cfg(test)]
                {
                    repainted_pieces += 1;
                }
                let painted = if let Some(kind) = kind {
                    piece
                        .split('\n')
                        .map(|text| {
                            vec![Token {
                                kind,
                                text: text.into(),
                            }]
                        })
                        .collect()
                } else {
                    highlight_lines(piece, language)
                };
                Arc::new(PaintedPiece {
                    language,
                    kind,
                    rows: share_token_rows(
                        painted
                            .into_iter()
                            .map(|tokens| {
                                tokens
                                    .into_iter()
                                    .filter(|token| !token.text.is_empty())
                                    .collect()
                            })
                            .collect(),
                    ),
                })
            });
            for index in 0..paint.rows.len() {
                if index > 0 {
                    lines.push(Vec::new());
                }
                if !paint.rows[index].is_empty() {
                    lines.last_mut()?.push((paint.clone(), index));
                }
            }
            pieces.insert(start, Piece { end, paint });
        }
        let mut rows = HashMap::new();
        let mut tokens = Vec::with_capacity(lines.len());
        let mut start = 0;
        for (mut parts, source) in lines.into_iter().zip(self.text.split('\n')) {
            let end = start + source.len();
            let plain = source.len() > crate::highlight::MAX_HIGHLIGHT_LINE_BYTES;
            if plain {
                parts.clear();
            }
            let retained =
                previous_range(start..end, &edit).and_then(|range| {
                    let row = previous.rows.get(&range.start)?;
                    (row.end == range.end
                        && previous.source.get(range)? == source
                        && row.pieces.len() == parts.len()
                        && row.pieces.iter().zip(&parts).all(
                            |((old, old_row), (next, next_row))| {
                                old_row == next_row && Arc::ptr_eq(old, next)
                            },
                        ))
                    .then(|| row.tokens.clone())
                });
            let line = retained.unwrap_or_else(|| {
                if plain {
                    Arc::from(vec![Token {
                        kind: TokenKind::Plain,
                        text: source.into(),
                    }])
                } else if let [(piece, row)] = parts.as_slice() {
                    piece.rows[*row].clone()
                } else {
                    Arc::from(
                        parts
                            .iter()
                            .flat_map(|(piece, row)| piece.rows[*row].iter().cloned())
                            .collect::<Vec<_>>(),
                    )
                }
            });
            tokens.push(line.clone());
            rows.insert(
                start,
                PaintedRow {
                    end,
                    pieces: parts,
                    tokens: line,
                },
            );
            start = end + 1;
        }
        *previous = SyntaxPaint {
            source: self.text.clone(),
            pieces,
            rows,
            #[cfg(test)]
            repainted_pieces,
        };
        Some(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grammar_paint_retains_unchanged_piece_and_row_allocations() {
        let source = (0..200)
            .map(|index| format!("fn f{index}() {{\r\n call(\"文😀\");\r\n}}\r\n"))
            .collect::<String>();
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        let original = document.prepare(&source, 4, || true).1.unwrap();
        let pieces = document.paint.borrow().pieces.len();
        let changed = source.replacen("文😀", "😀 changed", 1);
        let next = document.prepare(&changed, 4, || true).1.unwrap();
        let old_rows = original.highlights.as_ref().unwrap();
        let rows = next.highlights.as_ref().unwrap();
        assert!(
            old_rows
                .iter()
                .zip(rows.iter())
                .filter(|(old, next)| Arc::ptr_eq(old, next))
                .count()
                > rows.len() / 2
        );
        assert!(document.paint.borrow().repainted_pieces < pieces / 10);
        assert!(original.matches_source(&source));
        let shifted = document
            .prepare(&format!("\r\n{changed}"), 4, || true)
            .1
            .unwrap();
        let shifted_rows = shifted.highlights.as_ref().unwrap();
        assert!(
            rows.iter()
                .zip(shifted_rows.iter().skip(1))
                .filter(|(old, next)| Arc::ptr_eq(old, next))
                .count()
                > rows.len() / 2
        );
        document.update("cancelled", || false);
        assert!(document.paint.borrow().pieces.is_empty());
        assert!(document.paint.borrow().rows.is_empty());
        assert!(next.matches_source(&changed));
    }

    #[test]
    fn retained_grammar_paint_matches_cold_colors_after_context_and_scope_edits() {
        for &(path, source, _) in crate::editor::syntax_contracts::LANGUAGE_CASES {
            let language = crate::highlight::language_from_path(path);
            let source = source.repeat(3);
            let mut document = SyntaxDocument::new(language).unwrap();
            for text in [
                source.clone(),
                format!("\r\n{source}"),
                source.replacen("文😀", "😀 changed", 1),
                source.replace('"', ""),
                format!("/*\r\n{source}"),
                source.replace('}', ""),
                source,
            ] {
                let warm = document.prepare(&text, 4, || true).1.unwrap();
                let mut fresh = SyntaxDocument::new(language).unwrap();
                let cold = fresh.prepare(&text, 4, || true).1.unwrap();
                assert_eq!(warm.highlights, cold.highlights, "{path}: {text}");
                assert!(warm.matches_source(&text));
            }
        }
    }

    #[test]
    fn paint_preserves_all_provider_sources_and_line_endings() {
        for &(path, source, _) in crate::editor::syntax_contracts::LANGUAGE_CASES {
            let language = crate::highlight::language_from_path(path);
            let mut document = SyntaxDocument::new(language).unwrap();
            document.update(source, || true);
            let lines = document.highlight_lines().unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line
                        .iter()
                        .map(|token| token.text.as_str())
                        .collect::<String>())
                    .collect::<Vec<_>>(),
                source.split('\n').collect::<Vec<_>>(),
                "{path}"
            );
        }
    }
    #[test]
    fn embedded_code_and_interpolation_receive_their_own_colors() {
        let source =
            "<script>const t = `文 ${call(42)} tail`;</script><style>a { color: red; }</style>";
        let mut document = SyntaxDocument::new(Language::Html).unwrap();
        document.update(source, || true);
        let lines = document.highlight_lines().unwrap();
        let tokens = &lines[0];
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Keyword && token.text == "const")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Function && token.text == "call")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Number && token.text == "42")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Attribute && token.text == "color")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::String && token.text.contains("tail"))
        );
    }

    #[test]
    fn failed_analysis_and_long_lines_retain_shared_fallback_limits() {
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        let source = format!(
            "let s = \"{}\";\nfn next() {{}}",
            "x".repeat(crate::highlight::MAX_HIGHLIGHT_LINE_BYTES)
        );
        document.update(&source, || true);
        let painted = document.highlight_lines().unwrap();
        assert_eq!(painted[0].len(), 1);
        assert_eq!(painted[0][0].kind, TokenKind::Plain);
        assert_eq!(painted[0][0].text, source.split('\n').next().unwrap());
        assert!(
            painted[1]
                .iter()
                .any(|token| token.kind == TokenKind::Function && token.text == "next")
        );
        document.update("changed", || false);
        assert!(document.highlight_lines().is_none());
        document.update(&"x".repeat(crate::editor::MAX_STRUCTURE_BYTES + 1), || true);
        assert!(document.highlight_lines().is_none());
    }
}
