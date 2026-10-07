//! Selection movement is source policy; the browser only maps keys and paints.
use super::visual_motion::{MotionColumns, VisualGoal, VisualLayout};
use super::{
    Document, EditError, Indentation, Selection, SelectionError,
    lines::{lines, row_at},
    selections::{byte_at_column, display_column},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMotion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
}

fn previous(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .take_while(|(offset, _)| *offset < at)
        .last()
        .map_or(0, |(offset, _)| offset)
}
fn next(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .find(|(offset, _)| *offset > at)
        .map_or(text.len(), |(offset, _)| offset)
}
pub(super) fn category(grapheme: &str) -> u8 {
    if grapheme.chars().all(char::is_whitespace) {
        0
    } else if grapheme
        .chars()
        .any(|character| character.is_alphanumeric() || character == '_')
    {
        1
    } else {
        2
    }
}
fn word_left(text: &str, at: usize) -> usize {
    let mut result = at;
    let mut kind = None;
    for (offset, grapheme) in text[..at].grapheme_indices(true).rev() {
        let current = category(grapheme);
        if kind.is_some_and(|kind| kind != current) {
            break;
        }
        result = offset;
        if current != 0 {
            kind = Some(current);
        }
    }
    result
}
fn word_right(text: &str, at: usize) -> usize {
    let mut result = at;
    let mut kind = None;
    for (offset, grapheme) in text[at..].grapheme_indices(true) {
        let current = category(grapheme);
        if kind.is_some_and(|kind| kind != current) && current != 0 {
            break;
        }
        result = at + offset + grapheme.len();
        if current == 0 {
            kind = Some(0);
        } else {
            kind = Some(current);
        }
    }
    result
}

impl Document {
    /// Affinity is transient view state, validated against current measurements.
    pub fn visual_caret(&self, index: usize, identity: &str) -> Option<super::VisualCaret> {
        let MotionColumns::Visual {
            identity: current,
            goals,
        } = self.motion_columns.as_ref()?
        else {
            return None;
        };
        if current != identity {
            return None;
        }
        let goal = goals.get(index)?;
        (self.selections.get(index)?.head == goal.source_head).then_some(goal.caret)
    }

    pub fn move_selections(
        &mut self,
        motion: SelectionMotion,
        extend: bool,
        indentation: Indentation,
    ) -> Result<bool, SelectionError> {
        self.move_selections_in(motion, extend, indentation, None)
    }

    pub fn move_selections_with_layout(
        &mut self,
        motion: SelectionMotion,
        extend: bool,
        indentation: Indentation,
        layout: &VisualLayout,
    ) -> Result<bool, SelectionError> {
        if layout.source != self.text || layout.projection != self.projection() {
            return Err(EditError::StaleContext.into());
        }
        self.move_selections_in(motion, extend, indentation, Some(layout))
    }

    fn move_selections_in(
        &mut self,
        motion: SelectionMotion,
        extend: bool,
        indentation: Indentation,
        layout: Option<&VisualLayout>,
    ) -> Result<bool, SelectionError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive.into());
        }
        if self.text.len() > super::MAX_STRUCTURE_BYTES {
            return Err(SelectionError::TooLarge);
        }
        let projection = self.projection();
        let text = projection.text();
        let rows = lines(text);
        let vertical = matches!(motion, SelectionMotion::Up | SelectionMotion::Down);
        let mut columns = Vec::with_capacity(self.selections.len());
        let mut selections = Vec::with_capacity(self.selections.len());
        let mut visual_goals: Vec<VisualGoal> = Vec::with_capacity(self.selections.len());
        for (index, selection) in self.selections.iter().enumerate() {
            let visible = projection
                .visible_selection(*selection)
                .map_err(|_| EditError::InvalidSelection)?;
            let at = visible.head;
            let row = row_at(&rows, at);
            let line = &rows[row];
            let column = self
                .motion_columns
                .as_ref()
                .and_then(|columns| match columns {
                    MotionColumns::Logical(columns)
                        if vertical && columns.len() == self.selections.len() =>
                    {
                        Some(columns)
                    }
                    _ => None,
                })
                .map_or_else(
                    || {
                        display_column(
                            &text[line.start..at.min(line.body_end)],
                            indentation.tab_width(),
                        )
                    },
                    |columns| columns[index],
                );
            columns.push(column);
            let head = if !extend
                && visible.anchor != visible.head
                && matches!(motion, SelectionMotion::Left | SelectionMotion::WordLeft)
            {
                visible.range().start
            } else if !extend
                && visible.anchor != visible.head
                && matches!(motion, SelectionMotion::Right | SelectionMotion::WordRight)
            {
                visible.range().end
            } else {
                match motion {
                    SelectionMotion::Left => previous(text, at),
                    SelectionMotion::Right => next(text, at),
                    SelectionMotion::WordLeft => word_left(text, at),
                    SelectionMotion::WordRight => word_right(text, at),
                    SelectionMotion::LineStart => line.start,
                    SelectionMotion::LineEnd => line.body_end,
                    SelectionMotion::DocumentStart => 0,
                    SelectionMotion::DocumentEnd => text.len(),
                    SelectionMotion::Up | SelectionMotion::Down if layout.is_some() => {
                        let layout = layout.unwrap();
                        let previous =
                            self.motion_columns
                                .as_ref()
                                .and_then(|columns| match columns {
                                    MotionColumns::Visual { identity, goals }
                                        if identity == &layout.identity
                                            && goals.len() == self.selections.len() =>
                                    {
                                        goals.get(index)
                                    }
                                    _ => None,
                                });
                        let (target, goal) =
                            layout.target(at, motion == SelectionMotion::Down, previous)?;
                        visual_goals.push(goal);
                        target
                    }
                    SelectionMotion::Up | SelectionMotion::Down => {
                        let target = if motion == SelectionMotion::Up {
                            row.saturating_sub(1)
                        } else {
                            (row + 1).min(rows.len() - 1)
                        };
                        let line = &rows[target];
                        line.start
                            + byte_at_column(
                                &text[line.start..line.body_end],
                                column,
                                indentation.tab_width(),
                                false,
                            )
                    }
                }
            };
            let head = projection
                .source_offset(head)
                .map_err(|_| EditError::InvalidSelection)?;
            if layout.is_some()
                && vertical
                && let Some(goal) = visual_goals.last_mut()
            {
                goal.source_head = head;
            }
            selections.push(Selection {
                anchor: if extend { selection.anchor } else { head },
                head,
            });
        }
        let before = self.selections.clone();
        self.set_selections(selections)?;
        self.motion_columns = if vertical && self.selections.len() == columns.len() {
            Some(if let Some(layout) = layout {
                MotionColumns::Visual {
                    identity: layout.identity.clone(),
                    goals: visual_goals,
                }
            } else {
                MotionColumns::Logical(columns)
            })
        } else {
            None
        };
        Ok(self.selections != before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_motion_keeps_selections_history_and_dirty_state_unchanged() {
        let mut doc = Document::new("x".repeat(super::super::MAX_STRUCTURE_BYTES + 1));
        doc.set_selections(vec![Selection::caret(0), Selection::caret(1)])
            .unwrap();
        let before = doc.clone();
        assert_eq!(
            doc.move_selections(SelectionMotion::Right, false, Indentation::default()),
            Err(SelectionError::TooLarge)
        );
        assert_eq!(doc, before);
        doc.selection_command(
            super::super::SelectionCommand::Single,
            crate::highlight::Language::Plain,
            Indentation::default(),
        )
        .unwrap();
        assert_eq!(doc.selections(), &[Selection::caret(0)]);
    }
    #[test]
    fn grapheme_motion_preserves_all_cursors_and_crlf_and_does_not_edit_history() {
        let mut doc = Document::new("a\u{301}😀\r\na\u{301}😀");
        doc.set_selections(vec![Selection::caret(7), Selection::caret(16)])
            .unwrap();
        doc.move_selections(SelectionMotion::Left, false, Indentation::default())
            .unwrap();
        assert_eq!(
            doc.selections(),
            &[Selection::caret(3), Selection::caret(12)]
        );
        doc.move_selections(SelectionMotion::Left, true, Indentation::default())
            .unwrap();
        assert_eq!(doc.selected_text(), "a\u{301}\na\u{301}");
        doc.move_selections(SelectionMotion::Right, false, Indentation::default())
            .unwrap();
        assert_eq!(
            doc.selections(),
            &[Selection::caret(3), Selection::caret(12)]
        );
        assert!(!doc.can_undo());
        assert!(!doc.is_dirty());
        doc.move_selections(SelectionMotion::Right, false, Indentation::default())
            .unwrap();
        doc.move_selections(SelectionMotion::Right, false, Indentation::default())
            .unwrap();
        assert_eq!(
            doc.selections(),
            &[Selection::caret(9), Selection::caret(16)]
        );
    }
    #[test]
    fn vertical_motion_retains_tab_columns_across_short_rows_and_resets_after_horizontal_motion() {
        let mut doc = Document::new("\tx\nq\n\tx\nq\n\tx");
        doc.set_selections(vec![Selection::caret(2), Selection::caret(7)])
            .unwrap();
        doc.move_selections(SelectionMotion::Down, false, Indentation::default())
            .unwrap();
        assert_eq!(
            doc.selections(),
            &[Selection::caret(4), Selection::caret(9)]
        );
        doc.move_selections(SelectionMotion::Down, false, Indentation::default())
            .unwrap();
        assert_eq!(
            doc.selections(),
            &[Selection::caret(7), Selection::caret(12)]
        );
        doc.move_selections(SelectionMotion::Left, false, Indentation::default())
            .unwrap();
        doc.move_selections(SelectionMotion::Up, false, Indentation::default())
            .unwrap();
        doc.move_selections(SelectionMotion::Up, false, Indentation::default())
            .unwrap();
        assert_eq!(
            doc.selections(),
            &[Selection::caret(1), Selection::caret(6)]
        );
    }
    #[test]
    fn word_and_boundary_motion_preserve_direction_and_merge_colliding_cursors() {
        let mut doc = Document::new("文_word  +  next\r\nlast");
        doc.set_selections(vec![Selection::caret(10), Selection::caret(17)])
            .unwrap();
        doc.move_selections(SelectionMotion::WordLeft, true, Indentation::default())
            .unwrap();
        assert_eq!(doc.selected_text(), "文_word  \nnext");
        doc.move_selections(
            SelectionMotion::DocumentStart,
            false,
            Indentation::default(),
        )
        .unwrap();
        assert_eq!(doc.selections(), &[Selection::caret(0)]);
        doc.move_selections(SelectionMotion::WordRight, false, Indentation::default())
            .unwrap();
        assert_eq!(doc.selections(), &[Selection::caret(10)]);
        doc.move_selections(SelectionMotion::LineEnd, true, Indentation::default())
            .unwrap();
        assert_eq!(doc.selections()[0].head, 17);
    }
    #[test]
    fn folded_motion_skips_hidden_source_and_composition_failure_is_atomic() {
        let mut doc = Document::new("head\nhidden\nend\ntail");
        doc.fold_state_mut().set_ranges(
            vec![super::super::FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        doc.fold_command(super::super::FoldCommand::CollapseAll);
        doc.set_selections(vec![Selection::caret(4)]).unwrap();
        doc.move_selections(SelectionMotion::Right, false, Indentation::default())
            .unwrap();
        assert_eq!(doc.selections(), &[Selection::caret(16)]);
        assert!(doc.fold_state().collapsed_at(0).is_some());
        doc.move_selections(SelectionMotion::Up, false, Indentation::default())
            .unwrap();
        assert_eq!(doc.selections(), &[Selection::caret(0)]);
        doc.begin_composition(None);
        let before = doc.clone();
        assert_eq!(
            doc.move_selections(SelectionMotion::Down, false, Indentation::default()),
            Err(SelectionError::Edit(EditError::CompositionActive))
        );
        assert_eq!(doc, before);
    }
}
