//! Incremental logical-line and UTF-16 coordinates owned by the source document.
use super::{
    EditError, Selection,
    coordinates::LineCoordinates,
    lines::{Line, lines, row_at},
};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LineIndex {
    pub rows: Vec<Line>,
    offsets: Vec<(usize, usize)>,
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
            coordinates: Vec::new(),
            utf16_len: 0,
            textarea_len: 0,
        };
        for row in &index.rows {
            index.offsets.push((index.utf16_len, index.textarea_len));
            let text = &source[row.start..row.end];
            index.coordinates.push(LineCoordinates::new(text));
            let units = text.encode_utf16().count();
            index.utf16_len += units;
            index.textarea_len += units - usize::from(text.ends_with("\r\n"));
        }
        index
    }

    /// Re-scan changed logical rows, preserving all unchanged suffix coordinates.
    /// The edit envelope includes every replacement in the transaction.
    pub fn update(&mut self, old: &str, new: &str, changed: Range<usize>, new_end: usize) {
        let start_row = row_at(&self.rows, changed.start).saturating_sub(1);
        let mut end_row = (row_at(&self.rows, changed.end) + 1).min(self.rows.len());
        if self
            .rows
            .get(end_row)
            .is_some_and(|row| row.start == old.len())
        {
            end_row = self.rows.len();
        }
        let start = self.rows[start_row].start;
        let old_end = self.rows.get(end_row).map_or(old.len(), |row| row.start);
        let suffix = old.len() - old_end;
        let end = new.len() - suffix;
        debug_assert!(start <= changed.start && changed.end <= old_end && new_end <= end);
        let mut replacement = Self::new(&new[start..end]);
        if end < new.len()
            && replacement
                .rows
                .last()
                .is_some_and(|row| row.start == end - start)
        {
            replacement.rows.pop();
            replacement.offsets.pop();
            replacement.coordinates.pop();
        }
        let (raw_start, native_start) = self.offsets[start_row];
        let (raw_end, native_end) = self
            .offsets
            .get(end_row)
            .copied()
            .unwrap_or((self.utf16_len, self.textarea_len));
        let next_raw = raw_start + replacement.utf16_len;
        let next_native = native_start + replacement.textarea_len;
        for row in &mut self.rows[end_row..] {
            row.start = end + (row.start - old_end);
            row.body_end = end + (row.body_end - old_end);
            row.end = end + (row.end - old_end);
        }
        for (raw, native) in &mut self.offsets[end_row..] {
            *raw = next_raw + (*raw - raw_end);
            *native = next_native + (*native - native_end);
        }
        for row in &mut replacement.rows {
            row.start += start;
            row.body_end += start;
            row.end += start;
        }
        for (raw, native) in &mut replacement.offsets {
            *raw += raw_start;
            *native += native_start;
        }
        self.utf16_len = next_raw + (self.utf16_len - raw_end);
        self.textarea_len = next_native + (self.textarea_len - native_end);
        self.rows.splice(start_row..end_row, replacement.rows);
        self.offsets.splice(start_row..end_row, replacement.offsets);
        self.coordinates
            .splice(start_row..end_row, replacement.coordinates);
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
    fn long_row_coordinates_survive_edits_history_composition_and_folds() {
        use super::super::{Document, Edit, FoldCommand, FoldRange, NativeInputKind};
        fn verify(document: &Document) {
            assert_eq!(document.line_index, LineIndex::new(document.text()));
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
            assert_eq!(document.line_index, LineIndex::new(document.text()));
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
            index.update(&old, &new, changed.clone(), changed.start + inserted.len());
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
