//! A folded edit view is a projection of the source, never a second document.
use std::{ops::Range, sync::Arc};

use super::{FoldState, Selection};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleLine {
    /// Zero-based logical source line, independent of folded view rows.
    pub source_line: usize,
    pub source: Range<usize>,
    pub visible_start: usize,
    /// UTF-16 offset in the native textarea, whose CRLF endings normalize to LF.
    pub textarea_start: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HiddenText {
    source: Range<usize>,
    visible_offset: usize,
    header_end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    InvalidOffset,
    /// Expand the fold before replaying an edit that crosses its hidden gap.
    HiddenText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldProjection {
    text: Arc<str>,
    textarea_text: Arc<str>,
    source_len: usize,
    lines: Arc<[VisibleLine]>,
    hidden: Arc<[HiddenText]>,
    uniform_rows: bool,
    coordinates: Arc<[super::coordinates::LineCoordinates]>,
}

impl FoldProjection {
    pub fn new(source: &str, folds: &FoldState) -> Self {
        Self::indexed(source, folds, &super::index::LineIndex::new(source))
    }

    pub(super) fn indexed(
        source: &str,
        folds: &FoldState,
        index: &super::index::LineIndex,
    ) -> Self {
        let logical = &index.rows;
        let mut text = String::new();
        let mut visible = Vec::new();
        let mut coordinates = Vec::new();
        let mut hidden: Vec<HiddenText> = Vec::new();
        let mut row = 0;
        let mut textarea_start = 0;
        while row < logical.len() {
            let line = &logical[row];
            visible.push(VisibleLine {
                source_line: row,
                source: line.start..line.end,
                visible_start: text.len(),
                textarea_start,
            });
            coordinates.push(index.coordinates[row].clone());
            text.push_str(&source[line.start..line.end]);
            textarea_start += index.native_line_len(row);
            if let Some(range) = folds.collapsed_at(row) {
                let end = range.end_line.min(logical.len() - 1);
                if end > row {
                    let omitted = logical[row + 1].start..logical[end].end;
                    if !omitted.is_empty() {
                        hidden.push(HiddenText {
                            source: omitted,
                            visible_offset: text.len(),
                            header_end: line.body_end,
                        });
                    }
                    row = end;
                }
            }
            row += 1;
        }
        // A trailing newline always creates a logical empty input row. Keep its
        // source identity even when a provider included it in a fold's range.
        if text.ends_with('\n')
            && visible
                .last()
                .is_some_and(|line| line.source.start != source.len())
        {
            coordinates.push(index.coordinates[logical.len() - 1].clone());
            visible.push(VisibleLine {
                source_line: logical.len() - 1,
                source: source.len()..source.len(),
                visible_start: text.len(),
                textarea_start,
            });
        }
        // Lone CR normalizes to a native line break inside one logical source
        // row. Such rows need measured heights rather than fixed-row windowing.
        let uniform_rows = !text.as_bytes().iter().enumerate().any(|(offset, byte)| {
            *byte == b'\r' && text.as_bytes().get(offset + 1) != Some(&b'\n')
        });
        let text: Arc<str> = text.into();
        let textarea_text = if text.contains('\r') {
            Arc::from(text.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            text.clone()
        };
        Self {
            text,
            textarea_text,
            uniform_rows,
            coordinates: coordinates.into(),
            source_len: source.len(),
            lines: visible.into(),
            hidden: hidden.into(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn textarea_text(&self) -> &str {
        &self.textarea_text
    }
    pub fn visual_line_index(&self, row: usize) -> Option<super::VisualLineIndex> {
        self.coordinates.get(row)?.visual()
    }
    pub fn textarea_to_byte(&self, offset: usize) -> usize {
        let row = self
            .lines
            .partition_point(|line| line.textarea_start <= offset)
            .saturating_sub(1);
        let line = &self.lines[row];
        let end = self
            .lines
            .get(row + 1)
            .map_or(self.text.len(), |next| next.visible_start);
        line.visible_start
            + self.coordinates[row].textarea_to_byte(
                &self.text[line.visible_start..end],
                offset.saturating_sub(line.textarea_start),
            )
    }
    pub fn byte_to_textarea(&self, offset: usize) -> Result<usize, super::EditError> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return Err(super::EditError::InvalidSelection);
        }
        let row = self
            .lines
            .partition_point(|line| line.visible_start <= offset)
            .saturating_sub(1);
        let line = &self.lines[row];
        let end = self
            .lines
            .get(row + 1)
            .map_or(self.text.len(), |next| next.visible_start);
        Ok(line.textarea_start
            + self.coordinates[row].byte_to_textarea(
                &self.text[line.visible_start..end],
                offset - line.visible_start,
            )?)
    }
    pub fn source_native_selection(
        &self,
        selection: Selection,
    ) -> Result<Selection, ProjectionError> {
        self.source_selection(Selection {
            anchor: self.textarea_to_byte(selection.anchor),
            head: self.textarea_to_byte(selection.head),
        })
    }
    pub fn lines(&self) -> &[VisibleLine] {
        &self.lines
    }
    pub fn has_uniform_rows(&self) -> bool {
        self.uniform_rows
    }

    pub fn is_folded(&self) -> bool {
        !self.hidden.is_empty()
    }

    fn hidden_at(&self, source: usize) -> Option<&HiddenText> {
        let index = self
            .hidden
            .partition_point(|gap| gap.source.start <= source)
            .checked_sub(1)?;
        self.hidden
            .get(index)
            .filter(|gap| gap.source.contains(&source))
    }

    /// At a hidden gap, a caret belongs to the next visible source row.
    pub fn source_offset(&self, visible: usize) -> Result<usize, ProjectionError> {
        if visible > self.text.len() || !self.text.is_char_boundary(visible) {
            return Err(ProjectionError::InvalidOffset);
        }
        if visible == self.text.len() {
            return Ok(self.source_len);
        }
        let line = self
            .lines
            .partition_point(|line| line.visible_start <= visible)
            .saturating_sub(1);
        let line = &self.lines[line];
        Ok(line.source.start + visible - line.visible_start)
    }

    pub fn visible_offset(&self, source: usize) -> Result<usize, ProjectionError> {
        if source > self.source_len {
            return Err(ProjectionError::InvalidOffset);
        }
        if source == self.source_len {
            return Ok(self.text.len());
        }
        if self.hidden_at(source).is_some() {
            return Err(ProjectionError::HiddenText);
        }
        let line = self
            .lines
            .partition_point(|line| line.source.start <= source)
            .saturating_sub(1);
        let line = &self.lines[line];
        let visible = line.visible_start + source - line.source.start;
        if visible > self.text.len() || !self.text.is_char_boundary(visible) {
            return Err(ProjectionError::InvalidOffset);
        }
        Ok(visible)
    }

    pub fn source_selection(&self, visible: Selection) -> Result<Selection, ProjectionError> {
        Ok(Selection {
            anchor: self.source_offset(visible.anchor)?,
            head: self.source_offset(visible.head)?,
        })
    }

    /// Collapsing a selection endpoint moves it to its containing visible header.
    pub fn visible_selection(&self, source: Selection) -> Result<Selection, ProjectionError> {
        let endpoint = |position| match self.visible_offset(position) {
            Err(ProjectionError::HiddenText) => {
                let gap = self.hidden_at(position).unwrap();
                self.visible_offset(gap.header_end)
            }
            result => result,
        };
        Ok(Selection {
            anchor: endpoint(source.anchor)?,
            head: endpoint(source.head)?,
        })
    }

    /// Replay an input-only event against the full document when the browser did
    /// not emit a usable beforeinput event. Return normalized textarea text/offsets.
    pub fn replay_input(
        &self,
        source: &str,
        value: &str,
        selection: Selection,
        input_type: &str,
        before_selection: Selection,
    ) -> Result<(String, Selection), ProjectionError> {
        if source.len() != self.source_len {
            return Err(ProjectionError::InvalidOffset);
        }
        let before = self.text.replace("\r\n", "\n").replace('\r', "\n");
        let full = source.replace("\r\n", "\n").replace('\r', "\n");
        let map = |offset| {
            let utf16 = super::byte_to_utf16(&before, offset)
                .map_err(|_| ProjectionError::InvalidOffset)?;
            let visible = super::textarea_to_byte(&self.text, utf16);
            let original = self.source_offset(visible)?;
            let utf16 = super::byte_to_textarea(source, original)
                .map_err(|_| ProjectionError::InvalidOffset)?;
            Ok::<_, ProjectionError>(super::utf16_to_byte(&full, utf16))
        };
        let Some(change) = super::text_change(&before, value) else {
            let selection = Selection {
                anchor: map(selection.anchor)?,
                head: map(selection.head)?,
            };
            return Ok((full, selection));
        };
        let raw_start = super::textarea_to_byte(
            &self.text,
            super::byte_to_utf16(&before, change.range.start)
                .map_err(|_| ProjectionError::InvalidOffset)?,
        );
        let raw_end = super::textarea_to_byte(
            &self.text,
            super::byte_to_utf16(&before, change.range.end)
                .map_err(|_| ProjectionError::InvalidOffset)?,
        );
        let crosses =
            self.source_edit_range(raw_start..raw_end) == Err(ProjectionError::HiddenText);
        let mut range = map(change.range.start)?..map(change.range.end)?;
        let inserted = &value[change.range.start..change.new_end];
        // Deleting the projected newline at a fold boundary means one native
        // logical newline, never the hidden block between the visible rows.
        if crosses && inserted.is_empty() && before_selection.range().is_empty() {
            if input_type == "deleteContentBackward" || input_type == "deleteWordBackward" {
                let count = if input_type == "deleteWordBackward" {
                    word_delete_len(full[..range.end].chars().rev())
                } else {
                    full[..range.end]
                        .chars()
                        .next_back()
                        .map_or(0, char::len_utf8)
                };
                range.start = range.end - count;
            } else if input_type == "deleteContentForward" || input_type == "deleteWordForward" {
                let count = full[range.start..].chars().next().map_or(0, char::len_utf8);
                range.end = range.start + count;
            }
        }
        let endpoint = |position: usize| -> Result<usize, ProjectionError> {
            if position > value.len() || !value.is_char_boundary(position) {
                return Err(ProjectionError::InvalidOffset);
            }
            if change.range.start <= position && position <= change.new_end {
                return Ok(range.start + position - change.range.start);
            }
            let old = if position < change.range.start {
                position
            } else {
                position - change.new_end + change.range.end
            };
            let original = map(old)?;
            Ok(if original >= range.end {
                original - range.end + range.start + inserted.len()
            } else {
                original.min(range.start)
            })
        };
        let mapped = Selection {
            anchor: endpoint(selection.anchor)?,
            head: endpoint(selection.head)?,
        };
        let mut result = full.clone();
        result.replace_range(range, inserted);
        Ok((result, mapped))
    }

    /// A native replacement must not silently consume bytes omitted from the view.
    pub fn source_edit_range(
        &self,
        visible: Range<usize>,
    ) -> Result<Range<usize>, ProjectionError> {
        if visible.start > visible.end {
            return Err(ProjectionError::InvalidOffset);
        }
        let start = self.source_offset(visible.start)?;
        let end = self.source_offset(visible.end)?;
        let gap = self
            .hidden
            .partition_point(|gap| gap.visible_offset <= visible.start);
        if self
            .hidden
            .get(gap)
            .is_some_and(|gap| gap.visible_offset <= visible.end)
        {
            return Err(ProjectionError::HiddenText);
        }
        Ok(start..end)
    }
}

fn word_delete_len(chars: impl Iterator<Item = char>) -> usize {
    let mut chars = chars.peekable();
    let mut count = 0;
    while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
        count += chars.next().unwrap().len_utf8();
    }
    let Some(first) = chars.next() else {
        return count;
    };
    count += first.len_utf8();
    let word = first.is_alphanumeric() || first == '_';
    while chars
        .peek()
        .is_some_and(|ch| !ch.is_whitespace() && (ch.is_alphanumeric() || *ch == '_') == word)
    {
        count += chars.next().unwrap().len_utf8();
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::FoldRange;

    #[test]
    fn document_projection_reuses_immutable_storage_and_invalidates_on_edits_and_folds() {
        use crate::editor::{Document, Edit, FoldCommand};
        let mut document = Document::new("head 😀\r\nbody 文\r\nend\r\nnext");
        let before = document.clone();
        let first = document.projection();
        assert_eq!(
            document, before,
            "preparing a derived view does not mutate document identity"
        );
        let again = document.projection();
        assert!(Arc::ptr_eq(&first.text, &again.text));
        assert!(Arc::ptr_eq(&first.lines, &again.lines));
        assert!(Arc::ptr_eq(&first.textarea_text, &again.textarea_text));
        for offset in 0..=first.textarea_text.encode_utf16().count() + 2 {
            assert_eq!(
                first.textarea_to_byte(offset),
                crate::editor::textarea_to_byte(first.text(), offset)
            );
        }
        document
            .apply(
                vec![Edit::replace(0..4, "header")],
                vec![Selection::caret(0)],
                None,
            )
            .unwrap();
        let changed = document.projection();
        assert!(!Arc::ptr_eq(&first.text, &changed.text));
        assert!(first.text().starts_with("head 😀"));
        document.fold_state_mut().set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        document.fold_command(FoldCommand::CollapseAll);
        let folded = document.projection();
        assert_eq!(folded.text(), "header 😀\r\nnext");
        assert_eq!(folded.textarea_text(), "header 😀\nnext");
        for offset in 0..=folded.textarea_text.encode_utf16().count() + 2 {
            assert_eq!(
                folded.textarea_to_byte(offset),
                crate::editor::textarea_to_byte(folded.text(), offset)
            );
        }
        assert_eq!(
            folded
                .source_native_selection(Selection::caret(folded.lines()[1].textarea_start))
                .unwrap()
                .head,
            document.text().find("next").unwrap()
        );
        assert_eq!(document.projection(), folded);
        document.fold_command(FoldCommand::ExpandAll);
        assert_eq!(document.projection().text(), changed.text());
    }

    proptest::proptest! {
        #[test]
        fn every_visible_unicode_boundary_round_trips(rows in proptest::collection::vec(".*", 3..20)) {
            let source = rows.join("\r\n");
            let count = source.split('\n').count();
            let mut state = FoldState::default();
            state.set_ranges(vec![FoldRange { start_line: 0, end_line: count - 2 }], count);
            state.collapse_all();
            let projection = FoldProjection::new(&source, &state);
            for offset in 0..=projection.text.len() {
                if projection.text.is_char_boundary(offset) {
                    let original = projection.source_offset(offset).unwrap();
                    proptest::prop_assert_eq!(projection.visible_offset(original).unwrap(), offset);
                }
            }
        }
    }

    #[test]
    fn window_offsets_preserve_native_utf16_after_unicode_crlf_and_folds() {
        assert!(FoldProjection::new("a\r\nb", &FoldState::default()).has_uniform_rows());
        assert!(!FoldProjection::new("a\rb", &FoldState::default()).has_uniform_rows());
        let source = "head 😀\r\nbody 文\r\nend\r\nnext 🦀";
        let mut folds = FoldState::default();
        folds.set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        for collapsed in [false, true] {
            if collapsed {
                folds.collapse_all();
            }
            let projection = FoldProjection::new(source, &folds);
            for line in projection.lines() {
                assert_eq!(
                    line.textarea_start,
                    projection.text()[..line.visible_start]
                        .replace("\r\n", "\n")
                        .encode_utf16()
                        .count()
                );
            }
        }
    }

    #[test]
    fn input_only_events_replay_unicode_and_boundary_deletion_without_losing_hidden_code() {
        let source = "head\r\nbody 文\r\nend\r\nnext 😀";
        let mut state = FoldState::default();
        state.set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        state.collapse_all();
        let projection = FoldProjection::new(source, &state);
        let next = source.find("next").unwrap();
        let (text, selection) = projection
            .replay_input(
                source,
                "head\nXnext 😀",
                Selection::caret(6),
                "insertText",
                Selection::caret(next),
            )
            .unwrap();
        assert_eq!(text, "head\nbody 文\nend\nXnext 😀");
        assert_eq!(selection.head, text.find("Xnext").unwrap() + 1);
        let (text, selection) = projection
            .replay_input(
                source,
                "headnext 😀",
                Selection::caret(4),
                "deleteContentBackward",
                Selection::caret(next),
            )
            .unwrap();
        assert_eq!(text, "head\nbody 文\nendnext 😀");
        assert_eq!(selection.head, text.find("next").unwrap());
        let (text, _) = projection
            .replay_input(
                source,
                "headnext 😀",
                Selection::caret(4),
                "deleteContentForward",
                Selection::caret(4),
            )
            .unwrap();
        assert_eq!(text, "headbody 文\nend\nnext 😀");
        let (text, _) = projection
            .replay_input(
                source,
                "headnext 😀",
                Selection::caret(4),
                "deleteContentBackward",
                Selection {
                    anchor: 4,
                    head: next,
                },
            )
            .unwrap();
        assert_eq!(
            text, "headnext 😀",
            "explicit selections still include folded source"
        );
    }

    #[test]
    fn unicode_crlf_projection_preserves_logical_rows_and_bidirectional_offsets() {
        let source = "😀 {\r\n    文\r\n}\r\nnext 😀\r\n";
        let mut folds = FoldState::default();
        folds.set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            5,
        );
        folds.toggle(0);
        let projection = FoldProjection::new(source, &folds);
        assert_eq!(projection.text(), "😀 {\r\nnext 😀\r\n");
        assert_eq!(
            projection
                .lines
                .iter()
                .map(|line| line.source_line)
                .collect::<Vec<_>>(),
            vec![0, 3, 4]
        );
        for offset in 0..=projection.text.len() {
            if projection.text.is_char_boundary(offset) {
                let source = projection.source_offset(offset).unwrap();
                assert_eq!(projection.visible_offset(source).unwrap(), offset);
            }
        }
        assert_eq!(
            projection.visible_offset(source.find('文').unwrap()),
            Err(ProjectionError::HiddenText)
        );
        assert_eq!(
            projection.visible_offset(1),
            Err(ProjectionError::InvalidOffset)
        );
        assert_eq!(
            projection.source_offset(1),
            Err(ProjectionError::InvalidOffset)
        );
        let source_selection = Selection {
            anchor: source.len(),
            head: source.find("next").unwrap(),
        };
        assert_eq!(
            projection
                .source_selection(projection.visible_selection(source_selection).unwrap())
                .unwrap(),
            source_selection
        );
        assert_eq!(
            projection
                .visible_selection(Selection::caret(source.find('文').unwrap()))
                .unwrap(),
            Selection::caret("😀 {".len())
        );
    }

    #[test]
    fn hidden_gaps_require_reveal_before_native_deletion_but_not_next_row_insertion() {
        let source = "head\nbody\nend\nnext";
        let mut folds = FoldState::default();
        folds.set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        folds.collapse_all();
        let projection = FoldProjection::new(source, &folds);
        assert_eq!(
            projection.source_edit_range(4..5),
            Err(ProjectionError::HiddenText)
        );
        assert_eq!(
            projection.source_edit_range(0..6),
            Err(ProjectionError::HiddenText)
        );
        assert_eq!(projection.source_edit_range(5..5).unwrap(), 14..14);
        assert_eq!(projection.source_edit_range(5..6).unwrap(), 14..15);
        assert_eq!(
            projection.source_edit_range(std::ops::Range { start: 6, end: 5 }),
            Err(ProjectionError::InvalidOffset)
        );
    }

    #[test]
    fn nested_folds_reveal_outer_rows_without_losing_inner_collapse_and_eof_identity() {
        let source = "outer\ninner\nbody\nclose\nlast\n";
        let mut folds = FoldState::default();
        folds.set_ranges(
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 4,
                },
                FoldRange {
                    start_line: 1,
                    end_line: 3,
                },
            ],
            6,
        );
        folds.collapse_all();
        assert_eq!(FoldProjection::new(source, &folds).text(), "outer\n");
        folds.toggle(0);
        let projection = FoldProjection::new(source, &folds);
        assert_eq!(projection.text(), "outer\ninner\nlast\n");
        assert_eq!(projection.lines.last().unwrap().source_line, 5);
        assert_eq!(
            projection.source_offset(projection.text.len()).unwrap(),
            source.len()
        );
        folds.expand_all();
        assert_eq!(FoldProjection::new(source, &folds).text(), source);
        assert_eq!(FoldProjection::new("", &folds).source_offset(0).unwrap(), 0);
    }
}
