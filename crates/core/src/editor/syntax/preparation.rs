//! Source-bound parsing progress; synchronous and yielding drivers share this engine.
use super::{
    EmbeddedSyntax, InjectionSelection, MAX_PROGRESS_CHECKS, MAX_STRUCTURE_BYTES, SyntaxAnalysis,
    SyntaxDocument, SyntaxStatus, indexed_point, input_edit, input_edit_change, new_parser,
    preparation_exceeds_limits, syntax_provider,
};
use crate::highlight::Language;
use std::{collections::HashMap, ops::ControlFlow, sync::Arc};
use tree_sitter::{InputEdit, ParseOptions, Parser, Range, Tree};

pub(super) struct SyntaxWork {
    source: Arc<String>,
    edit: InputEdit,
    incremental: bool,
    previous: Option<Tree>,
    tree: Option<Tree>,
    embedded: Option<EmbeddedWork>,
    checks: usize,
    selection: InjectionSelection,
}
struct EmbeddedWork {
    selected: std::vec::IntoIter<(Language, Range)>,
    old: std::vec::IntoIter<EmbeddedSyntax>,
    unchanged: HashMap<(usize, usize), Vec<EmbeddedSyntax>>,
    next: Vec<EmbeddedSyntax>,
    current: Option<EmbeddedBody>,
}
struct EmbeddedBody {
    body: EmbeddedSyntax,
    previous: Option<Tree>,
    parser_index: usize,
    #[cfg(test)]
    checks_at_start: usize,
}

impl SyntaxDocument {
    pub(super) fn begin_update(
        &mut self,
        text: &str,
        mut should_continue: impl FnMut() -> bool,
        source: impl FnOnce() -> Arc<String>,
        resolved_change: Option<(&Arc<String>, &crate::editor::TextChange)>,
    ) -> Option<SyntaxStatus> {
        if text.len() > MAX_STRUCTURE_BYTES {
            self.clear();
            return Some(SyntaxStatus::TooLarge);
        }
        if !should_continue() {
            self.clear();
            return Some(SyntaxStatus::Cancelled);
        }
        if let Some(pending) = &self.pending {
            if std::ptr::eq(pending.source.as_str(), text) || pending.source.as_str() == text {
                return None;
            }
            self.clear();
        }
        // Only the resolver can supply a change, and it owns the exact base used
        // to construct this source. An equal-content but distinct base is insufficient.
        let resolved_change = resolved_change
            .filter(|(base, _)| self.ready && Arc::ptr_eq(base, &self.text))
            .map(|(_, change)| change);
        let unchanged = resolved_change.map_or_else(
            || std::ptr::eq(self.text.as_str(), text) || self.text.as_str() == text,
            |change| change.range.is_empty() && change.new_end == change.range.start,
        );
        if self.ready && unchanged {
            return Some(SyntaxStatus::Ready { incremental: true });
        }
        self.prepared = None;
        if self.parser.is_none() {
            self.text = source();
            self.ready = true;
            return Some(SyntaxStatus::Ready { incremental: false });
        }
        let edit_result = if let Some(change) = resolved_change {
            input_edit_change(
                &self.text,
                text,
                Some(&mut self.source_lines),
                change.clone(),
            )
        } else {
            input_edit(&self.text, text, Some(&mut self.source_lines))
        };
        let edit = match edit_result {
            Ok(edit) => edit,
            Err(status) => {
                self.clear();
                return Some(status);
            }
        };
        let mut previous = self.tree.clone();
        if let Some(tree) = previous.as_mut() {
            tree.edit(&edit);
        }
        self.ready = false;
        self.pending = Some(SyntaxWork {
            source: source(),
            edit,
            incremental: previous.is_some(),
            previous,
            tree: None,
            embedded: None,
            checks: 0,
            selection: InjectionSelection::default(),
        });
        None
    }

    /// None means parsing yielded. Partial analysis is never published;
    /// cancellation or a new source releases all unfinished progress.
    pub fn prepare_cooperative(
        &mut self,
        source: Arc<String>,
        tab_width: usize,
        mut should_continue: impl FnMut() -> bool,
        mut should_yield: impl FnMut() -> bool,
    ) -> Option<(SyntaxStatus, Option<Arc<SyntaxAnalysis>>)> {
        self.prepare_resolved_cooperative(
            source,
            tab_width,
            &mut should_continue,
            &mut should_yield,
            None,
        )
    }
    pub(super) fn prepare_resolved_cooperative(
        &mut self,
        source: Arc<String>,
        tab_width: usize,
        should_continue: &mut impl FnMut() -> bool,
        should_yield: &mut impl FnMut() -> bool,
        change: Option<(&Arc<String>, &crate::editor::TextChange)>,
    ) -> Option<(SyntaxStatus, Option<Arc<SyntaxAnalysis>>)> {
        let admitted = self
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(&pending.source, &source))
            || (self.ready && Arc::ptr_eq(&self.text, &source));
        if !admitted && preparation_exceeds_limits(&source) {
            self.clear();
            return Some((SyntaxStatus::TooLarge, None));
        }
        let status = self
            .begin_update(&source, &mut *should_continue, || source.clone(), change)
            .or_else(|| self.advance_update(should_continue, should_yield))?;
        Some(self.finish_preparation(status, tab_width, should_continue))
    }
    pub(super) fn advance_update(
        &mut self,
        should_continue: &mut impl FnMut() -> bool,
        should_yield: &mut impl FnMut() -> bool,
    ) -> Option<SyntaxStatus> {
        let mut work = self.pending.take().expect("started syntax preparation");
        match work.advance(self, should_continue, should_yield) {
            Ok(false) => {
                self.pending = Some(work);
                None
            }
            Ok(true) => {
                self.ready = true;
                self.tree = work.tree;
                self.embedded = work.embedded.map_or_else(Vec::new, |work| work.next);
                let mut paint = self.paint.borrow_mut();
                paint.source_change = Arc::ptr_eq(&paint.source, &self.text).then_some(work.edit);
                drop(paint);
                self.text = work.source;
                Some(SyntaxStatus::Ready {
                    incremental: work.incremental,
                })
            }
            Err(status) => {
                self.clear();
                Some(status)
            }
        }
    }
    fn begin_embedded(
        &mut self,
        selected: Vec<(Language, Range)>,
        edit: &InputEdit,
        should_continue: &mut impl FnMut() -> bool,
    ) -> Result<EmbeddedWork, SyntaxStatus> {
        let mut unchanged: HashMap<(usize, usize), Vec<EmbeddedSyntax>> = HashMap::new();
        let mut changed = Vec::new();
        for mut body in std::mem::take(&mut self.embedded) {
            if !should_continue() {
                return Err(SyntaxStatus::Cancelled);
            }
            let bytes = if body.range.end_byte <= edit.start_byte {
                Some(body.range.start_byte..body.range.end_byte)
            } else if body.range.start_byte >= edit.old_end_byte {
                Some(
                    edit.new_end_byte + (body.range.start_byte - edit.old_end_byte)
                        ..edit.new_end_byte + (body.range.end_byte - edit.old_end_byte),
                )
            } else {
                None
            };
            if let Some(bytes) = bytes {
                let range = Range {
                    start_byte: bytes.start,
                    end_byte: bytes.end,
                    start_point: indexed_point(&self.source_lines, bytes.start),
                    end_point: indexed_point(&self.source_lines, bytes.end),
                };
                if body.range != range
                    && let Some(tree) = body.tree.as_mut()
                {
                    tree.edit(edit);
                }
                body.range = range;
                unchanged
                    .entry((bytes.start, bytes.end))
                    .or_default()
                    .push(body);
            } else {
                changed.push(body);
            }
        }

        #[cfg(test)]
        {
            self.embedded_parses = 0;
        }
        Ok(EmbeddedWork {
            next: Vec::with_capacity(selected.len()),
            selected: selected.into_iter(),
            old: changed.into_iter(),
            unchanged,
            current: None,
        })
    }
}
impl SyntaxWork {
    fn advance(
        &mut self,
        document: &mut SyntaxDocument,
        should_continue: &mut impl FnMut() -> bool,
        should_yield: &mut impl FnMut() -> bool,
    ) -> Result<bool, SyntaxStatus> {
        loop {
            if !should_continue() {
                return Err(SyntaxStatus::Cancelled);
            }
            if should_yield() {
                return Ok(false);
            }
            if self.tree.is_none() {
                let Some(tree) = parse_step(
                    document.parser.as_mut().expect("grammar parser"),
                    &self.source,
                    self.previous.as_ref(),
                    &mut self.checks,
                    should_continue,
                    should_yield,
                )?
                else {
                    return Ok(false);
                };
                self.tree = Some(tree);
                continue;
            }
            if self.embedded.is_none() {
                if !self.selection.advance(
                    self.tree.as_ref().unwrap(),
                    document.provider,
                    &self.source,
                    &document.source_lines,
                    should_continue,
                    should_yield,
                )? {
                    return Ok(false);
                }
                let selected = std::mem::take(&mut self.selection.selected);
                self.embedded =
                    Some(document.begin_embedded(selected, &self.edit, should_continue)?);
                continue;
            }
            let embedded = self.embedded.as_mut().unwrap();
            if embedded.current.is_none() {
                let Some((language, range)) = embedded.selected.next() else {
                    return Ok(true);
                };
                if let Some(bodies) = embedded
                    .unchanged
                    .get_mut(&(range.start_byte, range.end_byte))
                    && let Some(index) = bodies
                        .iter()
                        .position(|body| body.provider.language == language && body.range == range)
                {
                    embedded.next.push(bodies.swap_remove(index));
                    continue;
                }
                let provider = syntax_provider(language).ok_or(SyntaxStatus::Cancelled)?;
                let mut body = embedded
                    .old
                    .next()
                    .filter(|body| body.provider.language == language)
                    .unwrap_or_else(|| EmbeddedSyntax {
                        provider,
                        tree: None,
                        range,
                        folds: std::cell::RefCell::default(),
                        contexts: std::cell::RefCell::default(),
                        highlights: std::cell::RefCell::default(),
                    });
                let mut previous = body.tree.clone();
                if let Some(tree) = previous.as_mut() {
                    tree.edit(&self.edit);
                }
                body.range = range;
                let parser_index = if let Some(index) = document
                    .embedded_parsers
                    .iter()
                    .position(|(retained, _)| *retained == language)
                {
                    index
                } else {
                    document
                        .embedded_parsers
                        .push((language, new_parser(provider)?));
                    document.embedded_parsers.len() - 1
                };
                document.embedded_parsers[parser_index]
                    .1
                    .set_included_ranges(&[range])
                    .map_err(|_| SyntaxStatus::Cancelled)?;
                embedded.current = Some(EmbeddedBody {
                    body,
                    previous,
                    parser_index,
                    #[cfg(test)]
                    checks_at_start: self.checks,
                });
            }
            let current = embedded.current.as_mut().unwrap();
            let Some(tree) = parse_step(
                &mut document.embedded_parsers[current.parser_index].1,
                &self.source,
                current.previous.as_ref(),
                &mut self.checks,
                should_continue,
                should_yield,
            )?
            else {
                return Ok(false);
            };
            let mut current = embedded.current.take().unwrap();
            current.body.tree = Some(tree);
            embedded.next.push(current.body);
            #[cfg(test)]
            {
                document.embedded_parses += 1;
            }
        }
    }
}
fn parse_step(
    parser: &mut Parser,
    text: &str,
    previous: Option<&Tree>,
    checks: &mut usize,
    should_continue: &mut impl FnMut() -> bool,
    should_yield: &mut impl FnMut() -> bool,
) -> Result<Option<Tree>, SyntaxStatus> {
    if !should_continue() {
        return Err(SyntaxStatus::Cancelled);
    }
    if should_yield() {
        return Ok(None);
    }
    let mut yielded = false;
    let mut progress = |_: &tree_sitter::ParseState| {
        *checks += 1;
        if *checks > MAX_PROGRESS_CHECKS || !should_continue() {
            return ControlFlow::Break(());
        }
        if should_yield() {
            yielded = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser.parse_with_options(
        &mut |offset, _| &text.as_bytes()[offset..],
        previous,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    );
    if tree.is_some() || yielded {
        Ok(tree)
    } else {
        Err(SyntaxStatus::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_selection_resumes_at_exact_nodes_and_keeps_global_limits() {
        for (language, source) in [
            (Language::Markdown, (0..1_000).map(|index| format!("Paragraph {index}: **文😀** and `code`.\r\n\r\n")).collect::<String>()),
            (Language::Html, "<main><script>const x = 1;</script><section><style>a { color: red; }</style></section></main>".to_owned()),
        ] {
            let mut document = SyntaxDocument::new(language).unwrap();
            let (status, _) = document.prepare_shared(Arc::new(source.clone()), 4, || true);
            assert!(matches!(status, SyntaxStatus::Ready { .. }));
            let tree = document.tree.as_ref().unwrap();
            let select = document.provider.unwrap().injection.unwrap();
            let mut expected = Vec::new();
            let mut visits = 0;
            super::super::visit_tree(tree, &mut visits, |node| {
                if let Some(body) = select(node, &source) {
                    expected.push(body);
                }
                Ok(())
            }).unwrap();
            let mut selection = InjectionSelection::default();
            let mut batches = 0;
            loop {
                let before = selection.visited;
                let mut checks = 0;
                let complete = selection.advance(tree, document.provider, &source, &document.source_lines, &mut || true, &mut || {
                    checks += 1;
                    checks > 1
                }).unwrap();
                assert!(selection.visited > before);
                assert!(selection.visited - before <= 256);
                batches += 1;
                if complete { break; }
                let retained = selection.visited;
                assert_eq!(selection.advance(tree, document.provider, &source, &document.source_lines, &mut || false, &mut || false), Err(SyntaxStatus::Cancelled));
                assert_eq!(selection.visited, retained);
                assert!(batches < 1_000);
            }
            assert_eq!(selection.visited, visits, "each node is visited exactly once");
            assert_eq!(selection.selected, expected);
            if language == Language::Markdown { assert!(batches > 1); }
            if visits > 256 {
                let mut limited = InjectionSelection {
                    visited: super::super::MAX_FOLD_NODES - 256,
                    ..InjectionSelection::default()
                };
                let mut checks = 0;
                assert!(!limited.advance(tree, document.provider, &source, &document.source_lines, &mut || true, &mut || { checks += 1; checks > 1 }).unwrap());
                assert_eq!(limited.visited, super::super::MAX_FOLD_NODES);
                assert_eq!(limited.advance(tree, document.provider, &source, &document.source_lines, &mut || true, &mut || false), Err(SyntaxStatus::TooLarge));
            }
        }
    }

    #[test]
    fn cooperative_parser_resumes_without_reparsing_completed_bodies() {
        for ending in ["\n", "\r\n"] {
            let source = Arc::new(
                (0..1_000)
                    .map(|index| format!("Paragraph {index}: **文😀** and `code`.{ending}{ending}"))
                    .collect::<String>(),
            );
            let mut document = SyntaxDocument::new(Language::Markdown).unwrap();
            let mut previous_count = 0;
            let mut batches = 0;
            let result = loop {
                let mut checks = 0;
                let result = document.prepare_cooperative(
                    source.clone(),
                    4,
                    || true,
                    || {
                        checks += 1;
                        checks >= 8
                    },
                );
                batches += 1;
                assert!(batches < 10_000, "bounded fixture must make progress");
                assert!(document.embedded_parses >= previous_count);
                previous_count = document.embedded_parses;
                if let Some(result) = result {
                    break result;
                }
                assert!(!document.ready, "partial syntax cannot be queried");
                assert!(document.structure().is_none());
                assert!(document.pending.is_some());
            };
            assert!(batches > 1);
            assert_eq!(document.embedded_parses, 1_000);
            assert_eq!(result.0, SyntaxStatus::Ready { incremental: false });
            let result = result.1.unwrap();
            assert!(Arc::ptr_eq(result.source_snapshot(), &source));
            let mut fresh = SyntaxDocument::new(Language::Markdown).unwrap();
            let (_, expected) = fresh.prepare_shared(source, 4, || true);
            assert_eq!(
                serde_json::to_value(result.transfer_data()).unwrap(),
                serde_json::to_value(expected.unwrap().transfer_data()).unwrap()
            );
        }
    }

    #[test]
    fn pending_parser_cancellation_and_supersession_release_old_source() {
        let source = Arc::new("fn main() { call(); }\n".repeat(1_000));
        let weak = Arc::downgrade(&source);
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        assert!(
            document
                .prepare_cooperative(source.clone(), 4, || true, || true)
                .is_none()
        );
        assert_eq!(
            document
                .prepare_cooperative(source.clone(), 4, || false, || false)
                .unwrap()
                .0,
            SyntaxStatus::Cancelled
        );
        assert!(document.pending.is_none());
        assert!(document.tree.is_none());
        assert!(document.embedded_parsers.is_empty());
        assert!(
            document
                .prepare_cooperative(source.clone(), 4, || true, || true)
                .is_none()
        );
        drop(source);
        let next = Arc::new("fn replacement() {}".to_owned());
        let (status, analysis) = document
            .prepare_cooperative(next.clone(), 4, || true, || false)
            .unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert_eq!(analysis.unwrap().source(), next.as_str());
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn yields_do_not_reset_the_parser_progress_limit() {
        let source = Arc::new("fn main() { call(); }\n".repeat(1_000));
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        assert!(
            document
                .prepare_cooperative(source.clone(), 4, || true, || true)
                .is_none()
        );
        document.pending.as_mut().unwrap().checks = MAX_PROGRESS_CHECKS;
        assert_eq!(
            document
                .prepare_cooperative(source, 4, || true, || false)
                .unwrap()
                .0,
            SyntaxStatus::Cancelled
        );
        assert!(document.pending.is_none());
        assert!(document.tree.is_none());
    }

    #[test]
    fn interrupted_embedded_parser_resumes_its_current_range() {
        let source = Arc::new(format!(
            "```rust\n{}```\n",
            "fn call() { if true { println!(\"文😀\"); } }\n".repeat(1_000)
        ));
        let mut document = SyntaxDocument::new(Language::Markdown).unwrap();
        let mut interrupted_body = false;
        let result = loop {
            let mut checks = 0;
            let result = document.prepare_cooperative(
                source.clone(),
                4,
                || true,
                || {
                    checks += 1;
                    checks >= 6
                },
            );
            if let Some(result) = result {
                break result;
            }
            interrupted_body |= document.pending.as_ref().is_some_and(|work| {
                work.embedded.as_ref().is_some_and(|embedded| {
                    embedded
                        .current
                        .as_ref()
                        .is_some_and(|body| work.checks > body.checks_at_start)
                })
            });
        };
        assert!(interrupted_body);
        assert_eq!(document.embedded_parses, 1);
        let mut fresh = SyntaxDocument::new(Language::Markdown).unwrap();
        let (_, expected) = fresh.prepare_shared(source, 4, || true);
        assert_eq!(
            serde_json::to_value(result.1.unwrap().transfer_data()).unwrap(),
            serde_json::to_value(expected.unwrap().transfer_data()).unwrap()
        );
    }

    #[test]
    fn cancelling_an_interrupted_embedded_parser_releases_source_before_recovery() {
        let source = Arc::new(format!(
            "```rust\n{}```\n",
            "fn main() { println!(\"文😀\"); }\n".repeat(1_000)
        ));
        let weak = Arc::downgrade(&source);
        let mut document = SyntaxDocument::new(Language::Markdown).unwrap();
        for attempt in 0..10_000 {
            let mut checks = 0;
            assert!(
                document
                    .prepare_cooperative(
                        source.clone(),
                        4,
                        || true,
                        || {
                            checks += 1;
                            checks >= 6
                        }
                    )
                    .is_none()
            );
            let interrupted = document.pending.as_ref().is_some_and(|work| {
                work.embedded.as_ref().is_some_and(|embedded| {
                    embedded
                        .current
                        .as_ref()
                        .is_some_and(|body| work.checks > body.checks_at_start)
                })
            });
            if interrupted {
                break;
            }
            assert!(attempt < 9_999);
        }
        assert_eq!(
            document
                .prepare_cooperative(source.clone(), 4, || false, || false)
                .unwrap()
                .0,
            SyntaxStatus::Cancelled
        );
        drop(source);
        assert!(weak.upgrade().is_none());
        assert!(document.pending.is_none());
        assert!(document.embedded_parsers.is_empty());
        let next = Arc::new("```rust\nfn replacement() {}\n```\n".to_owned());
        let (status, analysis) = document
            .prepare_cooperative(next.clone(), 4, || true, || false)
            .unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert_eq!(analysis.unwrap().source(), next.as_str());
    }
}
