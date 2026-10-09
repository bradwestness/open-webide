//! Source-bound parsing progress; synchronous and yielding drivers share this engine.
use super::{
    EmbeddedSyntax, FallbackContexts, InjectionSelection, MAX_PROGRESS_CHECKS, MAX_STRUCTURE_BYTES,
    SyntaxAdmission, SyntaxAdmissionStatus, SyntaxAnalysis, SyntaxDocument, SyntaxStatus,
    indexed_point, input_edit, input_edit_change, new_parser, syntax_provider,
};
use crate::editor::structure::scanning::LexicalScan;
use crate::highlight::Language;

const FALLBACK_BATCH_BYTES: usize = 8 * 1024;
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
    fallback: LexicalScan,
}
struct EmbeddedWork {
    selected: std::vec::IntoIter<(Language, Range)>,
    old: std::vec::IntoIter<EmbeddedSyntax>,
    retained: Option<std::vec::IntoIter<EmbeddedSyntax>>,
    changed: Vec<EmbeddedSyntax>,
    unchanged: HashMap<(usize, usize), Vec<EmbeddedSyntax>>,
    next: Vec<EmbeddedSyntax>,
    current: Option<EmbeddedBody>,
}
struct EmbeddedBody {
    body: EmbeddedSyntax,
    previous: Option<Tree>,
    parser_index: usize,
    fallback: LexicalScan,
    #[cfg(test)]
    checks_at_start: usize,
}

pub(super) struct SourceComparison {
    base: Arc<String>,
    source: Arc<String>,
    job: crate::editor::TextChangePreparation,
}

impl SyntaxDocument {
    pub(super) fn begin_update(
        &mut self,
        text: &str,
        mut should_continue: impl FnMut() -> bool,
        source: impl FnOnce() -> Arc<String>,
        resolved_change: Option<(&Arc<String>, &crate::editor::TextChange)>,
    ) -> Option<SyntaxStatus> {
        self.admission = None;
        self.source_comparison = None;
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
        self.lexical_pending = None;
        self.outer_fallback = None;
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
            fallback: LexicalScan::new(self.language),
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
            || self
                .source_comparison
                .as_ref()
                .is_some_and(|pending| Arc::ptr_eq(&pending.source, &source))
            || (self.ready && Arc::ptr_eq(&self.text, &source));
        if !admitted {
            match self.admit_source_cooperative(&source, should_continue, should_yield)? {
                Ok(()) => {}
                Err(status) => return Some((status, None)),
            }
        }
        let change = change.filter(|(base, _)| self.ready && Arc::ptr_eq(base, &self.text));
        let base = self.text.clone();
        let compared = if change.is_none() && self.ready && !Arc::ptr_eq(&base, &source) {
            match self.compare_source_cooperative(&source, should_continue, should_yield)? {
                Ok(change) => Some(change),
                Err(status) => return Some((status, None)),
            }
        } else {
            self.source_comparison = None;
            None
        };
        let change = compared.as_ref().map(|change| (&base, change)).or(change);
        let status = self
            .begin_update(&source, &mut *should_continue, || source.clone(), change)
            .or_else(|| self.advance_update(should_continue, should_yield))?;
        let status = self.advance_lexical(status, should_continue, should_yield, change)?;
        Some(self.finish_preparation(status, tab_width, should_continue))
    }

    fn admit_source_cooperative(
        &mut self,
        source: &Arc<String>,
        should_continue: &mut impl FnMut() -> bool,
        should_yield: &mut impl FnMut() -> bool,
    ) -> Option<Result<(), SyntaxStatus>> {
        if !self
            .admission
            .as_ref()
            .is_some_and(|job| Arc::ptr_eq(job.source_snapshot(), source))
        {
            self.prepared = None;
            self.admission = Some(SyntaxAdmission::new(source.clone()));
        }
        loop {
            let job = self.admission.as_mut().expect("retained admission source");
            match job.status() {
                SyntaxAdmissionStatus::Admitted => return Some(Ok(())),
                SyntaxAdmissionStatus::TooLarge => {
                    self.clear();
                    return Some(Err(SyntaxStatus::TooLarge));
                }
                SyntaxAdmissionStatus::Pending => {}
            }
            if !should_continue() {
                self.clear();
                return Some(Err(SyntaxStatus::Cancelled));
            }
            if should_yield() {
                return None;
            }
            job.advance(crate::highlight::LEXICAL_BATCH_BYTES);
        }
    }

    fn compare_source_cooperative(
        &mut self,
        source: &Arc<String>,
        should_continue: &mut impl FnMut() -> bool,
        should_yield: &mut impl FnMut() -> bool,
    ) -> Option<Result<crate::editor::TextChange, SyntaxStatus>> {
        if !self.source_comparison.as_ref().is_some_and(|pending| {
            Arc::ptr_eq(&pending.source, source) && Arc::ptr_eq(&pending.base, &self.text)
        }) {
            self.prepared = None;
            self.source_comparison = Some(SourceComparison {
                base: self.text.clone(),
                source: source.clone(),
                job: crate::editor::TextChangePreparation::default(),
            });
        }
        loop {
            if !should_continue() {
                self.clear();
                return Some(Err(SyntaxStatus::Cancelled));
            }
            let pending = self
                .source_comparison
                .as_mut()
                .expect("retained source comparison");
            if pending.job.is_complete() {
                let result = pending
                    .job
                    .change()
                    .cloned()
                    .unwrap_or(crate::editor::TextChange {
                        range: 0..0,
                        new_end: 0,
                    });
                self.source_comparison = None;
                return Some(Ok(result));
            }
            if should_yield() {
                return None;
            }
            pending.job.advance(
                &pending.base,
                &pending.source,
                crate::highlight::LEXICAL_BATCH_BYTES,
            );
        }
    }

    /// Both drivers retain the same row scanner. Yielding never publishes a
    /// partial token table; status and old-row reuse survive each continuation.
    pub(super) fn advance_lexical(
        &mut self,
        status: SyntaxStatus,
        should_continue: &mut impl FnMut() -> bool,
        should_yield: &mut impl FnMut() -> bool,
        change: Option<(&Arc<String>, &crate::editor::TextChange)>,
    ) -> Option<SyntaxStatus> {
        if !matches!(status, SyntaxStatus::Ready { .. })
            || self.provider.is_some()
            || self.language == Language::Plain
            || self
                .lexical
                .as_ref()
                .is_some_and(|snapshot| Arc::ptr_eq(snapshot.source_snapshot(), &self.text))
        {
            return Some(status);
        }
        let (_, lexical) = self.lexical_pending.get_or_insert_with(|| {
            let mut lexical =
                crate::highlight::LexicalPreparation::new(self.text.clone(), self.language);
            if let Some(previous) = &self.lexical {
                lexical = if let Some((base, change)) = change {
                    lexical.reuse_validated_change(previous.clone(), base, change)
                } else {
                    lexical.reuse_cooperative(previous.clone())
                };
            }
            (status, lexical)
        });
        while !lexical.is_complete() {
            if !should_continue() {
                self.clear();
                return Some(SyntaxStatus::Cancelled);
            }
            if should_yield() {
                return None;
            }
            lexical.advance_bounded(1, crate::highlight::LEXICAL_BATCH_BYTES);
        }
        let (status, lexical) = self.lexical_pending.take().expect("completed lexical job");
        self.lexical = Some(Arc::new(
            lexical.finish_snapshot().expect("completed lexical source"),
        ));
        Some(status)
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
                self.outer_fallback = Some(Arc::new(FallbackContexts::from_lexical(
                    work.fallback.finish().unwrap_or_default(),
                )));
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
    fn begin_embedded(&mut self, selected: Vec<(Language, Range)>) -> EmbeddedWork {
        #[cfg(test)]
        {
            self.embedded_parses = 0;
        }
        EmbeddedWork {
            next: Vec::with_capacity(selected.len()),
            selected: selected.into_iter(),
            old: Vec::new().into_iter(),
            retained: Some(std::mem::take(&mut self.embedded).into_iter()),
            changed: Vec::new(),
            unchanged: HashMap::new(),
            current: None,
        }
    }
}
impl EmbeddedWork {
    fn retain_body(
        &mut self,
        mut body: EmbeddedSyntax,
        edit: &InputEdit,
        rows: &[crate::editor::lines::Line],
    ) {
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
                start_point: indexed_point(rows, bytes.start),
                end_point: indexed_point(rows, bytes.end),
            };
            if body.range != range
                && let Some(tree) = body.tree.as_mut()
            {
                tree.edit(edit);
            }
            body.range = range;
            self.unchanged
                .entry((bytes.start, bytes.end))
                .or_default()
                .push(body);
        } else {
            self.changed.push(body);
        }
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
                self.embedded = Some(document.begin_embedded(selected));
                continue;
            }
            let embedded = self.embedded.as_mut().unwrap();
            // Charge matching one retained body to the current batch. Cancellation
            // and yield are checked by the same outer loop before the next body.
            if let Some(retained) = embedded.retained.as_mut() {
                if let Some(body) = retained.next() {
                    embedded.retain_body(body, &self.edit, &document.source_lines);
                } else {
                    embedded.old = std::mem::take(&mut embedded.changed).into_iter();
                    embedded.retained = None;
                }
                continue;
            }
            if embedded.current.is_none() {
                let Some((language, range)) = embedded.selected.next() else {
                    if self.fallback.advance(&self.source, FALLBACK_BATCH_BYTES) == Some(false) {
                        continue;
                    }
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
                        fallback: std::cell::RefCell::default(),
                        folds: std::cell::RefCell::default(),
                        contexts: std::cell::RefCell::default(),
                        highlights: std::cell::RefCell::default(),
                    });
                let mut previous = body.tree.take();
                if let Some(tree) = previous.as_mut() {
                    tree.edit(&self.edit);
                }
                body.range = range;
                // Only exact unchanged-body matches retain fallback metadata.
                // An intersecting/replacement body must rescan its own source.
                body.fallback.get_mut().take();
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
                    fallback: LexicalScan::new(language),
                    #[cfg(test)]
                    checks_at_start: self.checks,
                });
            }
            let current = embedded.current.as_mut().unwrap();
            if current.body.tree.is_none() {
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
                current.body.tree = Some(tree);
                current.previous = None;
                #[cfg(test)]
                {
                    document.embedded_parses += 1;
                }
            }
            if should_yield() {
                return Ok(false);
            }
            let range = current.body.range;
            if current.fallback.advance(
                &self.source[range.start_byte..range.end_byte],
                FALLBACK_BATCH_BYTES,
            ) == Some(false)
            {
                continue;
            }
            let current = embedded.current.take().unwrap();
            *current.body.fallback.borrow_mut() = Some(Arc::new(FallbackContexts::from_lexical(
                current.fallback.finish().unwrap_or_default(),
            )));
            embedded.next.push(current.body);
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

    fn admit_for_phase(document: &mut SyntaxDocument, source: &Arc<String>) {
        let mut admission = SyntaxAdmission::new(source.clone());
        assert_eq!(
            admission.advance(usize::MAX),
            SyntaxAdmissionStatus::Admitted
        );
        document.admission = Some(admission);
    }

    #[test]
    fn source_admission_yields_before_parsing_and_releases_cancelled_or_replaced_sources() {
        for language in [
            Language::Rust,
            Language::Sql,
            Language::Plain,
            Language::Markdown,
        ] {
            for ending in ["\n", "\r\n"] {
                let mut document = SyntaxDocument::new(language).unwrap();
                let initial = Arc::new("fn original() {}".to_owned());
                document
                    .prepare_shared(initial.clone(), 4, || true)
                    .1
                    .unwrap();
                let source = Arc::new(format!(
                    "/*{}*/{ending}fn changed() {{}}",
                    "文😀 ".repeat(40_000)
                ));
                let weak = Arc::downgrade(&source);
                let mut checks = 0;
                assert!(
                    document
                        .prepare_cooperative(
                            source.clone(),
                            4,
                            || true,
                            || {
                                checks += 1;
                                checks >= 2
                            }
                        )
                        .is_none()
                );
                let admission = document.admission.as_ref().unwrap();
                assert_eq!(admission.status(), SyntaxAdmissionStatus::Pending);
                assert!(Arc::ptr_eq(admission.source_snapshot(), &source));
                assert!(document.pending.is_none());
                assert!(document.lexical_pending.is_none());
                assert!(document.source_comparison.is_none());
                assert!(document.prepared.is_none());
                assert!(document.structure().is_none());
                assert!(document.folds().is_empty());
                assert!(Arc::ptr_eq(&document.text, &initial));
                drop(source);

                let replacement = Arc::new(format!(
                    "/*{}*/{ending}fn replacement() {{}}",
                    "文😀 ".repeat(41_000)
                ));
                assert!(
                    document
                        .prepare_cooperative(replacement.clone(), 4, || true, || true)
                        .is_none()
                );
                assert!(weak.upgrade().is_none());
                let weak = Arc::downgrade(&replacement);
                let (status, analysis) = document
                    .prepare_cooperative(replacement.clone(), 4, || false, || false)
                    .unwrap();
                assert_eq!(status, SyntaxStatus::Cancelled);
                assert!(analysis.is_none());
                assert!(document.admission.is_none());
                drop(replacement);
                assert!(weak.upgrade().is_none());

                let replacement = Arc::new(format!(
                    "/*{}*/{ending}fn final_source() {{}}",
                    "文😀 ".repeat(40_000)
                ));
                let replacement = if language == Language::Markdown {
                    Arc::new(format!(
                        "Paragraph {}{ending}{ending}# Final source",
                        "x".repeat(300_000)
                    ))
                } else {
                    replacement
                };
                let mut turns = 0;
                let analysis = loop {
                    turns += 1;
                    assert!(turns < 10_000);
                    let mut checks = 0;
                    if let Some((status, analysis)) = document.prepare_cooperative(
                        replacement.clone(),
                        4,
                        || true,
                        || {
                            checks += 1;
                            checks >= if turns == 1 { 2 } else { 8 }
                        },
                    ) {
                        assert_eq!(
                            status,
                            SyntaxStatus::Ready { incremental: false },
                            "{language:?} {ending:?}"
                        );
                        break analysis.unwrap();
                    }
                };
                assert!(turns > 1);
                let (_, expected) = SyntaxDocument::new(language).unwrap().prepare_shared(
                    replacement.clone(),
                    4,
                    || true,
                );
                assert_eq!(
                    serde_json::to_value(analysis.transfer_data()).unwrap(),
                    serde_json::to_value(expected.unwrap().transfer_data()).unwrap()
                );
                assert!(Arc::ptr_eq(analysis.source_snapshot(), &replacement));
                assert!(document.admission.is_none());
            }
        }
    }

    #[test]
    fn cooperative_admission_preserves_exact_row_and_byte_limits() {
        for (source, expected) in [
            (
                "\n".repeat(49_999),
                SyntaxStatus::Ready { incremental: false },
            ),
            ("\r\n".repeat(50_000), SyntaxStatus::TooLarge),
            (
                "x".repeat(super::super::MAX_STRUCTURE_BYTES),
                SyntaxStatus::Ready { incremental: false },
            ),
            (
                "x".repeat(super::super::MAX_STRUCTURE_BYTES + 1),
                SyntaxStatus::TooLarge,
            ),
        ] {
            let source = Arc::new(source);
            let mut document = SyntaxDocument::new(Language::Plain).unwrap();
            let mut turns = 0;
            let (status, analysis) = loop {
                turns += 1;
                assert!(turns < 1_000);
                let mut checks = 0;
                if let Some(result) = document.prepare_cooperative(
                    source.clone(),
                    4,
                    || true,
                    || {
                        checks += 1;
                        checks >= 2
                    },
                ) {
                    break result;
                }
                assert!(document.pending.is_none());
            };
            assert_eq!(status, expected);
            assert_eq!(
                analysis.is_some(),
                matches!(expected, SyntaxStatus::Ready { .. })
            );
            assert!(document.admission.is_none());
        }
    }

    #[test]
    fn parser_source_comparison_yields_and_matches_fresh_analysis() {
        for (language, prefix, suffix) in [
            (Language::Rust, "/*", "*/\nfn main() { let value = 1; }"),
            (Language::Sql, "SELECT ", "\nSELECT 'done';"),
            (Language::Plain, "Plain ", "\ntail"),
            (
                Language::TypeScript,
                "/*",
                "*/\nfunction main() { const value = 1; }",
            ),
            (
                Language::Markdown,
                "Paragraph ",
                "\n\n# Heading\n\n**done**",
            ),
        ] {
            for ending in ["\n", "\r\n"] {
                let body = if language == Language::Markdown {
                    format!("words 文😀{}", "x".repeat(300_000))
                } else {
                    "words 文😀 ".repeat(20_000)
                };
                let source = Arc::new(format!("{prefix}{body}{suffix}").replace('\n', ending));
                let mut document = SyntaxDocument::new(language).unwrap();
                let (status, original) = document.prepare_shared(source.clone(), 4, || true);
                let original = original.unwrap_or_else(|| {
                    panic!("{language:?} {ending:?}: initial status {status:?}")
                });
                let changed = Arc::new(source.replacen("words", "revised", 1));
                admit_for_phase(&mut document, &changed);
                let mut checks = 0;
                assert!(
                    document
                        .prepare_cooperative(
                            changed.clone(),
                            4,
                            || true,
                            || {
                                checks += 1;
                                checks >= 2
                            }
                        )
                        .is_none()
                );
                assert!(document.source_comparison.is_some());
                assert!(document.pending.is_none());
                assert!(document.prepared.is_none());
                assert!(document.structure().is_none());
                assert!(document.folds().is_empty());
                assert!(Arc::ptr_eq(&document.text, &source));
                assert!(Arc::ptr_eq(original.source_snapshot(), &source));
                let mut turns = 0;
                let analysis = loop {
                    turns += 1;
                    assert!(turns < 10_000);
                    let mut checks = 0;
                    if let Some((status, analysis)) = document.prepare_cooperative(
                        changed.clone(),
                        4,
                        || true,
                        || {
                            checks += 1;
                            checks >= 8
                        },
                    ) {
                        assert_eq!(
                            status,
                            SyntaxStatus::Ready {
                                incremental: document.provider.is_some()
                            }
                        );
                        break analysis.unwrap();
                    }
                    assert!(document.prepared.is_none());
                };
                assert!(Arc::ptr_eq(analysis.source_snapshot(), &changed));
                let (_, expected) = SyntaxDocument::new(language).unwrap().prepare_shared(
                    changed.clone(),
                    4,
                    || true,
                );
                assert_eq!(
                    serde_json::to_value(analysis.transfer_data()).unwrap(),
                    serde_json::to_value(expected.unwrap().transfer_data()).unwrap()
                );
                assert!(document.source_comparison.is_none());
            }
        }
    }

    #[test]
    fn resolved_parser_delta_skips_comparison_only_for_its_exact_ready_base() {
        let source = Arc::new(format!(
            "/*{}*/\nfn main() {{}}",
            "words 文😀 ".repeat(20_000)
        ));
        let changed = Arc::new(source.replacen("words", "changed", 1));
        let change = crate::editor::text_change(&source, &changed).unwrap();
        for trusted in [false, true] {
            let mut document = SyntaxDocument::new(Language::Rust).unwrap();
            document
                .prepare_shared(source.clone(), 4, || true)
                .1
                .unwrap();
            admit_for_phase(&mut document, &changed);
            let base = if trusted {
                source.clone()
            } else {
                Arc::new(source.as_ref().clone())
            };
            assert!(
                document
                    .prepare_resolved_cooperative(
                        changed.clone(),
                        4,
                        &mut || true,
                        &mut || true,
                        Some((&base, &change))
                    )
                    .is_none()
            );
            assert_eq!(document.source_comparison.is_some(), !trusted);
            assert_eq!(document.pending.is_some(), trusted);
            let (status, analysis) = document
                .prepare_resolved_cooperative(
                    changed.clone(),
                    4,
                    &mut || true,
                    &mut || false,
                    Some((&base, &change)),
                )
                .unwrap();
            assert_eq!(status, SyntaxStatus::Ready { incremental: true });
            assert!(Arc::ptr_eq(analysis.unwrap().source_snapshot(), &changed));
            assert!(document.source_comparison.is_none());
        }
    }

    #[test]
    fn parser_source_comparison_releases_superseded_and_cancelled_sources() {
        let source = Arc::new(format!(
            "/*{}*/\nfn main() {{}}",
            "words 文😀 ".repeat(20_000)
        ));
        let mut document = SyntaxDocument::new(Language::Rust).unwrap();
        document
            .prepare_shared(source.clone(), 4, || true)
            .1
            .unwrap();
        let changed = Arc::new(source.replacen("words", "changed", 1));
        let weak = Arc::downgrade(&changed);
        assert!(
            document
                .prepare_cooperative(changed.clone(), 4, || true, || true)
                .is_none()
        );
        drop(changed);
        let next = Arc::new(source.replacen("words", "replacement", 1));
        assert!(
            document
                .prepare_cooperative(next.clone(), 4, || true, || true)
                .is_none()
        );
        assert!(weak.upgrade().is_none());
        let weak = Arc::downgrade(&next);
        assert_eq!(
            document
                .prepare_cooperative(next.clone(), 4, || false, || false)
                .unwrap()
                .0,
            SyntaxStatus::Cancelled
        );
        drop(next);
        assert!(weak.upgrade().is_none());
        assert!(document.source_comparison.is_none());
        assert!(document.tree.is_none());
        let replacement = Arc::new("fn replacement() {}".to_owned());
        let (status, analysis) = document
            .prepare_cooperative(replacement.clone(), 4, || true, || false)
            .unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert!(Arc::ptr_eq(
            analysis.unwrap().source_snapshot(),
            &replacement
        ));
    }

    fn finish_plain_rows(
        document: &mut SyntaxDocument,
        source: Arc<String>,
    ) -> (SyntaxStatus, Arc<SyntaxAnalysis>, usize) {
        let mut turns = 0;
        loop {
            turns += 1;
            assert!(turns < 2_000, "plain-row preparation must advance");
            let mut checks = 0;
            let result = document.prepare_cooperative(
                source.clone(),
                4,
                || true,
                || {
                    checks += 1;
                    checks >= 4
                },
            );
            if let Some((status, analysis)) = result {
                return (status, analysis.unwrap(), turns);
            }
            assert!(
                document.prepared.is_none(),
                "unfinished rows publish no analysis"
            );
            assert!(
                document.admission.is_some()
                    || document.lexical_pending.is_some()
                    || document.source_comparison.is_some()
            );
        }
    }

    #[test]
    fn cancelling_inside_a_long_plain_row_discards_all_unpublished_work() {
        use crate::highlight::Language;
        for ending in ["\n", "\r\n"] {
            let source = Arc::new(format!(
                "{}{ending}tail",
                "SELECT value 文😀 ".repeat(40_000)
            ));
            let mut document = SyntaxDocument::new(Language::Sql).unwrap();
            for turn in 0..200 {
                let mut checks = 0;
                let result = document.prepare_cooperative(
                    source.clone(),
                    4,
                    || true,
                    || {
                        checks += 1;
                        checks >= 2
                    },
                );
                assert!(result.is_none());
                if document.lexical_pending.is_some() {
                    break;
                }
                assert!(turn < 199);
            }
            assert!(document.lexical.is_none() && document.prepared.is_none());
            let result = document
                .prepare_cooperative(source, 4, || false, || false)
                .unwrap();
            assert_eq!(result.0, SyntaxStatus::Cancelled);
            assert!(result.1.is_none());
            assert!(document.lexical_pending.is_none());
            let replacement = Arc::new("SELECT replacement;".to_owned());
            let (_, analysis, _) = finish_plain_rows(&mut document, replacement.clone());
            assert!(Arc::ptr_eq(analysis.source_snapshot(), &replacement));
        }
    }

    #[test]
    fn warm_plain_source_comparison_cancellation_keeps_replacement_owned() {
        for ending in ["\n", "\r\n"] {
            let source = Arc::new(format!(
                "header{ending}{}{ending}tail",
                "SELECT 文😀 ".repeat(40_000)
            ));
            let mut document = SyntaxDocument::new(Language::Sql).unwrap();
            let (_, original, _) = finish_plain_rows(&mut document, source.clone());
            let changed = Arc::new(source.replacen("header", "changed", 1));
            for turn in 0..200 {
                let mut checks = 0;
                let result = document.prepare_cooperative(
                    changed.clone(),
                    4,
                    || true,
                    || {
                        checks += 1;
                        checks >= 2
                    },
                );
                assert!(result.is_none());
                if document.lexical_pending.is_some() {
                    break;
                }
                assert!(turn < 199);
            }
            assert!(document.lexical_pending.is_some());
            assert!(Arc::ptr_eq(original.source_snapshot(), &source));
            let result = document
                .prepare_cooperative(changed, 4, || false, || false)
                .unwrap();
            assert_eq!(result.0, SyntaxStatus::Cancelled);
            assert!(result.1.is_none());
            assert!(document.lexical_pending.is_none());
            let replacement = Arc::new("SELECT fresh;".to_owned());
            let (_, analysis, _) = finish_plain_rows(&mut document, replacement.clone());
            assert!(Arc::ptr_eq(analysis.source_snapshot(), &replacement));
        }
    }

    #[test]
    fn plain_row_preparation_yields_reuses_rows_and_matches_synchronous_source() {
        assert!(syntax_provider(Language::Sql).is_none());
        for ending in ["\n", "\r\n"] {
            let source = Arc::new(
                (0..1_000)
                    .map(|row| format!("SELECT '文😀', {row};{ending}"))
                    .collect::<String>(),
            );
            let mut document = SyntaxDocument::new(Language::Sql).unwrap();
            let (status, analysis, turns) = finish_plain_rows(&mut document, source.clone());
            assert_eq!(status, SyntaxStatus::Ready { incremental: false });
            assert!(turns > 100);
            assert!(Arc::ptr_eq(analysis.source_snapshot(), &source));
            let (_, expected) = SyntaxDocument::new(Language::Sql).unwrap().prepare_shared(
                source.clone(),
                4,
                || true,
            );
            assert_eq!(analysis.highlights, expected.unwrap().highlights);
            let lexical = document.lexical.as_ref().unwrap().clone();
            let (_, width_changed) = document
                .prepare_cooperative(source.clone(), 8, || true, || true)
                .unwrap();
            assert!(Arc::ptr_eq(&lexical, document.lexical.as_ref().unwrap()));
            assert!(Arc::ptr_eq(
                analysis.highlights.as_ref().unwrap(),
                width_changed.unwrap().highlights.as_ref().unwrap()
            ));
            let changed =
                Arc::new(source.replacen("SELECT '文😀', 500;", "SELECT 'different', 500;", 1));
            let (status, changed_analysis, turns) =
                finish_plain_rows(&mut document, changed.clone());
            assert_eq!(status, SyntaxStatus::Ready { incremental: false });
            assert!(turns > 100);
            assert_eq!(document.lexical.as_ref().unwrap().retokenized_rows(), 1);
            let (_, expected) = SyntaxDocument::new(Language::Sql).unwrap().prepare_shared(
                changed.clone(),
                4,
                || true,
            );
            assert_eq!(changed_analysis.highlights, expected.unwrap().highlights);
            assert_eq!(
                analysis.source(),
                source.as_str(),
                "published sources stay immutable"
            );
        }
    }

    #[test]
    fn plain_row_continuations_cancel_replace_source_and_allow_synchronous_completion() {
        let source = Arc::new("SELECT '文😀';\r\n".repeat(1_000));
        let mut document = SyntaxDocument::new(Language::Sql).unwrap();
        admit_for_phase(&mut document, &source);
        assert!(
            document
                .prepare_cooperative(source.clone(), 4, || true, || true)
                .is_none()
        );
        assert!(document.lexical_pending.is_some());
        let mut checks = 0;
        let cancelled = document
            .prepare_cooperative(
                source.clone(),
                4,
                || {
                    checks += 1;
                    checks < 4
                },
                || false,
            )
            .unwrap();
        assert_eq!(checks, 4, "cancellation interrupts partially prepared rows");
        assert_eq!(cancelled.0, SyntaxStatus::Cancelled);
        assert!(cancelled.1.is_none());
        assert!(document.lexical_pending.is_none());
        assert!(
            document
                .prepare_cooperative(source.clone(), 4, || true, || true)
                .is_none()
        );
        let replacement = Arc::new("SELECT 'replacement';\n".repeat(800));
        let (_, analysis, turns) = finish_plain_rows(&mut document, replacement.clone());
        assert!(turns > 100);
        assert!(Arc::ptr_eq(analysis.source_snapshot(), &replacement));
        assert!(analysis.highlights.as_ref().unwrap().iter().all(|row| {
            row.iter()
                .all(|token| token.kind == crate::highlight::TokenKind::Plain)
        }));
        assert!(
            document
                .prepare_cooperative(source.clone(), 4, || true, || true)
                .is_none()
        );
        let (status, completed) = document.prepare_shared(source.clone(), 4, || true);
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert!(document.lexical_pending.is_none());
        assert_eq!(completed.unwrap().source(), source.as_str());
    }

    #[test]
    fn outer_and_embedded_large_literal_scans_yield_and_cancel() {
        let literal = "文😀".repeat(149_000);
        for (language, source, embedded) in [
            (Language::Rust, format!("let s = \"{literal}\";\n"), false),
            (
                Language::Yaml,
                format!("value: |\n  {literal}\nnext: true\n"),
                false,
            ),
            (
                Language::Markdown,
                format!("```yaml\nvalue: |\n  {literal}\nnext: true\n```\n"),
                true,
            ),
            (
                Language::Markdown,
                format!("```rust\nlet s = \"{literal}\";\n```\n"),
                true,
            ),
        ] {
            let source = Arc::new(source);
            assert!(source.len() > 1_000_000 && source.len() < 1_048_576);
            let mut document = SyntaxDocument::new(language).unwrap();
            admit_for_phase(&mut document, &source);
            let mut scan_batches = 0;
            let mut previous_position = 0;
            let mut total_batches = 0;
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
                total_batches += 1;
                assert!(total_batches < 10_000);
                if let Some(result) = result {
                    break result;
                }
                assert!(document.structure().is_none());
                let work = document.pending.as_ref().unwrap();
                let position = if embedded {
                    work.embedded
                        .as_ref()
                        .and_then(|bodies| bodies.current.as_ref())
                        .filter(|body| body.body.tree.is_some())
                        .map_or(0, |body| body.fallback.position())
                } else {
                    work.fallback.position()
                };
                if position > 0 {
                    assert!(position >= previous_position);
                    assert!(position - previous_position <= (FALLBACK_BATCH_BYTES + 4) * 8);
                    previous_position = position;
                    scan_batches += 1;
                }
            };
            assert!(
                scan_batches > 10,
                "the large literal cannot finish in one scan task"
            );
            assert_eq!(result.0, SyntaxStatus::Ready { incremental: false });
            let actual = result.1.unwrap();
            assert!(Arc::ptr_eq(actual.source_snapshot(), &source));
            let mut fresh = SyntaxDocument::new(language).unwrap();
            let (_, expected) = fresh.prepare_shared(source.clone(), 4, || true);
            assert_eq!(
                serde_json::to_value(actual.transfer_data()).unwrap(),
                serde_json::to_value(expected.unwrap().transfer_data()).unwrap()
            );

            let mut cancelled = SyntaxDocument::new(language).unwrap();
            let cancel_source = Arc::new(source.as_str().to_owned());
            let weak = Arc::downgrade(&cancel_source);
            admit_for_phase(&mut cancelled, &cancel_source);
            loop {
                let mut checks = 0;
                assert!(
                    cancelled
                        .prepare_cooperative(
                            cancel_source.clone(),
                            4,
                            || true,
                            || {
                                checks += 1;
                                checks >= 8
                            }
                        )
                        .is_none()
                );
                let work = cancelled.pending.as_ref().unwrap();
                let position = if embedded {
                    work.embedded
                        .as_ref()
                        .and_then(|bodies| bodies.current.as_ref())
                        .filter(|body| body.body.tree.is_some())
                        .map_or(0, |body| body.fallback.position())
                } else {
                    work.fallback.position()
                };
                if position > 0 {
                    break;
                }
            }
            assert_eq!(
                cancelled
                    .prepare_cooperative(cancel_source.clone(), 4, || false, || false)
                    .unwrap()
                    .0,
                SyntaxStatus::Cancelled
            );
            assert!(cancelled.pending.is_none() && cancelled.outer_fallback.is_none());
            assert!(cancelled.embedded.is_empty());
            drop(cancel_source);
            assert!(weak.upgrade().is_none());
            let replacement = Arc::new("fn recovered() {}\n".to_owned());
            assert!(matches!(
                cancelled
                    .prepare_cooperative(replacement, 4, || true, || false)
                    .unwrap()
                    .0,
                SyntaxStatus::Ready { .. }
            ));
        }
    }

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
    fn retained_body_matching_yields_and_cancels_without_partial_publication() {
        for ending in ["\n", "\r\n"] {
            let source = Arc::new(
                (0..1_000)
                    .map(|index| format!("Paragraph {index}: **文😀** and `code`.{ending}{ending}"))
                    .collect::<String>(),
            );
            let changed = Arc::new(source.replacen("Paragraph 500:", "Changed paragraph 500:", 1));
            let mut document = SyntaxDocument::new(Language::Markdown).unwrap();
            assert!(matches!(
                document.prepare_shared(source.clone(), 4, || true).0,
                SyntaxStatus::Ready { .. }
            ));
            let mut previous_remaining = None;
            let mut matching_batches = 0;
            let result = loop {
                let mut checks = 0;
                let result = document.prepare_cooperative(
                    changed.clone(),
                    4,
                    || true,
                    || {
                        checks += 1;
                        checks >= 8
                    },
                );
                if let Some(result) = result {
                    break result;
                }
                assert!(!document.ready);
                assert!(document.structure().is_none());
                if let Some(remaining) = document
                    .pending
                    .as_ref()
                    .and_then(|work| work.embedded.as_ref())
                    .and_then(|work| work.retained.as_ref())
                    .map(ExactSizeIterator::len)
                {
                    if let Some(previous) = previous_remaining {
                        assert!(remaining < previous);
                        assert!(
                            previous - remaining < 8,
                            "each body observes the batch boundary"
                        );
                    }
                    previous_remaining = Some(remaining);
                    matching_batches += 1;
                }
            };
            assert!(matching_batches > 100);
            assert_eq!(
                document.embedded_parses, 1,
                "matching preserves every unaffected tree"
            );
            let mut fresh = SyntaxDocument::new(Language::Markdown).unwrap();
            let (_, expected) = fresh.prepare_shared(changed.clone(), 4, || true);
            let expected = expected.unwrap();
            assert_eq!(
                serde_json::to_value(result.1.unwrap().transfer_data()).unwrap(),
                serde_json::to_value(expected.transfer_data()).unwrap()
            );

            // Cancel while old bodies are still being matched, then recover fresh.
            let replacement = Arc::new(changed.replacen("Paragraph 100:", "New paragraph 100:", 1));
            let weak = Arc::downgrade(&replacement);
            loop {
                let mut checks = 0;
                assert!(
                    document
                        .prepare_cooperative(
                            replacement.clone(),
                            4,
                            || true,
                            || {
                                checks += 1;
                                checks >= 8
                            }
                        )
                        .is_none()
                );
                if document
                    .pending
                    .as_ref()
                    .and_then(|work| work.embedded.as_ref())
                    .is_some_and(|work| {
                        work.retained
                            .as_ref()
                            .is_some_and(|bodies| bodies.len() > 0)
                    })
                {
                    break;
                }
            }
            assert_eq!(
                document
                    .prepare_cooperative(replacement.clone(), 4, || false, || false)
                    .unwrap()
                    .0,
                SyntaxStatus::Cancelled
            );
            assert!(document.pending.is_none());
            assert!(document.embedded.is_empty());
            drop(replacement);
            assert!(weak.upgrade().is_none());
            let (_, recovered) = document.prepare_shared(changed, 4, || true);
            assert_eq!(
                serde_json::to_value(recovered.unwrap().transfer_data()).unwrap(),
                serde_json::to_value(expected.transfer_data()).unwrap()
            );
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
        admit_for_phase(&mut document, &source);
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
