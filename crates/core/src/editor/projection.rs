//! A folded edit view is a projection of the source, never a second document.
use std::ops::Range;

use super::{FoldState, Selection, lines::lines};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleLine {
    /// Zero-based logical source line, independent of folded view rows.
    pub source_line: usize,
    pub source: Range<usize>,
    pub visible_start: usize,
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
    text: String,
    source_len: usize,
    lines: Vec<VisibleLine>,
    hidden: Vec<HiddenText>,
}

impl FoldProjection {
    pub fn new(source: &str, folds: &FoldState) -> Self {
        let logical = lines(source);
        let mut text = String::new();
        let mut visible = Vec::new();
        let mut hidden: Vec<HiddenText> = Vec::new();
        let mut row = 0;
        while row < logical.len() {
            let line = &logical[row];
            visible.push(VisibleLine {
                source_line: row,
                source: line.start..line.end,
                visible_start: text.len(),
            });
            text.push_str(&source[line.start..line.end]);
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
            visible.push(VisibleLine {
                source_line: logical.len() - 1,
                source: source.len()..source.len(),
                visible_start: text.len(),
            });
        }
        Self {
            text,
            source_len: source.len(),
            lines: visible,
            hidden,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn lines(&self) -> &[VisibleLine] {
        &self.lines
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::FoldRange;

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
