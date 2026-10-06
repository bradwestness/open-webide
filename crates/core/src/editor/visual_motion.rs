//! Measured visual rows are data; movement and selection policy stay in Rust.
use super::{EditError, FoldProjection, SelectionError};
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_VISUAL_CARETS: usize = 65_536;

/// One insertion position; columns are measured in 1/64 CSS pixels.
/// A soft-wrap boundary can have a position on both adjacent visual rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualCaret {
    pub offset: usize,
    pub column: i64,
    pub row: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualLayout {
    pub(super) source: String,
    pub(super) projection: FoldProjection,
    pub(super) identity: String,
    rows: usize,
    carets: BTreeMap<usize, Vec<VisualCaret>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VisualGoal {
    pub column: i64,
    pub caret: VisualCaret,
    pub source_head: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MotionColumns {
    Logical(Vec<usize>),
    Visual {
        identity: String,
        goals: Vec<VisualGoal>,
    },
}

/// Bounded grapheme positions for the browser measurement adapter.
pub fn visual_caret_offsets(text: &str) -> Result<Vec<usize>, SelectionError> {
    let offsets: Vec<_> = text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .take(MAX_VISUAL_CARETS + 1)
        .collect();
    if offsets.len() > MAX_VISUAL_CARETS {
        return Err(SelectionError::TooLarge);
    }
    Ok(offsets)
}

/// Index a bounded logical line independently of the number of measured carets.
/// Each pair is a grapheme boundary in source bytes and browser UTF-16 units.
pub fn visual_line_offsets(text: &str) -> Result<Vec<(usize, usize)>, SelectionError> {
    if text.len() > super::MAX_STRUCTURE_BYTES {
        return Err(SelectionError::TooLarge);
    }
    let mut utf16 = 0;
    let mut offsets = Vec::new();
    for (byte, grapheme) in text.grapheme_indices(true) {
        offsets.push((byte, utf16));
        utf16 += grapheme.encode_utf16().count();
    }
    offsets.push((text.len(), utf16));
    Ok(offsets)
}

impl VisualLayout {
    /// Partial measurement is allowed, but every moved cursor must have its
    /// current and neighboring visual row. Missing coverage rejects atomically.
    pub fn new(
        source: &str,
        projection: FoldProjection,
        identity: String,
        rows: usize,
        carets: Vec<VisualCaret>,
    ) -> Result<Self, SelectionError> {
        if source.len() > super::MAX_STRUCTURE_BYTES || carets.len() > MAX_VISUAL_CARETS {
            return Err(SelectionError::TooLarge);
        }
        if identity.is_empty()
            || identity.len() > 1024
            || rows == 0
            || carets.is_empty()
            || carets.iter().any(|caret| {
                caret.row >= rows
                    || caret.column.unsigned_abs() > i64::MAX as u64 / 4
                    || caret.offset > projection.text().len()
            })
        {
            return Err(EditError::InvalidSelection.into());
        }
        let mut offsets: BTreeSet<_> = carets.iter().map(|caret| caret.offset).collect();
        for offset in projection
            .text()
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain(std::iter::once(projection.text().len()))
        {
            offsets.remove(&offset);
            if offsets.is_empty() {
                break;
            }
        }
        if !offsets.is_empty() {
            return Err(EditError::InvalidSelection.into());
        }
        let mut by_row: BTreeMap<usize, Vec<VisualCaret>> = BTreeMap::new();
        for caret in carets {
            by_row.entry(caret.row).or_default().push(caret);
        }
        for row in by_row.values_mut() {
            row.sort_by_key(|caret| (caret.column, caret.offset));
            row.dedup();
        }
        Ok(Self {
            source: source.into(),
            projection,
            identity,
            rows,
            carets: by_row,
        })
    }

    pub(super) fn target(
        &self,
        at: usize,
        down: bool,
        previous: Option<&VisualGoal>,
    ) -> Result<(usize, VisualGoal), SelectionError> {
        // Default affinity is the start of the following wrapped row. Vertical
        // motion retains its selected row when the same byte has two positions.
        let current = self
            .carets
            .values()
            .flat_map(|row| row.iter())
            .filter(|caret| caret.offset == at)
            .max_by_key(|caret| {
                (
                    previous
                        .is_some_and(|goal| goal.caret.offset == at && goal.caret.row == caret.row),
                    caret.row,
                )
            })
            .ok_or(EditError::InvalidSelection)?;
        let column = previous.map_or(current.column, |goal| goal.column);
        let target_row = if down {
            (current.row + 1).min(self.rows - 1)
        } else {
            current.row.saturating_sub(1)
        };
        let target = if target_row == current.row {
            current
        } else {
            self.carets
                .get(&target_row)
                .and_then(|row| {
                    row.iter()
                        .min_by_key(|caret| (caret.column.abs_diff(column), caret.offset))
                })
                .ok_or(EditError::InvalidSelection)?
        };
        Ok((
            target.offset,
            VisualGoal {
                column,
                caret: *target,
                source_head: 0,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Document, Indentation, Selection, SelectionMotion};

    #[test]
    fn long_line_index_keeps_unicode_byte_and_utf16_boundaries() {
        let text = "文😀e\u{301} ".repeat(20_000);
        assert_eq!(visual_caret_offsets(&text), Err(SelectionError::TooLarge));
        let offsets = visual_line_offsets(&text).unwrap();
        assert_eq!(&offsets[..5], &[(0, 0), (3, 1), (7, 3), (10, 5), (11, 6)]);
        assert_eq!(offsets.last(), Some(&(text.len(), 120_000)));
        assert_eq!(offsets.len(), 80_001);
        assert_eq!(
            visual_line_offsets(&"x".repeat(super::super::MAX_STRUCTURE_BYTES + 1)),
            Err(SelectionError::TooLarge)
        );
    }

    fn wrapped(doc: &Document, identity: &str) -> VisualLayout {
        let carets = [
            (0, 0, 0),
            (1, 10, 0),
            (2, 20, 0),
            (3, 30, 0),
            (3, 0, 1),
            (4, 10, 1),
            (5, 20, 1),
            (6, 30, 1),
            (7, 0, 2),
            (8, 10, 2),
            (9, 20, 2),
            (10, 0, 3),
            (11, 10, 3),
            (12, 20, 3),
            (13, 30, 3),
            (13, 0, 4),
            (14, 10, 4),
            (15, 20, 4),
            (16, 30, 4),
        ]
        .into_iter()
        .map(|(offset, column, row)| VisualCaret {
            offset,
            column,
            row,
        })
        .collect();
        VisualLayout::new(doc.text(), doc.projection(), identity.into(), 5, carets).unwrap()
    }

    #[test]
    fn vertical_motion_keeps_pixel_goals_wrap_affinity_and_selection_direction() {
        let mut doc = Document::new("abcdef\nxy\nabcdef");
        doc.set_selections(vec![Selection::caret(6), Selection::caret(16)])
            .unwrap();
        let layout = wrapped(&doc, "wide");
        let step = |doc: &mut Document, motion, extend| {
            doc.move_selections_with_layout(motion, extend, Indentation::default(), &layout)
                .unwrap()
        };
        step(&mut doc, SelectionMotion::Down, true);
        assert_eq!(
            doc.selections(),
            &[Selection { anchor: 6, head: 9 }, Selection::caret(16)]
        );
        step(&mut doc, SelectionMotion::Down, true);
        assert_eq!(
            doc.selections()[0],
            Selection {
                anchor: 6,
                head: 13
            }
        );
        step(&mut doc, SelectionMotion::Up, true);
        assert_eq!(doc.selections()[0], Selection { anchor: 6, head: 9 });
        step(&mut doc, SelectionMotion::Up, true);
        assert_eq!(doc.selections()[0], Selection::caret(6));
        assert!(!doc.is_dirty());
        assert_eq!(doc.revision(), 0);
        assert!(!doc.undo());
    }

    #[test]
    fn layout_changes_reset_goals_and_missing_rows_or_stale_sources_reject_atomically() {
        let mut doc = Document::new("abcdef\nxy\nabcdef");
        doc.set_selections(vec![Selection::caret(6), Selection::caret(16)])
            .unwrap();
        let layout = wrapped(&doc, "wide");
        doc.move_selections_with_layout(
            SelectionMotion::Down,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        let resized = wrapped(&doc, "narrow");
        doc.move_selections_with_layout(
            SelectionMotion::Down,
            false,
            Indentation::default(),
            &resized,
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(12));
        let missing = VisualLayout::new(
            doc.text(),
            doc.projection(),
            "missing".into(),
            5,
            vec![
                VisualCaret {
                    offset: 12,
                    column: 20,
                    row: 3,
                },
                VisualCaret {
                    offset: 16,
                    column: 30,
                    row: 4,
                },
            ],
        )
        .unwrap();
        let before = doc.clone();
        assert!(
            doc.move_selections_with_layout(
                SelectionMotion::Up,
                false,
                Indentation::default(),
                &missing
            )
            .is_err()
        );
        assert_eq!(doc, before);
        let mut different = Document::new("abcdeg\nxy\nabcdef");
        let before = different.clone();
        assert_eq!(
            different.move_selections_with_layout(
                SelectionMotion::Down,
                false,
                Indentation::default(),
                &layout
            ),
            Err(EditError::StaleContext.into())
        );
        assert_eq!(different, before);
    }

    #[test]
    fn fold_changes_invalidate_measurements_and_transient_caret_affinity() {
        use crate::editor::{FoldCommand, FoldRange};
        let mut doc = Document::new("header\nhidden\ntail");
        let layout = VisualLayout::new(
            doc.text(),
            doc.projection(),
            "fold".into(),
            3,
            vec![
                VisualCaret {
                    offset: 0,
                    column: 0,
                    row: 0,
                },
                VisualCaret {
                    offset: 7,
                    column: 0,
                    row: 1,
                },
                VisualCaret {
                    offset: 14,
                    column: 0,
                    row: 2,
                },
            ],
        )
        .unwrap();
        doc.move_selections_with_layout(
            SelectionMotion::Down,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        assert!(doc.visual_caret(0, "fold").is_some());
        doc.fold_state_mut().set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 1,
            }],
            3,
        );
        doc.fold_command(FoldCommand::Toggle(0));
        assert!(doc.visual_caret(0, "fold").is_none());
        let before = doc.clone();
        assert_eq!(
            doc.move_selections_with_layout(
                SelectionMotion::Down,
                false,
                Indentation::default(),
                &layout
            ),
            Err(EditError::StaleContext.into())
        );
        assert_eq!(doc, before);
    }

    #[test]
    fn measurements_cannot_split_graphemes_or_exceed_shared_limits() {
        let doc = Document::new("e\u{301}😀\r\n");
        assert_eq!(visual_caret_offsets(doc.text()).unwrap(), vec![0, 3, 7, 9]);
        assert!(
            VisualLayout::new(
                doc.text(),
                doc.projection(),
                "bad".into(),
                1,
                vec![VisualCaret {
                    offset: 1,
                    column: 0,
                    row: 0
                }]
            )
            .is_err()
        );
        assert!(visual_caret_offsets(&"a".repeat(MAX_VISUAL_CARETS)).is_err());
    }
}
