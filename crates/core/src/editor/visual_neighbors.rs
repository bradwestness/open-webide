//! Neighbor relationships use logical lines and measured wraps, not estimated Y.
use super::{EditError, FoldProjection, MAX_VISUAL_CARETS, SelectionError};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualLineRows {
    /// Index in the folded projection, independent of original source line numbers.
    pub line: usize,
    pub rows: usize,
}

/// Sample endpoints for adjacent logical lines and the current cursor's nearest
/// visual rows. This works before other lines have height measurements.
pub fn visual_probe_rows(
    rows: usize,
    current: &[usize],
) -> Result<BTreeSet<usize>, SelectionError> {
    if rows == 0 || current.len() > MAX_VISUAL_CARETS {
        return Err(EditError::InvalidSelection.into());
    }
    let mut wanted = BTreeSet::from([0, rows - 1]);
    for &row in current {
        if row >= rows {
            return Err(EditError::InvalidSelection.into());
        }
        wanted.extend(row.saturating_sub(1)..=(row + 1).min(rows - 1));
        if wanted.len() > MAX_VISUAL_CARETS {
            return Err(SelectionError::TooLarge);
        }
    }
    Ok(wanted)
}

/// An ID is the projected logical-line start plus its visual-row ordinal. Its
/// numeric gaps are source gaps, never unmeasured or estimated screen rows.
pub fn visual_row_id(
    projection: &FoldProjection,
    line: usize,
    row: usize,
) -> Result<usize, SelectionError> {
    let start = projection
        .lines()
        .get(line)
        .ok_or(EditError::InvalidSelection)?
        .visible_start;
    let end = projection
        .lines()
        .get(line + 1)
        .map_or_else(|| projection.text().len() + 1, |next| next.visible_start);
    let id = start.checked_add(row).ok_or(SelectionError::TooLarge)?;
    if id >= end {
        return Err(EditError::InvalidSelection.into());
    }
    Ok(id)
}

type Neighbors = BTreeMap<usize, (Option<usize>, Option<usize>)>;
pub(super) fn neighbors(
    projection: &FoldProjection,
    lines: &[VisualLineRows],
    ids: &BTreeSet<usize>,
) -> Result<Neighbors, SelectionError> {
    if lines.len() > MAX_VISUAL_CARETS || ids.len() > MAX_VISUAL_CARETS {
        return Err(SelectionError::TooLarge);
    }
    let mut counts = BTreeMap::new();
    for line in lines {
        if line.rows == 0 || counts.insert(line.line, line.rows).is_some() {
            return Err(EditError::InvalidSelection.into());
        }
        visual_row_id(projection, line.line, line.rows - 1)?;
    }
    let mut result = BTreeMap::new();
    for &id in ids {
        let line = projection
            .lines()
            .partition_point(|line| line.visible_start <= id)
            .checked_sub(1)
            .ok_or(EditError::InvalidSelection)?;
        let rows = *counts.get(&line).ok_or(EditError::InvalidSelection)?;
        let within = id - projection.lines()[line].visible_start;
        if within >= rows {
            return Err(EditError::InvalidSelection.into());
        }
        let up = if within > 0 {
            Some(id - 1)
        } else if line == 0 {
            Some(id)
        } else {
            counts
                .get(&(line - 1))
                .map(|rows| visual_row_id(projection, line - 1, rows - 1))
                .transpose()?
        };
        let down = if within + 1 < rows {
            Some(id + 1)
        } else if line + 1 == projection.lines().len() {
            Some(id)
        } else {
            counts
                .get(&(line + 1))
                .map(|_| visual_row_id(projection, line + 1, 0))
                .transpose()?
        };
        result.insert(id, (up, down));
    }
    Ok(result)
}

/// Adapters measure these IDs for the current cursors. Missing neighboring line
/// geometry stays missing and cannot cause a jump across unmeasured text.
pub fn visual_neighbor_rows(
    projection: &FoldProjection,
    lines: &[VisualLineRows],
    current: &[usize],
) -> Result<BTreeSet<usize>, SelectionError> {
    if current.len() > MAX_VISUAL_CARETS {
        return Err(SelectionError::TooLarge);
    }
    let current = current.iter().copied().collect::<BTreeSet<_>>();
    let adjacent = neighbors(projection, lines, &current)?;
    let mut wanted = current;
    for (up, down) in adjacent.values() {
        wanted.extend([*up, *down].into_iter().flatten());
        if wanted.len() > MAX_VISUAL_CARETS {
            return Err(SelectionError::TooLarge);
        }
    }
    Ok(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        Document, FoldCommand, FoldRange, Indentation, Selection, SelectionMotion, VisualCaret,
        VisualLayout,
    };

    fn fixture() -> (Document, Vec<VisualLineRows>, Vec<VisualCaret>) {
        let doc = Document::new("文😀ab\r\nxy\r\nabcdef");
        let lines = vec![
            VisualLineRows { line: 0, rows: 2 },
            VisualLineRows { line: 1, rows: 1 },
            VisualLineRows { line: 2, rows: 2 },
        ];
        let mut carets = Vec::new();
        for (row, offsets) in [
            (0, vec![0, 3, 7]),
            (1, vec![7, 8, 9]),
            (11, vec![11, 12, 13]),
            (15, vec![15, 16, 17, 18]),
            (16, vec![18, 19, 20, 21]),
        ] {
            for (column, offset) in offsets.into_iter().enumerate() {
                carets.push(VisualCaret {
                    row,
                    offset,
                    column: i64::try_from(column * 64).unwrap(),
                });
            }
        }
        (doc, lines, carets)
    }

    #[test]
    fn neighborhood_motion_keeps_unicode_crlf_columns_without_global_heights() {
        let (mut doc, lines, carets) = fixture();
        let layout = VisualLayout::neighborhood(
            doc.text(),
            doc.projection(),
            "metrics".into(),
            &lines,
            carets,
        )
        .unwrap();
        doc.set_selections(vec![Selection::caret(8)]).unwrap();
        let source = doc.text().to_string();
        for (motion, expected) in [
            (SelectionMotion::Down, 12),
            (SelectionMotion::Down, 16),
            (SelectionMotion::Down, 19),
            (SelectionMotion::Up, 16),
            (SelectionMotion::Up, 12),
            (SelectionMotion::Up, 8),
        ] {
            doc.move_selections_with_layout(motion, false, Indentation::default(), &layout)
                .unwrap();
            assert_eq!(doc.selections()[0], Selection::caret(expected));
            assert_eq!(doc.text(), source);
            assert!(!doc.is_dirty());
        }
        // Retain the end-of-previous-row affinity at a shared soft-wrap offset.
        doc.set_selections(vec![Selection::caret(21)]).unwrap();
        doc.move_selections_with_layout(
            SelectionMotion::Up,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(18));
        doc.move_selections_with_layout(
            SelectionMotion::Up,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(13));
        doc.move_selections_with_layout(
            SelectionMotion::Down,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(18));
        doc.move_selections_with_layout(
            SelectionMotion::Down,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(21));
        assert_eq!(
            visual_neighbor_rows(&doc.projection(), &lines, &[1, 16]).unwrap(),
            BTreeSet::from([0, 1, 11, 15, 16])
        );
    }

    #[test]
    fn missing_neighborhood_rejects_all_cursors_atomically() {
        let (mut doc, lines, mut carets) = fixture();
        carets.retain(|caret| caret.row <= 1);
        let layout = VisualLayout::neighborhood(
            doc.text(),
            doc.projection(),
            "partial".into(),
            &lines[..1],
            carets,
        )
        .unwrap();
        doc.set_selections(vec![Selection::caret(3), Selection::caret(8)])
            .unwrap();
        let before = doc.clone();
        assert!(
            doc.move_selections_with_layout(
                SelectionMotion::Down,
                false,
                Indentation::default(),
                &layout
            )
            .is_err()
        );
        assert_eq!(doc, before);
        let projection = doc.projection();
        assert!(
            visual_neighbor_rows(&projection, &[VisualLineRows { line: 0, rows: 0 }], &[0])
                .is_err()
        );
        assert!(visual_neighbor_rows(&projection, &[lines[0], lines[0]], &[0]).is_err());
        assert!(
            visual_neighbor_rows(&projection, &[VisualLineRows { line: 0, rows: 100 }], &[0])
                .is_err()
        );
        assert!(visual_row_id(&projection, 3, 0).is_err());
        assert!(visual_row_id(&projection, 0, usize::MAX).is_err());
        let wrong = vec![VisualCaret {
            offset: 15,
            column: 0,
            row: 0,
        }];
        assert!(
            VisualLayout::neighborhood(doc.text(), projection, "bad".into(), &lines, wrong)
                .is_err()
        );
    }

    #[test]
    fn neighborhoods_skip_folded_source_and_reject_stale_projection() {
        let mut doc = Document::new("header\nhidden\ntail");
        doc.fold_state_mut().set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 1,
            }],
            3,
        );
        doc.fold_command(FoldCommand::Toggle(0));
        let projection = doc.projection();
        assert_eq!(
            projection
                .lines()
                .iter()
                .map(|line| line.source_line)
                .collect::<Vec<_>>(),
            [0, 2]
        );
        let tail = projection.lines()[1].visible_start;
        let geometry = [
            VisualLineRows { line: 0, rows: 1 },
            VisualLineRows { line: 1, rows: 1 },
        ];
        let carets = vec![
            VisualCaret {
                offset: 0,
                column: 0,
                row: 0,
            },
            VisualCaret {
                offset: tail,
                column: 0,
                row: tail,
            },
        ];
        let layout =
            VisualLayout::neighborhood(doc.text(), projection, "fold".into(), &geometry, carets)
                .unwrap();
        doc.move_selections_with_layout(
            SelectionMotion::Down,
            false,
            Indentation::default(),
            &layout,
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(14));
        doc.fold_command(FoldCommand::Toggle(0));
        let before = doc.clone();
        assert_eq!(
            doc.move_selections_with_layout(
                SelectionMotion::Up,
                false,
                Indentation::default(),
                &layout
            ),
            Err(EditError::StaleContext.into())
        );
        assert_eq!(doc, before);
    }
}
