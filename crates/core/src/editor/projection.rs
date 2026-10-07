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
    source_range: Range<usize>,
    textarea_origin: usize,
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
            source_range: 0..source.len(),
            textarea_origin: 0,
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
    /// The native offsets in a context are local; this is its offset in the
    /// complete projected input. Source offsets always remain document offsets.
    pub fn textarea_origin(&self) -> usize {
        self.textarea_origin
    }

    pub fn source_range(&self) -> Range<usize> {
        self.source_range.clone()
    }

    pub fn is_windowed(&self) -> bool {
        self.source_range != (0..self.source_len)
    }

    /// Retain only this visible byte interval, including partial logical rows.
    /// A context is not a new document: folds and source row identities survive,
    /// while byte and native coordinates start at zero within the context.
    pub fn window(&self, range: Range<usize>) -> Result<Self, ProjectionError> {
        let between_crlf = |at: usize| {
            at > 0
                && self.text.as_bytes().get(at - 1) == Some(&b'\r')
                && self.text.as_bytes().get(at) == Some(&b'\n')
        };
        if range.start > range.end
            || range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
            || between_crlf(range.start)
            || between_crlf(range.end)
        {
            return Err(ProjectionError::InvalidOffset);
        }
        if range == (0..self.text.len()) {
            return Ok(self.clone());
        }
        let source_range = self.source_offset(range.start)?..self.source_offset(range.end)?;
        let textarea_origin = self.textarea_origin
            + self
                .byte_to_textarea(range.start)
                .map_err(|_| ProjectionError::InvalidOffset)?;
        let first = self
            .lines
            .partition_point(|line| line.visible_start <= range.start)
            .saturating_sub(1);
        let mut lines = Vec::new();
        let mut coordinates = Vec::new();
        let mut native = 0;
        for row in first..self.lines.len() {
            let line = &self.lines[row];
            if line.visible_start > range.end {
                break;
            }
            let end = self
                .lines
                .get(row + 1)
                .map_or(self.text.len(), |next| next.visible_start);
            let start = line.visible_start.max(range.start);
            let end = end.min(range.end);
            let slice = &self.text[start..end];
            let source_start = line.source.start + start - line.visible_start;
            lines.push(VisibleLine {
                source_line: line.source_line,
                source: source_start..source_start + slice.len(),
                visible_start: start - range.start,
                textarea_start: native,
            });
            coordinates.push(
                if start == line.visible_start && end - start == line.source.len() {
                    self.coordinates[row].clone()
                } else {
                    super::coordinates::LineCoordinates::new(slice)
                },
            );
            native += super::byte_to_textarea(slice, slice.len())
                .map_err(|_| ProjectionError::InvalidOffset)?;
        }
        let text: Arc<str> = self.text[range.clone()].into();
        let textarea_text = if text.contains('\r') {
            Arc::from(text.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            text.clone()
        };
        let uniform_rows = !text.as_bytes().iter().enumerate().any(|(offset, byte)| {
            *byte == b'\r' && text.as_bytes().get(offset + 1) != Some(&b'\n')
        });
        let first_gap = self
            .hidden
            .partition_point(|gap| gap.visible_offset <= range.start);
        let last_gap = self
            .hidden
            .partition_point(|gap| gap.visible_offset <= range.end);
        let hidden = self.hidden[first_gap..last_gap]
            .iter()
            .map(|gap| HiddenText {
                source: gap.source.clone(),
                visible_offset: gap.visible_offset - range.start,
                header_end: gap.header_end,
            })
            .collect::<Vec<_>>();
        Ok(Self {
            text,
            textarea_text,
            source_len: self.source_len,
            source_range,
            textarea_origin,
            lines: lines.into(),
            hidden: hidden.into(),
            uniform_rows,
            coordinates: coordinates.into(),
        })
    }

    /// Bound the browser's surrounding text around the primary selection head.
    /// Large selections remain in the document and are clipped only for native
    /// input; callers must retain the original selection for editing/clipboard.
    pub fn input_context(
        &self,
        selection: Selection,
        max_bytes: usize,
    ) -> Result<Self, ProjectionError> {
        if max_bytes < 4 {
            return Err(ProjectionError::InvalidOffset);
        }
        let head = self.visible_selection(selection)?.head;
        let head = if head > 0
            && self.text.as_bytes().get(head - 1) == Some(&b'\r')
            && self.text.as_bytes().get(head) == Some(&b'\n')
        {
            head - 1
        } else {
            head
        };
        let mut start = head.saturating_sub(max_bytes / 2);
        let mut end = start.saturating_add(max_bytes).min(self.text.len());
        start = end.saturating_sub(max_bytes);
        while !self.text.is_char_boundary(start) {
            start += 1;
        }
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        if start > 0
            && self.text.as_bytes().get(start - 1) == Some(&b'\r')
            && self.text.as_bytes().get(start) == Some(&b'\n')
        {
            start += 1;
        }
        if end > 0
            && self.text.as_bytes().get(end - 1) == Some(&b'\r')
            && self.text.as_bytes().get(end) == Some(&b'\n')
        {
            end -= 1;
        }
        self.window(start..end)
    }

    /// Selection for the native context, without changing document selections.
    pub fn input_selection(&self, selection: Selection) -> Result<Selection, ProjectionError> {
        if selection.anchor > self.source_len || selection.head > self.source_len {
            return Err(ProjectionError::InvalidOffset);
        }
        self.visible_selection(Selection {
            anchor: selection
                .anchor
                .clamp(self.source_range.start, self.source_range.end),
            head: selection
                .head
                .clamp(self.source_range.start, self.source_range.end),
        })
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
            return Ok(self.source_range.end);
        }
        let line = self
            .lines
            .partition_point(|line| line.visible_start <= visible)
            .saturating_sub(1);
        let line = &self.lines[line];
        Ok(line.source.start + visible - line.visible_start)
    }

    pub fn visible_offset(&self, source: usize) -> Result<usize, ProjectionError> {
        if source < self.source_range.start || source > self.source_range.end {
            return Err(ProjectionError::InvalidOffset);
        }
        if source == self.source_range.end {
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
                self.visible_offset(
                    gap.header_end
                        .clamp(self.source_range.start, self.source_range.end),
                )
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
    fn bounded_context_keeps_global_source_and_native_offsets_with_partial_rows() {
        for source in [
            "😀文e\u{301}\t\r\na\rbreak\nlast 😀\r\n",
            "header\r\nhidden 文\r\nend\r\nnext 😀\r\n",
        ] {
            let mut folds = FoldState::default();
            folds.set_ranges(
                vec![FoldRange {
                    start_line: 0,
                    end_line: 2,
                }],
                5,
            );
            for folded in [false, true] {
                if folded {
                    folds.collapse_all();
                }
                let full = FoldProjection::new(source, &folds);
                let boundaries: Vec<_> = (0..=full.text().len())
                    .filter(|&at| {
                        full.text().is_char_boundary(at)
                            && !(at > 0
                                && full.text().as_bytes().get(at - 1) == Some(&b'\r')
                                && full.text().as_bytes().get(at) == Some(&b'\n'))
                    })
                    .collect();
                for &start in &boundaries {
                    for &end in boundaries.iter().filter(|&&end| end >= start) {
                        let context = full.window(start..end).unwrap();
                        assert_eq!(context.text(), &full.text()[start..end]);
                        assert_eq!(
                            context.textarea_origin(),
                            full.byte_to_textarea(start).unwrap()
                        );
                        assert_eq!(
                            context.source_range(),
                            full.source_offset(start).unwrap()..full.source_offset(end).unwrap()
                        );
                        for &byte in boundaries
                            .iter()
                            .filter(|&&byte| start <= byte && byte <= end)
                        {
                            let local = byte - start;
                            let document = full.source_offset(byte).unwrap();
                            assert_eq!(context.source_offset(local), Ok(document));
                            assert_eq!(context.visible_offset(document), Ok(local));
                            assert_eq!(
                                context.textarea_origin()
                                    + context.byte_to_textarea(local).unwrap(),
                                full.byte_to_textarea(byte).unwrap()
                            );
                            let row = context
                                .lines()
                                .partition_point(|line| line.visible_start <= local)
                                .saturating_sub(1);
                            let full_row = full
                                .lines()
                                .partition_point(|line| line.visible_start <= byte)
                                .saturating_sub(1);
                            assert_eq!(
                                context.lines()[row].source_line,
                                full.lines()[full_row].source_line
                            );
                        }
                        for native in 0..=context.textarea_text().encode_utf16().count() {
                            let local = context.textarea_to_byte(native);
                            assert_eq!(
                                context.source_offset(local),
                                full.source_offset(
                                    full.textarea_to_byte(context.textarea_origin() + native)
                                )
                            );
                        }
                        if start > 0 {
                            assert_eq!(
                                context.visible_offset(0),
                                Err(ProjectionError::InvalidOffset)
                            );
                        }
                        if end < full.text().len() {
                            assert_eq!(
                                context.visible_offset(source.len()),
                                Err(ProjectionError::InvalidOffset)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn contexts_clip_large_backward_selections_without_changing_document_selections() {
        let source = "文😀abc\r\n".repeat(100_000);
        let full = FoldProjection::new(&source, &FoldState::default());
        let head = source.find("abc").unwrap() + 50_000 * "文😀abc\r\n".len();
        let selection = Selection {
            anchor: source.len(),
            head,
        };
        let context = full.input_context(selection, 4096).unwrap();
        assert!(context.is_windowed());
        assert!(context.text().len() <= 4096);
        let local = context.input_selection(selection).unwrap();
        assert_eq!(local.anchor, context.text().len());
        assert_eq!(context.source_offset(local.head), Ok(head));
        assert_eq!(
            context.source_offset(context.text().len()),
            Ok(context.source_range().end)
        );
        assert_ne!(
            context.source_offset(context.text().len()),
            Ok(source.len())
        );
        let nested = context.window(0..local.head).unwrap();
        assert_eq!(nested.textarea_origin(), context.textarea_origin());
        assert_eq!(nested.source_range().end, head);
        assert_eq!(full.visible_selection(selection).unwrap(), selection);
        assert_eq!(full.text(), source);
    }

    #[test]
    fn bounded_context_replay_edits_the_full_source_and_preserves_folded_text() {
        let source = "before\r\nheader\r\nhidden 文\r\nend\r\nnext 😀\r\nafter";
        let mut folds = FoldState::default();
        folds.set_ranges(
            vec![FoldRange {
                start_line: 1,
                end_line: 3,
            }],
            6,
        );
        folds.collapse_all();
        let full = FoldProjection::new(source, &folds);
        let at = source.find("next").unwrap();
        let context = full.input_context(Selection::caret(at), 18).unwrap();
        let local = context.visible_offset(at).unwrap();
        let local_native = context.byte_to_textarea(local).unwrap();
        let mut value = context.textarea_text().to_string();
        let local = super::super::utf16_to_byte(&value, local_native);
        value.insert(local, '🦀');
        let (result, selection) = context
            .replay_input(
                source,
                &value,
                Selection::caret(local + '🦀'.len_utf8()),
                "insertText",
                Selection::caret(at),
            )
            .unwrap();
        let mut expected = source.replace("\r\n", "\n");
        let at = expected.find("next").unwrap();
        expected.insert(at, '🦀');
        assert_eq!(result, expected);
        assert_eq!(selection, Selection::caret(at + '🦀'.len_utf8()));
        assert!(result.contains("hidden 文\nend\n"));
    }

    #[test]
    fn bounded_context_rejects_invalid_bounds_and_never_splits_unicode_or_crlf() {
        let full = FoldProjection::new("😀\r\n文\rnext", &FoldState::default());
        let reversed = Range { start: 9, end: 8 };
        for range in [1..4, 0..5, 5..6, reversed, 0..100] {
            assert_eq!(full.window(range), Err(ProjectionError::InvalidOffset));
        }
        for head in (0..=full.text().len()).filter(|&at| full.text().is_char_boundary(at)) {
            for budget in 4..=full.text().len() + 4 {
                let context = full.input_context(Selection::caret(head), budget).unwrap();
                assert!(context.text().len() <= budget);
                assert!(context.input_selection(Selection::caret(head)).is_ok());
                assert_eq!(
                    context.textarea_text(),
                    context.text().replace("\r\n", "\n").replace('\r', "\n")
                );
            }
        }
        assert_eq!(
            full.input_context(Selection::caret(0), 3),
            Err(ProjectionError::InvalidOffset)
        );
        assert_eq!(
            full.input_context(Selection::caret(100), 16),
            Err(ProjectionError::InvalidOffset)
        );
    }

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
        fn bounded_input_contexts_round_trip_arbitrary_unicode_and_folds(
            rows in proptest::collection::vec(".{0,30}", 3..12),
            budget in 4usize..128,
            location in 0usize..1000,
        ) {
            let source = rows.join("\r\n");
            let mut folds = FoldState::default();
            folds.set_ranges(vec![FoldRange { start_line: 0, end_line: rows.len() - 2 }], rows.len());
            folds.collapse_all();
            let full = FoldProjection::new(&source, &folds);
            let boundaries: Vec<_> = full.text().char_indices().map(|(at, _)| at).chain([full.text().len()]).filter(|&at| {
                !(at > 0 && full.text().as_bytes().get(at - 1) == Some(&b'\r') && full.text().as_bytes().get(at) == Some(&b'\n'))
            }).collect();
            let visible = boundaries[location % boundaries.len()];
            let caret = Selection::caret(full.source_offset(visible).unwrap());
            let context = full.input_context(caret, budget).unwrap();
            proptest::prop_assert!(context.text().len() <= budget);
            let local = context.input_selection(caret).unwrap();
            proptest::prop_assert_eq!(context.source_selection(local), Ok(caret));
            proptest::prop_assert_eq!(context.textarea_origin() + context.byte_to_textarea(local.head).unwrap(), full.byte_to_textarea(visible).unwrap());
        }

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
