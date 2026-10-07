//! Incremental logical-line and UTF-16 coordinates owned by the source document.
use super::{
    EditError, Selection,
    coordinates::LineCoordinates,
    lines::{Line, LineEdit, lines, row_at},
};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LineIndex {
    pub rows: Vec<Line>,
    offsets: Vec<(usize, usize, usize)>,
    oversized_rows: Vec<bool>,
    oversized_count: usize,
    breaks: usize,
    pub coordinates: Vec<LineCoordinates>,
    utf16_len: usize,
    textarea_len: usize,
}
impl LineIndex {
    pub fn new(source: &str) -> Self {
        let rows = lines(source);
        let mut index = Self {
            rows,
            offsets: Vec::new(),
            oversized_rows: Vec::new(),
            oversized_count: 0,
            breaks: 0,
            coordinates: Vec::new(),
            utf16_len: 0,
            textarea_len: 0,
        };
        for row in &index.rows {
            index
                .offsets
                .push((index.utf16_len, index.textarea_len, index.breaks));
            let text = &source[row.start..row.end];
            let (breaks, oversized) = super::capacity::row_admission(text);
            index.breaks += breaks;
            index.oversized_count += usize::from(oversized);
            index.oversized_rows.push(oversized);
            index.coordinates.push(LineCoordinates::new(text));
            let units = text.encode_utf16().count();
            index.utf16_len += units;
            index.textarea_len += units - usize::from(text.ends_with("\r\n"));
        }
        index
    }

    /// Re-scan changed logical rows, preserving all unchanged suffix coordinates.
    /// The edit envelope includes every replacement in one overlapping row batch.
    pub fn update(&mut self, old_len: usize, new: &str, changed: Range<usize>, new_end: usize) {
        let edit = LineEdit::new(&self.rows, old_len, new.len(), changed, new_end);
        let (start_row, end_row) = (edit.rows.start, edit.rows.end);
        let (start, end) = (edit.bytes.start, edit.bytes.end);
        let mut replacement = Self::new(&new[start..end]);
        if edit.trim_suffix_row(replacement.rows.last()) {
            replacement.rows.pop();
            replacement.offsets.pop();
            replacement.oversized_rows.pop();
            replacement.coordinates.pop();
        }
        let (raw_start, native_start, breaks_start) = self.offsets[start_row];
        let (raw_end, native_end, breaks_end) = self.offsets.get(end_row).copied().unwrap_or((
            self.utf16_len,
            self.textarea_len,
            self.breaks,
        ));
        let next_raw = raw_start + replacement.utf16_len;
        let next_native = native_start + replacement.textarea_len;
        let next_breaks = breaks_start + replacement.breaks;
        for (raw, native, breaks) in &mut self.offsets[end_row..] {
            *breaks = next_breaks + (*breaks - breaks_end);
            *raw = next_raw + (*raw - raw_end);
            *native = next_native + (*native - native_end);
        }
        for (raw, native, breaks) in &mut replacement.offsets {
            *breaks += breaks_start;
            *raw += raw_start;
            *native += native_start;
        }
        self.breaks = next_breaks + (self.breaks - breaks_end);
        self.oversized_count = self.oversized_count
            - self.oversized_rows[start_row..end_row]
                .iter()
                .filter(|invalid| **invalid)
                .count()
            + replacement.oversized_count;
        self.oversized_rows
            .splice(start_row..end_row, replacement.oversized_rows);
        self.utf16_len = next_raw + (self.utf16_len - raw_end);
        self.textarea_len = next_native + (self.textarea_len - native_end);
        edit.apply(&mut self.rows, replacement.rows);
        self.offsets.splice(start_row..end_row, replacement.offsets);
        self.coordinates
            .splice(start_row..end_row, replacement.coordinates);
    }
    pub fn admitted(&self, bytes: usize) -> bool {
        bytes <= super::MAX_EDITOR_BYTES
            && self.breaks < super::MAX_EDITOR_LINES
            && self.oversized_count == 0
    }

    pub fn row_breaks(&self, start: usize, end: usize) -> usize {
        self.offsets.get(end).map_or(self.breaks, |offset| offset.2) - self.offsets[start].2
    }

    pub fn native_line_len(&self, row: usize) -> usize {
        self.offsets
            .get(row + 1)
            .map_or(self.textarea_len, |offset| offset.1)
            - self.offsets[row].1
    }
    pub fn byte_to_textarea(&self, text: &str, offset: usize) -> Result<usize, EditError> {
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(EditError::InvalidSelection);
        }
        let row = row_at(&self.rows, offset);
        let line = &self.rows[row];
        // Include the following LF when the offset lies between CR and LF, so
        // the shared native mapping keeps the CR zero-width.
        let local = self.coordinates[row]
            .byte_to_textarea(&text[line.start..line.end], offset - line.start)?;
        Ok(self.offsets[row].1 + local)
    }
    pub fn textarea_to_byte(&self, text: &str, offset: usize) -> usize {
        let row = self
            .offsets
            .partition_point(|prefix| prefix.1 <= offset)
            .saturating_sub(1);
        let line = &self.rows[row];
        line.start
            + self.coordinates[row].textarea_to_byte(
                &text[line.start..line.end],
                offset.saturating_sub(self.offsets[row].1),
            )
    }
    pub fn native_selection(&self, text: &str, selection: Selection) -> Selection {
        Selection {
            anchor: self.textarea_to_byte(text, selection.anchor),
            head: self.textarea_to_byte(text, selection.head),
        }
    }
    pub fn line_column(&self, text: &str, offset: usize) -> (usize, usize) {
        let offset = text.floor_char_boundary(offset.min(text.len()));
        let row = row_at(&self.rows, offset);
        (
            row + 1,
            self.coordinates[row].chars_before(
                &text[self.rows[row].start..self.rows[row].end],
                offset.min(self.rows[row].body_end) - self.rows[row].start,
            ) + 1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_source_metadata_is_shared_and_rebuilt_only_for_changed_rows() {
        let body = "文😀e\u{301}\t words ".repeat(7000);
        let old = format!("head\r\n{body}\r\n{body}א\r\n");
        let mut index = LineIndex::new(&old);
        let ltr = index.coordinates[1].visual().unwrap();
        let rtl = index.coordinates[2].visual().unwrap();
        assert!(ltr.horizontal_paint_bounds(0.0, 400.0).is_some());
        assert!(rtl.horizontal_paint_bounds(0.0, 400.0).is_none());
        let next = old.replacen("head", "header changed", 1);
        index.update(old.len(), &next, 0..4, "header changed".len());
        assert!(index.coordinates[1].visual().unwrap().shared_with(&ltr));
        assert!(index.coordinates[2].visual().unwrap().shared_with(&rtl));
        let projection = super::super::FoldProjection::indexed(
            &next,
            &super::super::FoldState::default(),
            &index,
        );
        assert!(projection.visual_line_index(1).unwrap().shared_with(&ltr));
        let at = next.find(&body).unwrap();
        let mut changed = next.clone();
        changed.insert(at, 'א');
        index.update(next.len(), &changed, at..at, at + 'א'.len_utf8());
        let updated = index.coordinates[1].visual().unwrap();
        assert!(!updated.shared_with(&ltr));
        assert!(updated.horizontal_paint_bounds(0.0, 400.0).is_none());
        assert!(index.coordinates[2].visual().unwrap().shared_with(&rtl));
        assert_eq!(index, LineIndex::new(&changed));
    }
    #[test]
    fn long_row_coordinates_survive_edits_history_composition_and_folds() {
        use super::super::{Document, Edit, FoldCommand, FoldRange, NativeInputKind};
        fn verify(document: &Document) {
            assert_eq!(*document.line_index, LineIndex::new(document.text()));
            let projection = document.projection();
            for byte in document
                .text()
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([document.text().len()])
                .step_by(97)
            {
                assert_eq!(
                    document.byte_to_textarea(byte),
                    super::super::byte_to_textarea(document.text(), byte)
                );
                assert_eq!(
                    document.line_column(byte),
                    super::super::line_column(document.text(), byte)
                );
            }
            for byte in projection
                .text()
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([projection.text().len()])
                .step_by(83)
            {
                let native = super::super::byte_to_textarea(projection.text(), byte).unwrap();
                assert_eq!(projection.byte_to_textarea(byte).unwrap(), native);
                for near in native.saturating_sub(1)..=native + 1 {
                    assert_eq!(
                        projection.textarea_to_byte(near),
                        super::super::textarea_to_byte(projection.text(), near)
                    );
                }
            }
        }
        let long = "文😀e\u{301}\t\rword ".repeat(600);
        let source = format!("header {{\r\n{long}\r\n}}\r\n{long}\r\n");
        let mut document = Document::new(&source);
        verify(&document);
        document
            .apply(
                vec![
                    Edit::replace(0..0, "😀 new\r\n"),
                    Edit::replace(source.len()..source.len(), &long),
                ],
                vec![Selection::caret(0)],
                None,
            )
            .unwrap();
        verify(&document);
        assert!(document.undo());
        verify(&document);
        assert!(document.redo());
        verify(&document);
        assert!(document.begin_composition(Some(1)));
        document
            .native_input(
                &format!("文{}", document.text()),
                Selection::caret(3),
                NativeInputKind::Insert,
                Some(1),
            )
            .unwrap();
        verify(&document);
        document.cancel_composition();
        verify(&document);
        assert!(document.undo());
        document.set_fold_ranges(vec![FoldRange {
            start_line: 0,
            end_line: 2,
        }]);
        document.fold_command(FoldCommand::CollapseAll);
        verify(&document);
        assert!(document.projection().is_folded());
    }
    #[test]
    fn transactions_grouped_history_and_composition_keep_the_index_exact() {
        use super::super::{Document, Edit};
        fn verify(document: &Document) {
            assert_eq!(*document.line_index, LineIndex::new(document.text()));
            for offset in document
                .text()
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(document.text().len()))
            {
                assert_eq!(
                    document.byte_to_textarea(offset),
                    super::super::byte_to_textarea(document.text(), offset)
                );
            }
        }
        let source = "head 😀\r\nbody 文\nlast\r";
        let mut document = Document::new(source);
        for step in 0..10 {
            let end = document.text().len();
            document
                .apply(
                    vec![
                        super::super::Edit::replace(0..0, format!("文{step}\r\n")),
                        Edit::replace(end..end, "😀\n"),
                    ],
                    vec![Selection::caret(0)],
                    Some(step / 3),
                )
                .unwrap();
            verify(&document);
        }
        let final_text = document.text().to_owned();
        while document.undo() {
            verify(&document);
        }
        assert_eq!(document.text(), source);
        while document.redo() {
            verify(&document);
        }
        assert_eq!(document.text(), final_text);
        assert!(document.begin_composition(Some(999)));
        let preview = format!("文\r\n{}", document.text());
        document
            .native_input(
                &preview,
                Selection::caret(5),
                super::super::NativeInputKind::Insert,
                Some(999),
            )
            .unwrap();
        verify(&document);
        document.cancel_composition();
        verify(&document);
    }

    proptest::proptest! {
        #[test]
        fn incremental_coordinates_equal_full_reconstruction(
            old in "[a-z文😀\\r\\n\\t]{0,80}",
            inserted in "[a-z文😀\\r\\n\\t]{0,30}",
            first in proptest::num::usize::ANY,
            second in proptest::num::usize::ANY,
        ) {
            let boundaries: Vec<_> = old.char_indices().map(|(offset, _)| offset).chain(std::iter::once(old.len())).collect();
            let a = boundaries[first % boundaries.len()];
            let b = boundaries[second % boundaries.len()];
            let changed = a.min(b)..a.max(b);
            let new = format!("{}{}{}", &old[..changed.start], inserted, &old[changed.end..]);
            let mut index = LineIndex::new(&old);
            index.update(old.len(), &new, changed.clone(), changed.start + inserted.len());
            proptest::prop_assert_eq!(&index, &LineIndex::new(&new));
            for offset in new.char_indices().map(|(offset, _)| offset).chain(std::iter::once(new.len())) {
                proptest::prop_assert_eq!(index.byte_to_textarea(&new, offset), super::super::byte_to_textarea(&new, offset));
                proptest::prop_assert_eq!(index.line_column(&new, offset), super::super::line_column(&new, offset));
            }
            for offset in 0..=new.encode_utf16().count() + 2 {
                proptest::prop_assert_eq!(index.textarea_to_byte(&new, offset), super::super::textarea_to_byte(&new, offset));
            }
        }
    }
}
