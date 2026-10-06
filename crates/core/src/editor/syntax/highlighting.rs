//! Shared grammar-aware paint; source bytes and line boundaries are preserved.
use super::{SyntaxDocument, visit_tree};
use crate::editor::structure::RegionKind;
use crate::highlight::{Language, Token, TokenKind, highlight_lines};
use std::collections::BTreeSet;

impl SyntaxDocument {
    /// None requests the ordinary bounded lexical fallback after failed analysis.
    pub fn highlight_lines(&self) -> Option<Vec<Vec<Token>>> {
        let context = self.structure()?;
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
            for (index, tokens) in painted.into_iter().enumerate() {
                if index > 0 {
                    lines.push(Vec::new());
                }
                lines
                    .last_mut()?
                    .extend(tokens.into_iter().filter(|token| !token.text.is_empty()));
            }
        }
        for (line, source) in lines.iter_mut().zip(self.text.split('\n')) {
            if source.len() > crate::highlight::MAX_HIGHLIGHT_LINE_BYTES {
                *line = vec![Token {
                    kind: TokenKind::Plain,
                    text: source.into(),
                }];
            }
        }
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
