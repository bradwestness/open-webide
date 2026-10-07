//! Shared grammar-aware paint; source bytes and line boundaries are preserved.
use super::SyntaxDocument;
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

/// Match retained row parts without allocating another list for unchanged rows.
struct RowPaint {
    previous: Option<PaintedRow>,
    parts: Vec<(Arc<PaintedPiece>, usize)>,
    matched: usize,
    changed: bool,
    plain: bool,
}

impl RowPaint {
    fn new(
        row: &crate::editor::lines::Line,
        source: &str,
        previous: &mut HashMap<usize, PaintedRow>,
        edit: &super::InputEdit,
    ) -> Self {
        let end = row.end - usize::from(source[row.start..row.end].ends_with('\n'));
        let retained = previous_range(row.start..end, edit).and_then(|range| {
            previous
                .remove(&range.start)
                .filter(|row| row.end == range.end)
        });
        Self {
            previous: retained,
            parts: Vec::new(),
            matched: 0,
            changed: false,
            plain: end - row.start > crate::highlight::MAX_HIGHLIGHT_LINE_BYTES,
        }
    }

    fn append(&mut self, piece: &Arc<PaintedPiece>, index: usize) {
        if self.plain {
            return;
        }
        if !self.changed {
            if self
                .previous
                .as_ref()
                .and_then(|row| row.pieces.get(self.matched))
                .is_some_and(|(old, old_row)| *old_row == index && Arc::ptr_eq(old, piece))
            {
                self.matched += 1;
                return;
            }
            if let Some(row) = &mut self.previous {
                row.pieces.truncate(self.matched);
                self.parts = std::mem::take(&mut row.pieces);
            }
            self.changed = true;
        }
        self.parts.push((piece.clone(), index));
    }

    fn finish(mut self, row: &crate::editor::lines::Line, text: &str) -> PaintedRow {
        let raw = &text[row.start..row.end];
        let source = raw.strip_suffix('\n').unwrap_or(raw);
        let end = row.start + source.len();
        if !self.changed
            && let Some(mut row) = self.previous.take()
        {
            if self.matched == row.pieces.len() {
                row.end = end;
                return row;
            }
            row.pieces.truncate(self.matched);
            self.parts = row.pieces;
        }
        let tokens = if self.plain {
            Arc::from(vec![Token {
                kind: TokenKind::Plain,
                text: source.into(),
            }])
        } else if let [(piece, row)] = self.parts.as_slice() {
            piece.rows[*row].clone()
        } else {
            Arc::from(
                self.parts
                    .iter()
                    .flat_map(|(piece, row)| piece.rows[*row].iter().cloned())
                    .collect::<Vec<_>>(),
            )
        };
        PaintedRow {
            end,
            pieces: self.parts,
            tokens,
        }
    }
}

#[derive(Default)]
pub(super) struct SyntaxPaint {
    pub(super) source: Arc<str>,
    pub(super) source_change: Option<super::InputEdit>,
    pieces: HashMap<usize, Piece>,
    rows: HashMap<usize, PaintedRow>,
    #[cfg(test)]
    repainted_pieces: usize,
    #[cfg(test)]
    reused_source_change: bool,
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
        for (tree, provider, cache) in self
            .tree
            .iter()
            .zip(self.provider)
            .map(|(tree, provider)| (tree, provider, &self.highlights))
            .chain(self.embedded.iter().filter_map(|body| {
                body.tree
                    .as_ref()
                    .map(|tree| (tree, body.provider, &body.highlights))
            }))
        {
            cache
                .borrow_mut()
                .collect(tree, provider, &self.text, &mut visited, &mut semantic)
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
        // Only a source-identity match permits the parser's exact change span
        // to validate retained paint. Updates without paint can leave an older
        // base; compute that wider envelope instead of trusting the last edit.
        let source_change = previous.source_change;
        #[cfg(test)]
        let reused_source_change = source_change.is_some();
        let edit = if Arc::ptr_eq(&previous.source, &self.text) {
            super::input_edit("", "", None).ok()?
        } else if let Some(edit) = source_change {
            edit
        } else {
            super::input_edit(&previous.source, &self.text, None).ok()?
        };
        let mut pieces = HashMap::new();
        #[cfg(test)]
        let mut repainted_pieces = 0;
        let mut rows = HashMap::new();
        let mut tokens = Vec::with_capacity(self.source_lines.len());
        let mut current_row = 0;
        let mut current = Some(RowPaint::new(
            self.source_lines.first()?,
            &self.text,
            &mut previous.rows,
            &edit,
        ));
        let mut finish_row = |paint: RowPaint, row: &crate::editor::lines::Line| {
            let painted = paint.finish(row, &self.text);
            tokens.push(painted.tokens.clone());
            rows.insert(row.start, painted);
        };
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
                    && part.paint.kind == kind)
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
                    finish_row(current.take()?, self.source_lines.get(current_row)?);
                    current_row += 1;
                    current = Some(RowPaint::new(
                        self.source_lines.get(current_row)?,
                        &self.text,
                        &mut previous.rows,
                        &edit,
                    ));
                }
                if !paint.rows[index].is_empty() {
                    current.as_mut()?.append(&paint, index);
                }
            }
            pieces.insert(start, Piece { end, paint });
        }
        if current_row + 1 != self.source_lines.len() {
            return None;
        }
        finish_row(current.take()?, self.source_lines.get(current_row)?);
        *previous = SyntaxPaint {
            source: self.text.clone(),
            source_change: None,
            pieces,
            rows,
            #[cfg(test)]
            repainted_pieces,
            #[cfg(test)]
            reused_source_change,
        };
        Some(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_row_matching_rebuilds_changed_truncated_and_extended_parts() {
        let piece = |text: &str| {
            Arc::new(PaintedPiece {
                language: Language::Rust,
                kind: Some(TokenKind::Keyword),
                rows: share_token_rows(vec![vec![Token {
                    kind: TokenKind::Keyword,
                    text: text.into(),
                }]]),
            })
        };
        let pieces = [piece("a"), piece("b"), piece("c")];
        for indices in [
            vec![0, 1],
            vec![0, 2],
            vec![2, 1],
            vec![0],
            vec![0, 1, 2],
            vec![],
        ] {
            let previous = PaintedRow {
                end: 2,
                pieces: vec![(pieces[0].clone(), 0), (pieces[1].clone(), 0)],
                tokens: Arc::from(vec![
                    pieces[0].rows[0][0].clone(),
                    pieces[1].rows[0][0].clone(),
                ]),
            };
            let original_parts = previous.pieces.as_ptr();
            let original_tokens = previous.tokens.clone();
            let mut paint = RowPaint {
                previous: Some(previous),
                parts: Vec::new(),
                matched: 0,
                changed: false,
                plain: false,
            };
            let source = indices
                .iter()
                .map(|index| pieces[*index].rows[0][0].text.as_str())
                .collect::<String>();
            for index in &indices {
                paint.append(&pieces[*index], 0);
            }
            let row = paint.finish(&crate::editor::lines::lines(&source)[0], &source);
            assert_eq!(row.end, source.len());
            assert_eq!(row.pieces.len(), indices.len());
            assert_eq!(
                row.tokens
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<String>(),
                source
            );
            if indices == [0, 1] {
                assert_eq!(row.pieces.as_ptr(), original_parts);
                assert!(Arc::ptr_eq(&row.tokens, &original_tokens));
            } else {
                assert!(!Arc::ptr_eq(&row.tokens, &original_tokens));
            }
        }
    }

    #[test]
    fn color_classification_reuses_descendants_without_losing_work_limits() {
        let source = (0..200)
            .map(|index| format!("fn f{index}() {{ call(\"文😀\"); }}\r\n"))
            .collect::<String>();
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        document.prepare(&source, 4, || true).1.unwrap();
        let changed = source.replacen("文😀", "😀 changed", 1);
        let next = document.prepare(&changed, 4, || true).1.unwrap();
        let mut nodes = 0;
        super::super::visit_tree(document.tree.as_ref().unwrap(), &mut nodes, |_| Ok(())).unwrap();
        assert!(document.highlights.borrow().reused_nodes > nodes / 2);
        let mut fresh = SyntaxDocument::new(Language::Rust).unwrap();
        assert_eq!(
            next.highlights,
            fresh.prepare(&changed, 4, || true).1.unwrap().highlights
        );
        let mut visited = super::super::MAX_FOLD_NODES - nodes;
        let mut spans = Vec::new();
        document
            .highlights
            .borrow_mut()
            .collect(
                document.tree.as_ref().unwrap(),
                document.provider.unwrap(),
                &changed,
                &mut visited,
                &mut spans,
            )
            .unwrap();
        assert_eq!(visited, super::super::MAX_FOLD_NODES);
        assert_eq!(
            document.highlights.borrow_mut().collect(
                document.tree.as_ref().unwrap(),
                document.provider.unwrap(),
                &changed,
                &mut visited,
                &mut spans
            ),
            Err(super::super::SyntaxStatus::TooLarge)
        );
        document.update(&changed, || false);
        assert_eq!(document.highlights.borrow().reused_nodes, 0);
    }

    #[test]
    fn document_dependent_color_classification_remains_fresh() {
        fn classify(node: super::super::Node<'_>) -> Option<TokenKind> {
            if node.kind() != "identifier" {
                return None;
            }
            let mut root = node;
            while let Some(parent) = root.parent() {
                root = parent;
            }
            root.has_error().then_some(TokenKind::Type)
        }
        let provider = crate::editor::SyntaxProvider {
            highlight: Some(classify),
            highlight_scope: crate::editor::SyntaxHighlightScope::Document,
            ..crate::editor::syntax_provider(Language::Rust).unwrap()
        };
        let source = "fn first() { call(); }\r\nfn second() { call(); }\r\n";
        let mut document = SyntaxDocument::with_provider(Language::Rust, Some(provider)).unwrap();
        let before = document.prepare(source, 4, || true).1.unwrap();
        let changed = format!("{source}fn broken(");
        let next = document.prepare(&changed, 4, || true).1.unwrap();
        assert_ne!(
            before.highlights.as_ref().unwrap()[0],
            next.highlights.as_ref().unwrap()[0]
        );
        let mut fresh = SyntaxDocument::with_provider(Language::Rust, Some(provider)).unwrap();
        assert_eq!(
            next.highlights,
            fresh.prepare(&changed, 4, || true).1.unwrap().highlights
        );
        assert_eq!(document.highlights.borrow().reused_nodes, 0);
    }

    #[test]
    fn grammar_paint_retains_unchanged_piece_and_row_allocations() {
        let source = (0..200)
            .map(|index| format!("fn f{index}() {{\r\n call(\"文😀\");\r\n}}\r\n"))
            .collect::<String>();
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        let original = document.prepare(&source, 4, || true).1.unwrap();
        let pieces = document.paint.borrow().pieces.len();
        let row_allocations = |document: &SyntaxDocument| {
            document
                .source_lines
                .iter()
                .map(|row| {
                    let paint = document.paint.borrow();
                    let parts = &paint.rows[&row.start].pieces;
                    (!parts.is_empty()).then_some(parts.as_ptr() as usize)
                })
                .collect::<Vec<_>>()
        };
        let original_parts = row_allocations(&document);
        let changed = source.replacen("文😀", "😀 changed", 1);
        let next = document.prepare(&changed, 4, || true).1.unwrap();
        let next_parts = row_allocations(&document);
        assert!(
            original_parts
                .iter()
                .zip(&next_parts)
                .filter(|(old, next)| old.is_some() && old == next)
                .count()
                > next_parts.len() / 2
        );
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
        assert!(document.paint.borrow().reused_source_change);
        assert!(original.matches_source(&source));
        let shifted = document
            .prepare(&format!("\r\n{changed}"), 4, || true)
            .1
            .unwrap();
        let shifted_parts = row_allocations(&document);
        assert!(
            next_parts
                .iter()
                .zip(shifted_parts.iter().skip(1))
                .filter(|(old, next)| old.is_some() && old == next)
                .count()
                > next_parts.len() / 2
        );
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
    fn skipped_paint_versions_use_the_retained_source_base() {
        let source = "fn first() { call(\"文😀\"); }\r\nfn second() { call(); }\r\n";
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        let first = document.prepare(source, 4, || true).1.unwrap();
        let middle = source.replacen("文😀", "middle", 1);
        let changed = middle.replace("second", "renamed");
        document.update(&middle, || true);
        document.update(&changed, || true);
        let warm = document.prepare(&changed, 4, || true).1.unwrap();
        assert!(!document.paint.borrow().reused_source_change);
        let mut fresh = SyntaxDocument::new(Language::Rust).unwrap();
        assert_eq!(
            warm.highlights,
            fresh.prepare(&changed, 4, || true).1.unwrap().highlights
        );
        assert!(first.matches_source(source));
        document.update("cancelled", || false);
        assert!(document.paint.borrow().source_change.is_none());
        let restored = document.prepare(source, 4, || true).1.unwrap();
        let mut fresh = SyntaxDocument::new(Language::Rust).unwrap();
        assert_eq!(
            restored.highlights,
            fresh.prepare(source, 4, || true).1.unwrap().highlights
        );
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
