//! Native input replay policy. Adapters supply full source text and UTF-8 selections.
//! Composition previews edit only the primary range; secondary edits commit together.
use super::{Document, Edit, EditError, Selection, text_change};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeInputKind {
    Insert,
    DeleteBackward,
    DeleteForward,
    Other,
}
impl NativeInputKind {
    pub fn from_input_type(kind: &str) -> Self {
        match kind {
            "insertText"
            | "insertFromPaste"
            | "insertFromDrop"
            | "insertReplacementText"
            | "insertFromComposition"
            | "insertCompositionText"
            | "insertLineBreak"
            | "insertParagraph" => Self::Insert,
            "deleteContentBackward"
            | "deleteWordBackward"
            | "deleteSoftLineBackward"
            | "deleteHardLineBackward" => Self::DeleteBackward,
            "deleteContentForward"
            | "deleteWordForward"
            | "deleteSoftLineForward"
            | "deleteHardLineForward" => Self::DeleteForward,
            "deleteByCut" => Self::Insert,
            _ => Self::Other,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Composition {
    before: Box<Document>,
    span: Option<Range<usize>>,
    group: Option<u64>,
}
impl Composition {
    pub(super) fn mark_saved_version(&mut self, text: &str) {
        self.before.mark_saved_version(text);
    }
}
fn left(text: &str, at: usize, count: usize) -> usize {
    text[..at]
        .grapheme_indices(true)
        .rev()
        .nth(count.saturating_sub(1))
        .map_or(0, |(offset, _)| offset)
}
fn right(text: &str, at: usize, count: usize) -> usize {
    text[at..]
        .grapheme_indices(true)
        .nth(count)
        .map_or(text.len(), |(offset, _)| at + offset)
}
fn inserted<'a>(before: &str, candidate: &'a str, span: &Range<usize>) -> Option<&'a str> {
    let start = span.start;
    let end = candidate.len().checked_sub(before.len() - span.end)?;
    (start <= end
        && candidate.starts_with(&before[..start])
        && candidate.ends_with(&before[span.end..]))
    .then(|| &candidate[start..end])
}
fn extent(
    before: &Document,
    candidate: &str,
    kind: NativeInputKind,
) -> Result<Range<usize>, EditError> {
    let primary = before.selections[0].range();
    if inserted(&before.text, candidate, &primary).is_some() {
        return Ok(primary);
    }
    let change = text_change(&before.text, candidate).ok_or(EditError::UnsupportedNativeInput)?;
    let span = if primary.is_empty()
        && matches!(
            kind,
            NativeInputKind::DeleteBackward | NativeInputKind::DeleteForward
        ) {
        let count = before.text[change.range.clone()].graphemes(true).count();
        if count == 0 {
            return Err(EditError::UnsupportedNativeInput);
        }
        if kind == NativeInputKind::DeleteBackward {
            left(&before.text, primary.start, count)..primary.end
        } else {
            primary.start..right(&before.text, primary.end, count)
        }
    } else {
        change.range.start.min(primary.start)..change.range.end.max(primary.end)
    };
    inserted(&before.text, candidate, &span).ok_or(EditError::UnsupportedNativeInput)?;
    Ok(span)
}
fn changes(
    before: &Document,
    span: &Range<usize>,
    text: &str,
    primary: Selection,
) -> Result<Vec<(Edit, Selection)>, EditError> {
    let selected = before.selections[0].range();
    let backwards = before.text[span.start..selected.start]
        .graphemes(true)
        .count();
    let forwards = before.text[selected.end..span.end].graphemes(true).count();
    let ranges: Vec<_> = before
        .selections
        .iter()
        .enumerate()
        .map(|(index, selection)| {
            if index == 0 {
                return span.clone();
            }
            let range = selection.range();
            let start = if backwards == 0 {
                range.start
            } else {
                left(&before.text, range.start, backwards)
            };
            let end = if forwards == 0 {
                range.end
            } else {
                right(&before.text, range.end, forwards)
            };
            start..end
        })
        .collect();
    let mut sorted = ranges.clone();
    sorted.sort_by_key(|range| (range.start, range.end));
    for pair in sorted.windows(2) {
        if pair[0].end > pair[1].start || pair[0].start == pair[1].start {
            return Err(EditError::OverlappingEdits);
        }
    }
    let retained = ranges
        .iter()
        .fold(before.text.len(), |size, range| size - range.len());
    let output = text
        .len()
        .checked_mul(ranges.len())
        .and_then(|size| size.checked_add(retained))
        .ok_or(EditError::OutputTooLarge)?;
    if output > super::MAX_DOCUMENT_BYTES {
        return Err(EditError::OutputTooLarge);
    }
    let end = span.start + text.len();
    if primary.anchor < span.start
        || primary.head < span.start
        || primary.anchor > end
        || primary.head > end
    {
        return Err(EditError::InvalidSelection);
    }
    Ok(ranges
        .into_iter()
        .enumerate()
        .map(|(index, range)| {
            let selection = if index == 0 {
                Selection {
                    anchor: primary.anchor - span.start,
                    head: primary.head - span.start,
                }
            } else {
                Selection::caret(text.len())
            };
            (Edit::replace(range, text), selection)
        })
        .collect())
}
impl Document {
    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }
    pub fn begin_composition(&mut self, group: Option<u64>) -> bool {
        if self.is_composing() {
            return false;
        }
        self.composition = Some(Box::new(Composition {
            before: Box::new(self.clone()),
            span: None,
            group,
        }));
        true
    }
    pub fn cancel_composition(&mut self) -> bool {
        let Some(composition) = self.composition.take() else {
            return false;
        };
        *self = *composition.before;
        true
    }
    pub fn end_composition(&mut self) -> Result<bool, EditError> {
        let Some(composition) = self.composition.take() else {
            return Ok(false);
        };
        let Composition {
            before,
            span,
            group,
        } = *composition;
        let mut result = *before;
        let revision = self.revision;
        let outcome = (|| {
            let Some(span) = span else {
                return Ok(false);
            };
            let text = inserted(&result.text, &self.text, &span)
                .ok_or(EditError::UnsupportedNativeInput)?;
            if result.text == self.text {
                return Ok(false);
            }
            let edits = changes(&result, &span, text, self.selections[0])?;
            result.apply_grouped_caret_edits(edits, group)
        })();
        if matches!(outcome, Ok(true)) {
            result.revision = revision.wrapping_add(1);
        }
        *self = result;
        outcome
    }
    pub fn native_input(
        &mut self,
        candidate: &str,
        after: Selection,
        kind: NativeInputKind,
        group: Option<u64>,
    ) -> Result<bool, EditError> {
        let composition = self.composition.take();
        let outcome =
            self.native_input_inner(candidate, after, kind, group, composition.as_deref());
        match (outcome, composition) {
            (Err(error), Some(composition)) => {
                *self = *composition.before;
                Err(error)
            }
            (Ok(changed), Some(mut composition)) => {
                if composition.span.is_none() && changed {
                    match extent(&composition.before, candidate, NativeInputKind::Insert) {
                        Ok(span) => composition.span = Some(span),
                        Err(error) => {
                            *self = *composition.before;
                            return Err(error);
                        }
                    }
                }
                self.composition = Some(composition);
                Ok(changed)
            }
            (outcome, None) => outcome,
        }
    }
    fn native_input_inner(
        &mut self,
        candidate: &str,
        after: Selection,
        kind: NativeInputKind,
        group: Option<u64>,
        composition: Option<&Composition>,
    ) -> Result<bool, EditError> {
        if candidate.len() > super::MAX_DOCUMENT_BYTES {
            return Err(EditError::OutputTooLarge);
        }
        super::validate_selections(candidate, &[after])?;
        if let Some(composition) = composition {
            let before = &composition.before;
            let span = match &composition.span {
                Some(span) => span.clone(),
                None => extent(before, candidate, NativeInputKind::Insert)?,
            };
            let text = inserted(&before.text, candidate, &span)
                .ok_or(EditError::UnsupportedNativeInput)?;
            // Validate the eventual replicated edit before publishing any preview.
            changes(before, &span, text, after)?;
            let mut selections = vec![after];
            for selection in before.selections.iter().skip(1) {
                let map = |at| {
                    if at <= span.start {
                        at
                    } else if at >= span.end {
                        span.start + text.len() + at - span.end
                    } else {
                        span.start + text.len()
                    }
                };
                selections.push(Selection {
                    anchor: map(selection.anchor),
                    head: map(selection.head),
                });
            }
            // Minimal spans use independent old/new ends; repeated text must not
            // move the primary edit to a different occurrence at commit time.
            let edit = text_change(&self.text, candidate).map(|change| {
                let start = change.range.start;
                Edit::replace(change.range, &candidate[start..change.new_end])
            });
            return self.apply(edit.into_iter().collect(), selections, composition.group);
        }
        if self.selections.len() == 1 || candidate == self.text {
            let edit = text_change(&self.text, candidate).map(|change| {
                let start = change.range.start;
                Edit::replace(change.range, &candidate[start..change.new_end])
            });
            let selections = if candidate == self.text && after == self.selections[0] {
                self.selections.clone()
            } else {
                vec![after]
            };
            return self.apply(edit.into_iter().collect(), selections, group);
        }
        if kind == NativeInputKind::Other {
            return Err(EditError::UnsupportedNativeInput);
        }
        let span = extent(self, candidate, kind)?;
        let text =
            inserted(&self.text, candidate, &span).ok_or(EditError::UnsupportedNativeInput)?;
        let edits = changes(self, &span, text, after)?;
        self.apply_grouped_caret_edits(edits, group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_insert_uses_the_selected_occurrence_in_ambiguous_repeated_text() {
        let mut doc = Document::new("fooo\r\nfooo");
        let selected = vec![
            Selection { anchor: 2, head: 3 },
            Selection { anchor: 8, head: 9 },
        ];
        doc.set_selections(selected.clone()).unwrap();
        doc.native_input(
            "foo\r\nfooo",
            Selection::caret(2),
            NativeInputKind::Insert,
            Some(1),
        )
        .unwrap();
        assert_eq!(doc.text(), "foo\r\nfoo");
        assert_eq!(
            doc.selections(),
            &[Selection::caret(2), Selection::caret(7)]
        );
        doc.undo();
        assert_eq!(doc.text(), "fooo\r\nfooo");
        assert_eq!(doc.selections(), selected);
    }
    #[test]
    fn backward_delete_maps_graphemes_instead_of_primary_byte_lengths() {
        let mut doc = Document::new("a\u{301}\r\n😀");
        let selected = vec![Selection::caret(3), Selection::caret(9)];
        doc.set_selections(selected.clone()).unwrap();
        doc.native_input(
            "\r\n😀",
            Selection::caret(0),
            NativeInputKind::DeleteBackward,
            None,
        )
        .unwrap();
        assert_eq!(doc.text(), "\r\n");
        assert_eq!(
            doc.selections(),
            &[Selection::caret(0), Selection::caret(2)]
        );
        doc.undo();
        assert_eq!(doc.text(), "a\u{301}\r\n😀");
        assert_eq!(doc.selections(), selected);
    }
    #[test]
    fn composition_previews_only_primary_and_commits_all_in_one_undo_step() {
        let mut doc = Document::new("文 foo\r\nfoo");
        let selected = vec![
            Selection {
                anchor: 12,
                head: 9,
            },
            Selection { anchor: 4, head: 7 },
        ];
        doc.set_selections(selected.clone()).unwrap();
        assert!(doc.begin_composition(Some(4)));
        assert!(!doc.begin_composition(Some(5)));
        for (candidate, caret) in [("文 foo\r\n文", 12), ("文 foo\r\n文字", 15)] {
            doc.native_input(
                candidate,
                Selection::caret(caret),
                NativeInputKind::Insert,
                Some(4),
            )
            .unwrap();
            assert_eq!(doc.text(), candidate);
            assert_eq!(doc.selections()[1], selected[1]);
            assert!(doc.is_composing());
        }
        assert!(doc.end_composition().unwrap());
        assert!(!doc.is_composing());
        assert_eq!(doc.text(), "文 文字\r\n文字");
        assert_eq!(
            doc.selections(),
            &[Selection::caret(18), Selection::caret(10)]
        );
        assert!(doc.undo());
        assert_eq!(doc.text(), "文 foo\r\nfoo");
        assert_eq!(doc.selections(), selected);
        assert!(!doc.can_undo());
        doc.redo();
        assert_eq!(doc.text(), "文 文字\r\n文字");
    }
    #[test]
    fn composition_supports_accent_replacement_before_a_collapsed_caret() {
        let mut doc = Document::new("e\r\n😀");
        doc.set_selections(vec![Selection::caret(1), Selection::caret(7)])
            .unwrap();
        doc.begin_composition(Some(8));
        doc.native_input(
            "é\r\n😀",
            Selection::caret(2),
            NativeInputKind::Insert,
            Some(8),
        )
        .unwrap();
        assert_eq!(doc.text(), "é\r\n😀");
        doc.end_composition().unwrap();
        assert_eq!(doc.text(), "é\r\né");
        doc.undo();
        assert_eq!(doc.text(), "e\r\n😀");
    }
    #[test]
    fn cancelled_or_invalid_composition_restores_redo_and_selection_history() {
        let mut doc = Document::new("ab");
        doc.replace_selections("x", None).unwrap();
        doc.undo();
        doc.set_selections(vec![Selection::caret(0), Selection::caret(2)])
            .unwrap();
        let before = doc.clone();
        doc.begin_composition(Some(10));
        doc.native_input("文ab", Selection::caret(3), NativeInputKind::Insert, None)
            .unwrap();
        assert_eq!(doc.text(), "文ab");
        assert!(doc.cancel_composition());
        assert_eq!(doc, before);
        doc.begin_composition(Some(11));
        assert_eq!(
            doc.native_input("文ab", Selection::caret(1), NativeInputKind::Insert, None),
            Err(EditError::InvalidSelection)
        );
        assert_eq!(doc, before);
        assert!(doc.can_redo());
    }
    #[test]
    fn returning_to_original_text_ends_composition_without_a_history_step() {
        let mut doc = Document::new("foo");
        doc.set_selections(vec![Selection { anchor: 0, head: 3 }])
            .unwrap();
        let before = doc.clone();
        doc.begin_composition(None);
        doc.native_input("bar", Selection::caret(3), NativeInputKind::Insert, None)
            .unwrap();
        doc.native_input("foo", Selection::caret(3), NativeInputKind::Insert, None)
            .unwrap();
        assert!(!doc.end_composition().unwrap());
        assert_eq!(doc, before);
    }
    #[test]
    fn foreign_commands_cannot_mutate_a_composing_document() {
        let mut doc = Document::new("ab");
        doc.begin_composition(Some(1));
        let before = doc.clone();
        assert_eq!(
            doc.replace_selections("x", None),
            Err(EditError::CompositionActive)
        );
        assert_eq!(
            doc.set_selections(vec![Selection::caret(1)]),
            Err(EditError::CompositionActive)
        );
        assert!(!doc.undo());
        assert_eq!(doc, before);
    }
    #[test]
    fn saved_versions_received_during_composition_survive_cancellation() {
        let mut doc = Document::new("a");
        doc.begin_composition(Some(1));
        doc.native_input("xa", Selection::caret(1), NativeInputKind::Insert, None)
            .unwrap();
        doc.mark_saved_version("xa");
        doc.cancel_composition();
        assert_eq!(doc.text(), "a");
        assert!(doc.is_dirty());
        doc.begin_composition(Some(2));
        doc.native_input("xa", Selection::caret(1), NativeInputKind::Insert, None)
            .unwrap();
        doc.mark_saved();
        doc.cancel_composition();
        assert!(doc.is_dirty());
    }
    #[test]
    fn overlapping_native_delete_and_unknown_input_leave_everything_unchanged() {
        let mut doc = Document::new("abcdef");
        doc.set_selections(vec![Selection::caret(3), Selection::caret(4)])
            .unwrap();
        let before = doc.clone();
        assert_eq!(
            doc.native_input(
                "def",
                Selection::caret(0),
                NativeInputKind::DeleteBackward,
                None
            ),
            Err(EditError::OverlappingEdits)
        );
        assert_eq!(doc, before);
        assert_eq!(
            doc.native_input("abcxdef", Selection::caret(4), NativeInputKind::Other, None),
            Err(EditError::UnsupportedNativeInput)
        );
        assert_eq!(doc, before);
    }
}
