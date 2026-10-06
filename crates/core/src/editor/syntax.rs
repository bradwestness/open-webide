//! Incremental syntax analysis shared by browser and native editor adapters.
use super::{FoldRange, MAX_STRUCTURE_BYTES, SyntaxProvider, normalize_folds, syntax_provider};
use super::{Structure, SyntaxContextKind, structure::RegionKind};
use crate::highlight::Language;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::ops::Range as ByteRange;
use tree_sitter::{InputEdit, Node, ParseOptions, Parser, Point, Range, Tree};

const MAX_PROGRESS_CHECKS: usize = 4_096;
const MAX_FOLD_NODES: usize = 100_000;
const MAX_INJECTIONS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxStatus {
    Ready { incremental: bool },
    TooLarge,
    Cancelled,
}

struct EmbeddedSyntax {
    provider: SyntaxProvider,
    parser: Parser,
    tree: Option<Tree>,
    range: Range,
}

/// One outer parser plus independent embedded bodies. All positions are source coordinates.
pub struct SyntaxDocument {
    parser: Option<Parser>,
    provider: Option<SyntaxProvider>,
    language: Language,
    ready: bool,
    tree: Option<Tree>,
    embedded: Vec<EmbeddedSyntax>,
    text: String,
}

impl SyntaxDocument {
    pub fn new(language: Language) -> Option<Self> {
        Self::with_provider(language, syntax_provider(language))
    }

    pub fn with_provider(language: Language, provider: Option<SyntaxProvider>) -> Option<Self> {
        if provider.is_some_and(|provider| provider.language != language) {
            return None;
        }
        let parser = provider.map(new_parser).transpose().ok()?;
        Some(Self {
            parser,
            provider,
            language,
            ready: false,
            tree: None,
            embedded: Vec::new(),
            text: String::new(),
        })
    }

    pub fn update(
        &mut self,
        text: &str,
        mut should_continue: impl FnMut() -> bool,
    ) -> SyntaxStatus {
        if text.len() > MAX_STRUCTURE_BYTES {
            self.clear();
            return SyntaxStatus::TooLarge;
        }
        if !should_continue() {
            self.clear();
            return SyntaxStatus::Cancelled;
        }
        if self.ready && self.text == text {
            return SyntaxStatus::Ready { incremental: true };
        }
        let Some(parser) = self.parser.as_mut() else {
            self.text.clear();
            self.text.push_str(text);
            self.ready = true;
            return SyntaxStatus::Ready { incremental: false };
        };
        let edit = input_edit(&self.text, text);
        let mut previous = self.tree.clone();
        if let Some(tree) = previous.as_mut() {
            tree.edit(&edit);
        }
        let incremental = previous.is_some();
        let mut checks = 0;
        let result = parse_tree(
            parser,
            text,
            previous.as_ref(),
            &mut checks,
            &mut should_continue,
        )
        .and_then(|tree| {
            let selected = select_injections(&tree, self.provider, text, &mut should_continue)?;
            self.update_embedded(text, selected, &edit, &mut checks, &mut should_continue)?;
            Ok(tree)
        });
        match result {
            Ok(tree) => {
                self.ready = true;
                self.tree = Some(tree);
                self.text.clear();
                self.text.push_str(text);
                SyntaxStatus::Ready { incremental }
            }
            Err(status) => {
                self.clear();
                status
            }
        }
    }

    fn update_embedded(
        &mut self,
        text: &str,
        selected: Vec<(Language, Range)>,
        edit: &InputEdit,
        checks: &mut usize,
        should_continue: &mut impl FnMut() -> bool,
    ) -> Result<(), SyntaxStatus> {
        let mut old = std::mem::take(&mut self.embedded).into_iter();
        let mut next = Vec::with_capacity(selected.len());
        for (language, range) in selected {
            let provider = syntax_provider(language).ok_or(SyntaxStatus::Cancelled)?;
            let mut embedded = if let Some(previous) = old
                .next()
                .filter(|previous| previous.provider.language == language)
            {
                previous
            } else {
                EmbeddedSyntax {
                    provider,
                    parser: new_parser(provider)?,
                    tree: None,
                    range,
                }
            };
            let mut previous = embedded.tree.clone();
            if let Some(tree) = previous.as_mut() {
                tree.edit(edit);
            }
            embedded
                .parser
                .set_included_ranges(&[range])
                .map_err(|_| SyntaxStatus::Cancelled)?;
            embedded.tree = Some(parse_tree(
                &mut embedded.parser,
                text,
                previous.as_ref(),
                checks,
                should_continue,
            )?);
            embedded.range = range;
            next.push(embedded);
        }
        self.embedded = next;
        Ok(())
    }

    /// Insertions at the end of an embedded body still belong to that body.
    pub fn language_at(&self, position: usize) -> Language {
        if self.ready && self.text.is_char_boundary(position) {
            for embedded in &self.embedded {
                if embedded.range.start_byte <= position && position <= embedded.range.end_byte {
                    return embedded.provider.language;
                }
            }
        }
        self.language
    }

    /// Immutable editing contexts from the current tree; failed/stale parses publish nothing.
    pub fn structure(&self) -> Option<Structure> {
        if !self.ready || self.provider.is_none() {
            return None;
        }
        let fallback = Structure::new(&self.text, self.language);
        let mut opaque_starts = fallback.opaque_starts;
        let mut baseline = fallback.protected;
        let scopes: Vec<_> = self
            .embedded
            .iter()
            .map(|body| {
                (
                    body.range.start_byte..body.range.end_byte,
                    body.provider.language,
                )
            })
            .collect();
        baseline.retain(|(range, _, _)| !scopes.iter().any(|(body, _)| overlaps(range, body)));
        opaque_starts.retain(|position| !scopes.iter().any(|(body, _)| body.contains(position)));
        for (body, language) in &scopes {
            let fallback = Structure::new(&self.text[body.clone()], *language);
            opaque_starts.extend(
                fallback
                    .opaque_starts
                    .into_iter()
                    .map(|position| position + body.start),
            );
            baseline.extend(fallback.protected.into_iter().map(|(range, closed, kind)| {
                (
                    (range.start + body.start)..(range.end + body.start),
                    closed,
                    kind,
                )
            }));
        }
        let mut visited = 0;
        let mut captured = Vec::new();
        let mut holes = Vec::new();
        if let (Some(tree), Some(provider)) = (&self.tree, self.provider) {
            collect_contexts(
                tree,
                provider,
                &self.text,
                &mut visited,
                &mut captured,
                &mut holes,
            )
            .ok()?;
        }
        for body in &self.embedded {
            if let Some(tree) = &body.tree {
                collect_contexts(
                    tree,
                    body.provider,
                    &self.text,
                    &mut visited,
                    &mut captured,
                    &mut holes,
                )
                .ok()?;
            }
        }
        let mut coverage: Vec<_> = captured.iter().map(|(range, _, _)| range.clone()).collect();
        coverage.sort_by_key(|range| (range.start, range.end));
        let mut recognized: Vec<ByteRange<usize>> = Vec::new();
        for range in coverage {
            if let Some(previous) = recognized.last_mut()
                && range.start < previous.end
            {
                previous.end = previous.end.max(range.end);
            } else {
                recognized.push(range);
            }
        }
        baseline.retain(|(range, _, _)| {
            let end = recognized.partition_point(|parsed| parsed.start <= range.start);
            !end.checked_sub(1)
                .and_then(|index| recognized.get(index))
                .is_some_and(|parsed| range.end <= parsed.end || parsed.start == range.start)
        });
        let mut by_owner: HashMap<(usize, usize), Vec<ByteRange<usize>>> = HashMap::new();
        for (owner, hole) in holes {
            by_owner
                .entry((owner.start, owner.end))
                .or_default()
                .push(hole);
        }
        for (range, closed, kind) in captured {
            if kind == RegionKind::Text {
                opaque_starts.push(range.start);
            }
            let mut start = range.start;
            let mut owned = by_owner
                .remove(&(range.start, range.end))
                .unwrap_or_default();
            owned.sort_by_key(|hole| hole.start);
            for hole in owned {
                if start < hole.start {
                    baseline.push((start..hole.start, false, kind));
                }
                start = start.max(hole.end);
                opaque_starts.push(start);
            }
            if start < range.end {
                baseline.push((start..range.end, closed, kind));
            }
        }
        baseline.sort_by_key(|(range, _, _)| (range.start, range.end));
        let mut protected: Vec<(ByteRange<usize>, bool, RegionKind)> = Vec::new();
        for (range, closed, kind) in baseline {
            if let Some((previous, previous_closed, _)) = protected.last_mut()
                && range.start < previous.end
            {
                if range.end > previous.end {
                    previous.end = range.end;
                    *previous_closed = closed;
                }
            } else {
                protected.push((range, closed, kind));
            }
        }
        Structure::parsed(&self.text, self.language, protected, scopes, opaque_starts)
    }

    fn clear(&mut self) {
        if let Some(parser) = self.parser.as_mut() {
            parser.reset();
        }
        self.ready = false;
        self.tree = None;
        self.embedded.clear();
        self.text.clear();
    }

    pub fn folds(&self) -> Vec<FoldRange> {
        self.folds_with_tab_width(4)
    }
    pub fn folds_with_tab_width(&self, tab_width: usize) -> Vec<FoldRange> {
        if !self.ready {
            return Vec::new();
        }
        let parsed = self.tree.as_ref().map(|_| self.parser_folds());
        super::fold_ranges(&self.text, self.language, parsed.as_deref(), tab_width)
    }

    fn parser_folds(&self) -> Vec<FoldRange> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let Some(provider) = self.provider else {
            return Vec::new();
        };
        let lines: Vec<_> = self.text.split('\n').collect();
        let mut ranges = Vec::new();
        let mut visited = 0;
        if collect_folds(tree, provider, &lines, &mut ranges, &mut visited).is_err() {
            return Vec::new();
        }
        for embedded in &self.embedded {
            if let Some(tree) = &embedded.tree
                && collect_folds(tree, embedded.provider, &lines, &mut ranges, &mut visited)
                    .is_err()
            {
                return Vec::new();
            }
        }
        normalize_folds(ranges, lines.len())
    }
}

fn overlaps(left: &ByteRange<usize>, right: &ByteRange<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn collect_contexts(
    tree: &Tree,
    provider: SyntaxProvider,
    text: &str,
    visited: &mut usize,
    captured: &mut Vec<(ByteRange<usize>, bool, RegionKind)>,
    holes: &mut Vec<(ByteRange<usize>, ByteRange<usize>)>,
) -> Result<(), SyntaxStatus> {
    let Some(classify) = provider.context else {
        return Ok(());
    };
    let mut ancestry = 0;
    visit_tree(tree, visited, |node| {
        let Some(class) = classify(node) else {
            return Ok(());
        };
        let range = node.start_byte()..node.end_byte();
        if range.start > range.end
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err(SyntaxStatus::Cancelled);
        }
        if range.is_empty() {
            return Ok(());
        }
        if class == SyntaxContextKind::Interpolation {
            let mut parent = node.parent();
            while let Some(owner) = parent {
                ancestry += 1;
                if ancestry > MAX_FOLD_NODES {
                    return Err(SyntaxStatus::TooLarge);
                }
                if matches!(
                    classify(owner),
                    Some(
                        SyntaxContextKind::String
                            | SyntaxContextKind::Template
                            | SyntaxContextKind::Regex
                    )
                ) {
                    holes.push((owner.start_byte()..owner.end_byte(), range));
                    break;
                }
                parent = owner.parent();
            }
            return Ok(());
        }
        let value = &text[range.clone()];
        let (kind, closed) = match class {
            SyntaxContextKind::String => (RegionKind::String, !node.has_error()),
            SyntaxContextKind::Template => (RegionKind::Template, !node.has_error()),
            SyntaxContextKind::Text => (RegionKind::Text, true),
            SyntaxContextKind::Regex => (RegionKind::Regex, !node.has_error()),
            SyntaxContextKind::Comment if value.starts_with("/*") => {
                (RegionKind::BlockComment, value.ends_with("*/"))
            }
            SyntaxContextKind::Comment if value.starts_with("<!--") => {
                (RegionKind::BlockComment, value.ends_with("-->"))
            }
            SyntaxContextKind::Comment => (RegionKind::LineComment, false),
            SyntaxContextKind::Interpolation => unreachable!(),
        };
        captured.push((range, closed, kind));
        Ok(())
    })?;
    *visited += ancestry;
    if *visited > MAX_FOLD_NODES {
        return Err(SyntaxStatus::TooLarge);
    }
    Ok(())
}

fn new_parser(provider: SyntaxProvider) -> Result<Parser, SyntaxStatus> {
    let mut parser = Parser::new();
    parser
        .set_language(&(provider.grammar)())
        .map_err(|_| SyntaxStatus::Cancelled)?;
    Ok(parser)
}

fn parse_tree(
    parser: &mut Parser,
    text: &str,
    previous: Option<&Tree>,
    checks: &mut usize,
    should_continue: &mut impl FnMut() -> bool,
) -> Result<Tree, SyntaxStatus> {
    if !should_continue() {
        return Err(SyntaxStatus::Cancelled);
    }
    let mut progress = |_: &tree_sitter::ParseState| {
        *checks += 1;
        if *checks > MAX_PROGRESS_CHECKS || !should_continue() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    parser
        .parse_with_options(
            &mut |offset, _| &text.as_bytes()[offset..],
            previous,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or(SyntaxStatus::Cancelled)
}

fn visit_tree<'tree>(
    tree: &'tree Tree,
    visited: &mut usize,
    mut visitor: impl FnMut(Node<'tree>) -> Result<(), SyntaxStatus>,
) -> Result<(), SyntaxStatus> {
    let mut cursor = tree.walk();
    loop {
        *visited += 1;
        if *visited > MAX_FOLD_NODES {
            return Err(SyntaxStatus::TooLarge);
        }
        visitor(cursor.node())?;
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(());
            }
        }
    }
}

fn select_injections(
    tree: &Tree,
    provider: Option<SyntaxProvider>,
    text: &str,
    should_continue: &mut impl FnMut() -> bool,
) -> Result<Vec<(Language, Range)>, SyntaxStatus> {
    if !should_continue() {
        return Err(SyntaxStatus::Cancelled);
    }
    let Some(select) = provider.and_then(|provider| provider.injection) else {
        return Ok(Vec::new());
    };
    let starts: Vec<_> = std::iter::once(0)
        .chain(
            text.bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
        )
        .collect();
    let source_point = |offset: usize| {
        let row = starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        Point {
            row,
            column: offset - starts[row],
        }
    };
    let mut selected = Vec::new();
    let mut visited = 0;
    let mut until_check = 0;
    visit_tree(tree, &mut visited, |node| {
        if until_check == 0 {
            if !should_continue() {
                return Err(SyntaxStatus::Cancelled);
            }
            until_check = 256;
        }
        until_check -= 1;
        if let Some((language, range)) = select(node, text) {
            if selected.len() == MAX_INJECTIONS {
                return Err(SyntaxStatus::TooLarge);
            }
            if range.start_byte > range.end_byte
                || !text.is_char_boundary(range.start_byte)
                || !text.is_char_boundary(range.end_byte)
                || range.start_point != source_point(range.start_byte)
                || range.end_point != source_point(range.end_byte)
                || selected
                    .last()
                    .is_some_and(|(_, previous): &(Language, Range)| {
                        previous.end_byte > range.start_byte
                    })
            {
                return Err(SyntaxStatus::Cancelled);
            }
            selected.push((language, range));
        }
        Ok(())
    })?;
    Ok(selected)
}

fn collect_folds(
    tree: &Tree,
    provider: SyntaxProvider,
    lines: &[&str],
    ranges: &mut Vec<FoldRange>,
    visited: &mut usize,
) -> Result<(), SyntaxStatus> {
    visit_tree(tree, visited, |node| {
        if provider.fold_nodes.contains(&node.kind()) && !node.is_missing() {
            let start = if provider.parent_headers.contains(&node.kind()) {
                node.parent()
                    .map_or(node.start_position(), |parent| parent.start_position())
            } else {
                node.start_position()
            };
            let end = node.end_position();
            let trailing = lines
                .get(end.row)
                .and_then(|line| line.get(end.column..))
                .is_some_and(|rest| !rest.trim().is_empty());
            ranges.push(FoldRange {
                start_line: start.row,
                end_line: end
                    .row
                    .saturating_sub(usize::from(end.column == 0 || trailing)),
            });
        }
        Ok(())
    })
}

fn point(text: &str, offset: usize) -> Point {
    let before = &text[..offset];
    Point {
        row: before.bytes().filter(|byte| *byte == b'\n').count(),
        column: before
            .rfind('\n')
            .map_or(offset, |newline| offset - newline - 1),
    }
}

fn input_edit(old: &str, new: &str) -> InputEdit {
    let change = super::text_change(old, new).unwrap_or(super::TextChange {
        range: 0..0,
        new_end: 0,
    });
    let start = change.range.start;
    let old_end = change.range.end;
    let new_end = change.new_end;
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(old, start),
        old_end_position: point(old, old_end),
        new_end_position: point(new, new_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_embedded_bodies_keep_global_coordinates_incremental_trees_and_languages() {
        let original = "<header>文😀</header>\n<script type=module>\nfunction first() {\n  return { label: \"{\" };\n}\n</script>\n<style>\n.main {\n  content: \"}\";\n}\n</style>\n<script type=application/json>\n{\n  \"not\": \"code\"\n}\n</script>\n<script TYPE=\"text&sol;javascript\">\nfunction second() {\n  if (true) {\n    console.log(\"文😀\");\n  }\n}\n</script>\n";
        let mut syntax = SyntaxDocument::new(Language::Html).unwrap();
        let revised = original
            .replace(
                "<header>文😀</header>",
                "<header>😀文 changed</header>\n<p>prefix</p>",
            )
            .replace("return { label", "return { renamed")
            .replace('\n', "\r\n");
        let reordered = revised
            .replace("type=module", "type=application/json")
            .replace("TYPE=\"text&sol;javascript\"", "type=module");
        for (index, source) in [original.to_string(), revised, reordered]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                syntax.update(&source, || true),
                SyntaxStatus::Ready {
                    incremental: index != 0
                }
            );
            let mut fresh = SyntaxDocument::new(Language::Html).unwrap();
            fresh.update(&source, || true);
            assert_eq!(syntax.folds(), fresh.folds());
            assert_eq!(
                positions(syntax.tree.as_ref().unwrap()),
                positions(fresh.tree.as_ref().unwrap())
            );
            assert_eq!(syntax.embedded.len(), fresh.embedded.len());
            for (embedded, fresh) in syntax.embedded.iter().zip(&fresh.embedded) {
                assert_eq!(embedded.range, fresh.range);
                assert_eq!(
                    positions(embedded.tree.as_ref().unwrap()),
                    positions(fresh.tree.as_ref().unwrap())
                );
                assert!(!embedded.tree.as_ref().unwrap().root_node().has_error());
                assert_eq!(
                    embedded.range.start_point,
                    point(&source, embedded.range.start_byte)
                );
            }
            assert_eq!(
                syntax.language_at(source.find("content:").unwrap()),
                Language::Css
            );
            assert_eq!(
                syntax.language_at(source.find("not").unwrap()),
                Language::Html
            );
            assert_eq!(
                syntax.language_at(source.find("console.log").unwrap()),
                Language::JavaScript
            );
        }
        syntax.update(original, || true);
        assert_eq!(syntax.embedded.len(), 3);
        for expected in [
            FoldRange {
                start_line: 2,
                end_line: 4,
            },
            FoldRange {
                start_line: 7,
                end_line: 9,
            },
            FoldRange {
                start_line: 17,
                end_line: 21,
            },
        ] {
            assert!(syntax.folds().contains(&expected), "{:?}", syntax.folds());
        }
        assert_eq!(syntax.update(original, || false), SyntaxStatus::Cancelled);
        assert!(syntax.embedded.is_empty());
        assert_eq!(
            syntax.language_at(original.find("content:").unwrap()),
            Language::Html
        );
    }

    #[test]
    fn parsed_contexts_keep_interpolation_code_nested_literals_and_embedded_scopes() {
        let source = "const s = `hello ${call({x: \"}\"})} tail`;";
        let mut syntax = SyntaxDocument::new(Language::JavaScript).unwrap();
        syntax.update(source, || true);
        let structure = syntax.structure().unwrap();
        assert!(!structure.is_code(source.find("hello").unwrap()));
        assert!(structure.is_code(source.find("call").unwrap()));
        assert!(!structure.is_code(source.find("\"}\"").unwrap() + 1));
        assert!(!structure.is_code(source.find(" tail").unwrap()));
        assert!(!structure.is_code(source.find("tail").unwrap()));
        assert_eq!(structure.brackets.len(), 6);
        assert!(structure.brackets.iter().all(|(_, _, pair)| pair.is_some()));
        let html = format!(
            "<script>{source}</script><style>a {{ content: \"}}\"; }}</style><script>({{x:1}})</script>"
        );
        syntax = SyntaxDocument::new(Language::Html).unwrap();
        syntax.update(&html, || true);
        let structure = syntax.structure().unwrap();
        assert_eq!(
            structure.language_at(html.find("call").unwrap()),
            Language::JavaScript
        );
        assert_eq!(
            structure.language_at(html.find("content").unwrap()),
            Language::Css
        );
        assert_eq!(structure.language_at(0), Language::Html);
        assert!(!structure.is_code(html.find("\"}\"").unwrap() + 1));
        assert_eq!(structure.brackets.len(), 12);
        assert!(structure.brackets.iter().all(|(_, _, pair)| pair.is_some()));
        syntax.update(&html, || false);
        assert!(syntax.structure().is_none());
    }

    #[test]
    fn prepared_commands_use_each_cursor_language_and_reject_stale_contexts_atomically() {
        use super::super::{Document, EditError, Indentation, Selection};
        let source = "<script>const s = `text ${call()} tail`;</script><style>a { color: ; }</style><p>plain</p>";
        let mut syntax = SyntaxDocument::new(Language::Html).unwrap();
        syntax.update(source, || true);
        let context = syntax.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![
                Selection::caret(source.find("call()").unwrap() + "call(".len()),
                Selection::caret(source.find("; }").unwrap()),
                Selection::caret(source.find("plain").unwrap()),
            ])
            .unwrap();
        let before = document.clone();
        document.type_character_with_context('(', &context).unwrap();
        assert_eq!(
            document.text(),
            "<script>const s = `text ${call(())} tail`;</script><style>a { color: (); }</style><p>(plain</p>"
        );
        let changed = document.clone();
        assert_eq!(
            document.newline_with_context(Indentation::default(), None, &context),
            Err(EditError::StaleContext)
        );
        assert_eq!(document, changed);
        assert_eq!(
            document.delete_pairs_with_context(&context),
            Err(EditError::StaleContext)
        );
        assert_eq!(document, changed);
        assert!(document.undo());
        assert_eq!(document.text(), before.text());
        assert_eq!(document.selections(), before.selections());
        for source in ["`text $`", "`text $"] {
            let mut document = Document::new(source);
            document
                .set_selections(vec![Selection::caret(source.find('$').unwrap() + 1)])
                .unwrap();
            document.type_character('{', Language::JavaScript).unwrap();
            assert_eq!(document.text(), source.replace('$', "${}"));
        }
        let transition = "<script>const s = `text $`;</script>";
        syntax.update(transition, || true);
        let context = syntax.structure().unwrap();
        let mut document = Document::new(transition);
        document
            .set_selections(vec![Selection::caret(transition.find('$').unwrap() + 1)])
            .unwrap();
        document.type_character_with_context('{', &context).unwrap();
        assert_eq!(document.text(), transition.replace('$', "${}"));
        let escaped = "`text \\ $`".replace("\\ ", "\\");
        let mut document = Document::new(&escaped);
        document
            .set_selections(vec![Selection::caret(escaped.find('$').unwrap() + 1)])
            .unwrap();
        document.type_character('{', Language::JavaScript).unwrap();
        assert_eq!(document.text(), escaped.replace('$', "${"));
        let comment = "<script>// note</script>";
        syntax.update(comment, || true);
        let context = syntax.structure().unwrap();
        let mut comment_document = Document::new(comment);
        comment_document
            .set_selections(vec![Selection::caret(comment.find("</script>").unwrap())])
            .unwrap();
        comment_document
            .type_character_with_context('(', &context)
            .unwrap();
        assert_eq!(comment_document.text(), "<script>// note(</script>");
        for source in ["// note", "// note\nnext"] {
            let mut document = Document::new(source);
            document
                .set_selections(vec![Selection::caret("// note".len())])
                .unwrap();
            document.type_character('(', Language::JavaScript).unwrap();
            assert_eq!(document.text(), source.replace("// note", "// note("));
        }
        let empty = "<script></script><style></style>";
        syntax.update(empty, || true);
        let context = syntax.structure().unwrap();
        let mut empty_document = Document::new(empty);
        empty_document
            .set_selections(vec![
                Selection::caret(empty.find("</script>").unwrap()),
                Selection::caret(empty.find("</style>").unwrap()),
            ])
            .unwrap();
        empty_document
            .type_character_with_context('(', &context)
            .unwrap();
        assert_eq!(
            empty_document.text(),
            "<script>()</script><style>()</style>"
        );
        let blocks = "<script>\nfunction run() {}\n</script>\n<style>\na {}\n</style>";
        syntax.update(blocks, || true);
        let context = syntax.structure().unwrap();
        let mut document = Document::new(blocks);
        document
            .set_selections(vec![
                Selection::caret(blocks.find("{}").unwrap() + 1),
                Selection::caret(blocks.rfind("{}").unwrap() + 1),
            ])
            .unwrap();
        document
            .newline_with_context(Indentation::default(), None, &context)
            .unwrap();
        assert_eq!(
            document.text(),
            "<script>\nfunction run() {\n    \n}\n</script>\n<style>\na {\n    \n}\n</style>"
        );
        let nested = "`a ${`b ${call({key: '} '})} c`} d`";
        let fallback = Structure::new(nested, Language::JavaScript);
        assert!(fallback.is_code(nested.find("call").unwrap()));
        assert!(!fallback.is_code(nested.find(" c").unwrap()));
        assert!(!fallback.is_code(nested.find(" d").unwrap()));
        assert!(fallback.brackets.iter().all(|(_, _, pair)| pair.is_some()));
        let escaped = "`a \\${notCode} tail`";
        assert!(
            !Structure::new(escaped, Language::JavaScript)
                .is_code(escaped.find("notCode").unwrap())
        );
    }

    #[test]
    fn reindent_uses_embedded_bodies_without_leaking_unclosed_blocks() {
        use super::super::{Document, EditError, Indentation, Selection};
        let source = "<script>\r\nfunction first() {\r\nx();\r\n</script>\r\n<style>\r\na {\r\ncolor: red;\r\n}\r\n</style>\r\n<script>\r\nfunction second() {\r\ny();\r\n}\r\n</script>";
        let mut parser = SyntaxDocument::new(Language::Html).unwrap();
        parser.update(source, || true);
        let context = parser.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![Selection {
                anchor: source.len(),
                head: 0,
            }])
            .unwrap();
        document
            .reindent_with_context(Indentation::default(), &context)
            .unwrap();
        assert_eq!(
            document.text(),
            "<script>\r\nfunction first() {\r\n    x();\r\n</script>\r\n<style>\r\na {\r\n    color: red;\r\n}\r\n</style>\r\n<script>\r\nfunction second() {\r\n    y();\r\n}\r\n</script>"
        );
        let edited = document.clone();
        assert_eq!(
            document.reindent_with_context(Indentation::default(), &context),
            Err(EditError::StaleContext)
        );
        assert_eq!(document, edited);
        assert!(document.undo());
        assert_eq!(document.text(), source);
    }

    #[test]
    fn reindent_preserves_parser_classified_multiline_literals() {
        use super::super::{Document, Indentation, Selection};
        let source = "class C {\nvoid F() {\nvar text = @\"first\n  unchanged literal\nlast\";\nCall();\n}\n}";
        let mut parser = SyntaxDocument::new(Language::CSharp).unwrap();
        parser.update(source, || true);
        let context = parser.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![Selection {
                anchor: 0,
                head: source.len(),
            }])
            .unwrap();
        document
            .reindent_with_context(Indentation::default(), &context)
            .unwrap();
        assert_eq!(
            document.text(),
            "class C {\n    void F() {\n        var text = @\"first\n  unchanged literal\nlast\";\n        Call();\n    }\n}"
        );
    }

    #[test]
    fn reindent_does_not_change_literal_whitespace_before_interpolation_code() {
        use super::super::{Document, Indentation, Selection};
        let source = "class C {\nvoid F() {\nvar text = $@\"first\n  {Call()}\nlast\";\n}\n}";
        let mut parser = SyntaxDocument::new(Language::CSharp).unwrap();
        parser.update(source, || true);
        let context = parser.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![Selection {
                anchor: 0,
                head: source.len(),
            }])
            .unwrap();
        document
            .reindent_with_context(Indentation::default(), &context)
            .unwrap();
        assert_eq!(
            document.text(),
            "class C {\n    void F() {\n        var text = $@\"first\n  {Call()}\nlast\";\n    }\n}"
        );
    }

    #[test]
    fn reindent_preserves_python_fstring_whitespace_around_code_holes() {
        use super::super::{Document, Indentation, Selection};
        let source = "values = [\nf\"\"\"first\n  {call()}\nlast\"\"\"\n]";
        let mut parser = SyntaxDocument::new(Language::Python).unwrap();
        parser.update(source, || true);
        let context = parser.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![Selection {
                anchor: 0,
                head: source.len(),
            }])
            .unwrap();
        document
            .reindent_with_context(Indentation::default(), &context)
            .unwrap();
        assert_eq!(
            document.text(),
            "values = [\n    f\"\"\"first\n  {call()}\nlast\"\"\"\n]"
        );
    }

    #[test]
    fn block_comments_use_each_embedded_language_and_keep_markup_outside_caret_edits() {
        use super::super::{Document, EditError, Selection};
        let source = "<script>call();</script><style>a { color: red; }</style><p>文</p>";
        let mut parser = SyntaxDocument::new(Language::Html).unwrap();
        parser.update(source, || true);
        let context = parser.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![
                Selection::caret(source.find("call").unwrap()),
                Selection {
                    anchor: source.find("red").unwrap(),
                    head: source.find("red").unwrap() + 3,
                },
            ])
            .unwrap();
        document
            .toggle_block_comments_with_context(&context)
            .unwrap();
        assert_eq!(
            document.text(),
            "<script>/* call(); */</script><style>a { color: /* red */; }</style><p>文</p>"
        );
        let edited = document.clone();
        assert_eq!(
            document.toggle_block_comments_with_context(&context),
            Err(EditError::StaleContext)
        );
        assert_eq!(document, edited);
        parser.update(document.text(), || true);
        document
            .toggle_block_comments_with_context(&parser.structure().unwrap())
            .unwrap();
        assert_eq!(document.text(), source);
        assert!(document.undo());
        assert_eq!(document.text(), edited.text());
    }

    #[test]
    fn embedded_block_comment_selection_cannot_escape_its_language_body() {
        use super::super::{Document, Selection};
        let source = "<script>call();</script><style>a {}</style>";
        let mut parser = SyntaxDocument::new(Language::Html).unwrap();
        parser.update(source, || true);
        let context = parser.structure().unwrap();
        let mut document = Document::new(source);
        document
            .set_selections(vec![Selection {
                anchor: source.find("call").unwrap(),
                head: source.len(),
            }])
            .unwrap();
        let before = document.clone();
        assert!(
            !document
                .toggle_block_comments_with_context(&context)
                .unwrap()
        );
        assert_eq!(document, before);
    }

    #[test]
    fn line_comments_mix_embedded_syntax_in_one_transaction_and_reject_stale_contexts() {
        use super::super::{Document, EditError, Selection};
        let source = "<script>call();</script><style>a { color: red; }</style>";
        let mut parser = SyntaxDocument::new(Language::Html).unwrap();
        parser.update(source, || true);
        let mut document = Document::new(source);
        document
            .set_selections(vec![
                Selection::caret(source.find("call").unwrap()),
                Selection {
                    anchor: source.find("red").unwrap() + 3,
                    head: source.find("red").unwrap(),
                },
            ])
            .unwrap();
        let context = parser.structure().unwrap();
        document
            .toggle_line_comments_with_context(&context)
            .unwrap();
        let expected = "<script>// call();</script><style>a { color: /* red */; }</style>";
        assert_eq!(document.text(), expected);
        let css = document.selections()[1];
        assert!(css.anchor > css.head);
        assert_eq!(&document.text()[css.range()], "red");
        let changed = document.clone();
        assert_eq!(
            document.toggle_line_comments_with_context(&context),
            Err(EditError::StaleContext)
        );
        assert_eq!(document, changed);
        parser.update(document.text(), || true);
        document
            .toggle_line_comments_with_context(&parser.structure().unwrap())
            .unwrap();
        assert_eq!(document.text(), source);
        assert!(document.undo());
        assert_eq!(document.text(), expected);
        assert!(document.undo());
        assert_eq!(document.text(), source);
    }

    #[test]
    fn line_comments_deduplicate_same_body_rows_and_block_carets() {
        use super::super::{Document, Selection};
        let source = "<script>call();</script><style>a { color: red; }</style>";
        let mut parser = SyntaxDocument::new(Language::Html).unwrap();
        parser.update(source, || true);
        let mut document = Document::new(source);
        document
            .set_selections(vec![
                Selection::caret(source.find("call").unwrap()),
                Selection::caret(source.find("call").unwrap() + 3),
                Selection::caret(source.find("color").unwrap()),
                Selection::caret(source.find("red").unwrap()),
            ])
            .unwrap();
        document
            .toggle_line_comments_with_context(&parser.structure().unwrap())
            .unwrap();
        assert_eq!(
            document.text(),
            "<script>// call();</script><style>/* a { color: red; } */</style>"
        );
        assert!(document.undo());
        assert_eq!(document.text(), source);
        assert_eq!(document.selections().len(), 4);
    }

    #[test]
    fn line_comments_reject_escaping_selections_without_applying_other_cursors() {
        use super::super::{Document, Selection};
        let source = "<script>call();</script><style>a { color: red; }</style>";
        let mut parser = SyntaxDocument::new(Language::Html).unwrap();
        parser.update(source, || true);
        let mut document = Document::new(source);
        document
            .set_selections(vec![
                Selection::caret(source.find("call").unwrap()),
                Selection {
                    anchor: source.find("color").unwrap(),
                    head: source.len(),
                },
            ])
            .unwrap();
        let before = document.clone();
        assert!(
            !document
                .toggle_line_comments_with_context(&parser.structure().unwrap())
                .unwrap()
        );
        assert_eq!(document, before);
    }

    #[test]
    fn parsed_contexts_cover_builtin_literals_and_incomplete_interpolation() {
        for &(path, source, _) in super::super::syntax_contracts::LANGUAGE_CASES {
            let language = crate::highlight::language_from_path(path);
            let mut syntax = SyntaxDocument::new(language).unwrap();
            syntax.update(source, || true);
            let structure = syntax.structure().unwrap();
            if language != Language::Html {
                assert!(
                    !structure.is_code(source.find("文😀").unwrap() + "文".len()),
                    "{path}"
                );
            }
        }
        for (language, source, code) in [
            (Language::JavaScript, "`text ${call(", "call"),
            (Language::Python, "f'hello {call(\"x\")} end'", "call"),
            (
                Language::CSharp,
                "class C { string s = $\"text {Call(1)} tail\"; }",
                "Call",
            ),
        ] {
            let mut syntax = SyntaxDocument::new(language).unwrap();
            syntax.update(source, || true);
            let structure = syntax.structure().unwrap();
            assert!(
                structure.is_code(source.find(code).unwrap()),
                "{language:?}: {structure:?} {}",
                syntax.tree.as_ref().unwrap().root_node().to_sexp()
            );
            assert!(
                !structure.is_code(
                    source
                        .find("text")
                        .or_else(|| source.find("hello"))
                        .unwrap()
                        + 1
                ),
                "{language:?}"
            );
        }
    }

    #[test]
    fn html_mime_defaults_empty_bodies_and_independent_broken_scripts() {
        for (opening, expected) in [
            ("<script>", Some(Language::JavaScript)),
            (
                "<script type=\"TEXT/JAVASCRIPT1.5\">",
                Some(Language::JavaScript),
            ),
            (
                "<script TYPE=\"text&#47;javascript\">",
                Some(Language::JavaScript),
            ),
            (
                "<script type=module type=application/json>",
                Some(Language::JavaScript),
            ),
            ("<script type=application/json type=module>", None),
            ("<script type=\"text/javascript; charset=utf-8\">", None),
            ("<script type=importmap>", None),
            (
                "<script type=' text/javascript &#10;'>",
                Some(Language::JavaScript),
            ),
            ("<script type='&#160;text/javascript'>", None),
            ("<script type=' module '>", None),
            (
                "<script language=JavaScript1.5>",
                Some(Language::JavaScript),
            ),
            ("<script language=python>", None),
            ("<script language=' javascript '>", None),
            (
                "<script type='' language=python>",
                Some(Language::JavaScript),
            ),
            ("<style type='text/css'>", Some(Language::Css)),
            ("<style type=text/less>", None),
        ] {
            let closing = if opening.starts_with("<style") {
                "</style>"
            } else {
                "</script>"
            };
            let source = format!("{opening}{closing}");
            let mut syntax = SyntaxDocument::new(Language::Html).unwrap();
            assert_eq!(
                syntax.update(&source, || true),
                SyntaxStatus::Ready { incremental: false }
            );
            assert_eq!(
                syntax.embedded.first().map(|body| body.provider.language),
                expected,
                "{opening}"
            );
            assert_eq!(
                syntax.language_at(opening.len()),
                expected.unwrap_or(Language::Html)
            );
        }
        let source = "<script>\nfunction broken() {\n</script>\n<script>\nfunction valid() {\n  return 1;\n}\n</script>";
        let mut syntax = SyntaxDocument::new(Language::Html).unwrap();
        syntax.update(source, || true);
        assert_eq!(syntax.embedded.len(), 2);
        assert!(
            syntax.embedded[0]
                .tree
                .as_ref()
                .unwrap()
                .root_node()
                .has_error()
        );
        assert!(
            !syntax.embedded[1]
                .tree
                .as_ref()
                .unwrap()
                .root_node()
                .has_error()
        );
        assert!(syntax.folds().contains(&FoldRange {
            start_line: 4,
            end_line: 6
        }));
    }

    #[test]
    fn injection_limits_invalid_ranges_and_mid_parse_cancellation_discard_all_trees() {
        let small = "<script>let value = 1;</script>";
        let mut syntax = SyntaxDocument::new(Language::Html).unwrap();
        syntax.update(small, || true);
        let oversized = small.repeat(MAX_INJECTIONS + 1);
        assert_eq!(syntax.update(&oversized, || true), SyntaxStatus::TooLarge);
        assert!(syntax.tree.is_none());
        assert!(syntax.embedded.is_empty());
        let mut checks = 0;
        assert_eq!(
            syntax.update(small, || {
                checks += 1;
                checks < 5
            }),
            SyntaxStatus::Cancelled
        );
        assert!(syntax.tree.is_none());
        assert!(syntax.embedded.is_empty());
        assert_eq!(
            syntax.update(small, || true),
            SyntaxStatus::Ready { incremental: false }
        );
        let mut invalid = syntax_provider(Language::Html).unwrap();
        invalid.injection = Some(|_, _| {
            Some((
                Language::JavaScript,
                Range {
                    start_byte: 0,
                    end_byte: usize::MAX,
                    start_point: Point::new(0, 0),
                    end_point: Point::new(0, 0),
                },
            ))
        });
        let mut syntax = SyntaxDocument::with_provider(Language::Html, Some(invalid)).unwrap();
        assert_eq!(syntax.update(small, || true), SyntaxStatus::Cancelled);
        assert!(syntax.tree.is_none());
    }

    #[test]
    fn provider_fold_nodes_exist_in_the_released_grammars() {
        for provider in super::super::SYNTAX_PROVIDERS {
            let grammar = (provider.grammar)();
            for kind in provider.fold_nodes.iter().chain(provider.parent_headers) {
                assert_ne!(
                    grammar.id_for_node_kind(kind, true),
                    0,
                    "{:?}: {kind}",
                    provider.language
                );
            }
        }
    }

    fn positions(tree: &Tree) -> Vec<String> {
        let mut cursor = tree.walk();
        let mut positions = Vec::new();
        loop {
            let node = cursor.node();
            positions.push(format!(
                "{}:{}..{}:{:?}..{:?}:{}:{}",
                node.kind(),
                node.start_byte(),
                node.end_byte(),
                node.start_position(),
                node.end_position(),
                node.is_missing(),
                node.is_error()
            ));
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return positions;
                }
            }
        }
    }

    #[test]
    fn built_in_grammars_incremental_folds_and_recovery_share_one_contract() {
        let oversized = "x".repeat(MAX_STRUCTURE_BYTES + 1);
        for &(path, source, expected) in super::super::syntax_contracts::LANGUAGE_CASES {
            let language = crate::highlight::language_from_path(path);
            let mut syntax = SyntaxDocument::new(language).unwrap();
            for (index, source) in [
                source.to_string(),
                source.replace("文😀", "😀文 changed").replace('\n', "\r\n"),
            ]
            .into_iter()
            .enumerate()
            {
                assert_eq!(
                    syntax.update(&source, || true),
                    SyntaxStatus::Ready {
                        incremental: index != 0
                    },
                    "{path}"
                );
                let tree = syntax.tree.as_ref().unwrap();
                assert!(
                    !tree.root_node().has_error(),
                    "{path}: {}",
                    tree.root_node().to_sexp()
                );
                assert!(
                    syntax.folds().contains(&expected),
                    "{path}: {:?}",
                    syntax.folds()
                );
                let mut fresh = SyntaxDocument::new(language).unwrap();
                fresh.update(&source, || true);
                assert_eq!(
                    positions(tree),
                    positions(fresh.tree.as_ref().unwrap()),
                    "{path}"
                );
                assert_eq!(syntax.folds(), fresh.folds(), "{path}");
            }
            assert_eq!(syntax.update(source, || false), SyntaxStatus::Cancelled);
            assert!(syntax.tree.is_none());
            assert!(syntax.folds().is_empty());
            assert_eq!(syntax.update(&oversized, || true), SyntaxStatus::TooLarge);
            assert!(syntax.tree.is_none());
            assert_eq!(
                syntax.update(source, || true),
                SyntaxStatus::Ready { incremental: false }
            );
        }
    }

    #[test]
    fn custom_provider_uses_shared_parse_and_fold_policy() {
        let provider = SyntaxProvider {
            language: Language::Rust,
            context: None,
            injection: None,
            grammar: || tree_sitter_rust::LANGUAGE.into(),
            fold_nodes: &["arguments"],
            parent_headers: &[],
        };
        assert!(SyntaxDocument::with_provider(Language::Python, Some(provider)).is_none());
        let mut syntax = SyntaxDocument::with_provider(Language::Rust, Some(provider)).unwrap();
        syntax.update(
            "fn main() {\n    call(\n        1,\n        2\n    );\n}\n",
            || true,
        );
        assert_eq!(
            syntax.folds(),
            vec![FoldRange {
                start_line: 1,
                end_line: 3
            }]
        );
    }

    #[test]
    fn incremental_unicode_crlf_edits_match_fresh_trees_and_fold_ranges() {
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        let sources = [
            "// 😀\r\nfn main() {\r\n    let x = r###\" } /* \"###;\r\n    /* outer\r\n       /* nested */\r\n    */\r\n}\r\n",
            "// 😀\r\nfn main() {\r\n    if true {\r\n        let x = r###\" } /* \"###;\r\n    }\r\n}\r\n",
            "fn main() {\n    let text = \"文😀\";\n}\n",
            "",
        ];
        for (index, source) in sources.into_iter().enumerate() {
            assert_eq!(
                syntax.update(source, || true),
                SyntaxStatus::Ready {
                    incremental: index != 0
                }
            );
            let mut fresh = SyntaxDocument::new(Language::Rust).unwrap();
            fresh.update(source, || true);
            assert_eq!(
                syntax.tree.as_ref().unwrap().root_node().to_sexp(),
                fresh.tree.as_ref().unwrap().root_node().to_sexp()
            );
            assert_eq!(
                syntax.tree.as_ref().unwrap().root_node().end_byte(),
                source.len()
            );
            assert_eq!(
                positions(syntax.tree.as_ref().unwrap()),
                positions(fresh.tree.as_ref().unwrap())
            );
            assert_eq!(syntax.folds(), fresh.folds());
            assert!(!syntax.tree.as_ref().unwrap().root_node().has_error());
        }
    }

    #[test]
    fn syntax_folds_ignore_braces_in_literals_and_include_nested_comments() {
        let source = "fn main() {\n    let text = r###\" { } \"###;\n    /* outer\n       /* nested */\n    */\n}\n";
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        syntax.update(source, || true);
        assert_eq!(
            syntax.folds(),
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 5
                },
                FoldRange {
                    start_line: 2,
                    end_line: 4
                }
            ]
        );
        assert!(SyntaxDocument::new(Language::Python).is_some());
    }

    #[test]
    fn shared_closing_and_opening_rows_keep_both_fold_headers_visible() {
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        syntax.update(
            "fn main() {\n    if true {\n        a();\n    } else {\n        b();\n    }\n}\n",
            || true,
        );
        assert_eq!(
            syntax.folds(),
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 6
                },
                FoldRange {
                    start_line: 1,
                    end_line: 2
                },
                FoldRange {
                    start_line: 3,
                    end_line: 5
                },
            ]
        );
    }

    #[test]
    fn cancelled_and_oversize_parses_drop_stale_trees_and_can_recover() {
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        syntax.update("fn main() {\n}\n", || true);
        assert!(!syntax.folds().is_empty());
        assert_eq!(
            syntax.update("fn other() {\n}\n", || false),
            SyntaxStatus::Cancelled
        );
        assert!(syntax.folds().is_empty());
        assert_eq!(
            syntax.update(&"x".repeat(MAX_STRUCTURE_BYTES + 1), || true),
            SyntaxStatus::TooLarge
        );
        assert_eq!(
            syntax.update("fn main() {\n}\n", || true),
            SyntaxStatus::Ready { incremental: false }
        );
        // Cancel within parser progress, not just before entering the runtime.
        let mut calls = 0;
        assert_eq!(
            syntax.update(&"fn main() {}\n".repeat(10_000), || {
                calls += 1;
                calls < 2
            }),
            SyntaxStatus::Cancelled
        );
        assert!(calls >= 2);
        assert!(syntax.folds().is_empty());
        assert_eq!(
            syntax.update("fn recovered() {}", || true),
            SyntaxStatus::Ready { incremental: false }
        );
    }
}
