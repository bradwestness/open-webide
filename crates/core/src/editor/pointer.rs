//! Pointer selection units and drag direction are source policy, independent of pixels.
use super::{Document, EditError, Selection, lines::row_at};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Caret,
    Word,
    Line,
}

/// A pointer gesture retains its original source unit while dragging either way.
/// The caller must discard it when the source document changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerSelection {
    anchor: Range<usize>,
    unit: Unit,
    revision: u64,
}

impl Document {
    fn pointer_unit(&self, offset: usize, unit: Unit) -> Result<Range<usize>, EditError> {
        if !super::valid_position(&self.text, offset) {
            return Err(EditError::InvalidSelection);
        }
        let index = row_at(&self.line_index.rows, offset);
        let row = &self.line_index.rows[index];
        if unit == Unit::Line {
            return Ok(row.start..row.end);
        }
        let body = &self.text[row.start..row.body_end];
        let at = offset.min(row.body_end) - row.start;
        let snapped = self.line_index.coordinates[index]
            .visual()
            .and_then(|coordinates| {
                coordinates
                    .index_at_byte(body, at)
                    .and_then(|glyph| coordinates.at(body, glyph))
                    .map(|(byte, _)| byte)
            })
            .unwrap_or_else(|| {
                if at == body.len() {
                    at
                } else {
                    body.grapheme_indices(true)
                        .take_while(|(byte, _)| *byte <= at)
                        .last()
                        .map_or(0, |(byte, _)| byte)
                }
            });
        if unit == Unit::Caret || body.is_empty() {
            return Ok(row.start + snapped..row.start + snapped);
        }
        let mut start = 0;
        let mut previous = None;
        for (byte, glyph) in body.grapheme_indices(true) {
            let kind = super::motion::category(glyph);
            if previous.is_some_and(|before| before != kind) {
                if snapped < byte {
                    return Ok(row.start + start..row.start + byte);
                }
                start = byte;
            }
            previous = Some(kind);
        }
        Ok(row.start + start..row.body_end)
    }

    pub fn begin_pointer_selection(
        &mut self,
        offset: usize,
        clicks: u32,
        extend: bool,
    ) -> Result<PointerSelection, EditError> {
        let unit = match clicks {
            0 | 1 => Unit::Caret,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let hit = self.pointer_unit(offset, unit)?;
        let anchor = if extend {
            let anchor = self.selections[0].anchor;
            anchor..anchor
        } else {
            hit
        };
        let pointer = PointerSelection {
            anchor,
            unit,
            revision: self.revision,
        };
        self.drag_pointer_selection(&pointer, offset)?;
        Ok(pointer)
    }

    pub fn drag_pointer_selection(
        &mut self,
        pointer: &PointerSelection,
        offset: usize,
    ) -> Result<bool, EditError> {
        if self.revision != pointer.revision {
            return Err(EditError::StaleContext);
        }
        let hit = self.pointer_unit(offset, pointer.unit)?;
        let selection = if hit.start < pointer.anchor.start {
            Selection {
                anchor: pointer.anchor.end,
                head: hit.start,
            }
        } else {
            Selection {
                anchor: pointer.anchor.start,
                head: hit.end.max(pointer.anchor.end),
            }
        };
        let changed = self.selections != [selection];
        self.set_selections(vec![selection])?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_units_preserve_unicode_direction_and_crlf() {
        let text = "a\u{301}bc  文_foo!\r\nlast";
        let mut document = Document::new(text);
        let pointer = document.begin_pointer_selection(1, 1, false).unwrap();
        assert_eq!(document.selections(), &[Selection::caret(0)]);
        document.drag_pointer_selection(&pointer, 4).unwrap();
        assert_eq!(document.selections(), &[Selection { anchor: 0, head: 4 }]);
        let word = document.begin_pointer_selection(10, 2, false).unwrap();
        assert_eq!(&text[document.selections()[0].range()], "文_foo");
        document.drag_pointer_selection(&word, 0).unwrap();
        assert_eq!(
            &text[document.selections()[0].range()],
            "a\u{301}bc  文_foo"
        );
        assert!(document.selections()[0].anchor > document.selections()[0].head);
        let line = document
            .begin_pointer_selection(text.len(), 3, false)
            .unwrap();
        assert_eq!(&text[document.selections()[0].range()], "last");
        document.drag_pointer_selection(&line, 0).unwrap();
        assert_eq!(
            document.selections()[0],
            Selection {
                anchor: text.len(),
                head: 0
            }
        );
        assert!(!document.is_dirty());
        assert!(!document.undo());
    }
    #[test]
    fn shift_pointer_uses_existing_anchor_and_rejects_invalid_offsets() {
        let mut document = Document::new("one two\r\n");
        document
            .set_selections(vec![Selection { anchor: 5, head: 6 }])
            .unwrap();
        let pointer = document.begin_pointer_selection(1, 1, true).unwrap();
        assert_eq!(document.selections(), &[Selection { anchor: 5, head: 1 }]);
        document.drag_pointer_selection(&pointer, 7).unwrap();
        assert_eq!(document.selections(), &[Selection { anchor: 5, head: 7 }]);
        let before = document.clone();
        assert_eq!(
            document.begin_pointer_selection(10, 1, false),
            Err(EditError::InvalidSelection)
        );
        assert_eq!(document, before);
        let mut document = Document::new("");
        document.begin_pointer_selection(0, 3, false).unwrap();
        assert_eq!(document.selections(), &[Selection::caret(0)]);
    }
    #[test]
    fn edits_invalidate_pointer_gestures_without_changing_new_selections() {
        let mut document = Document::new("one two");
        let pointer = document.begin_pointer_selection(1, 2, false).unwrap();
        document
            .apply(
                vec![super::super::Edit::replace(0..3, "new")],
                vec![Selection::caret(3)],
                None,
            )
            .unwrap();
        let before = document.clone();
        assert_eq!(
            document.drag_pointer_selection(&pointer, 6),
            Err(EditError::StaleContext)
        );
        assert_eq!(document, before);
    }
}
