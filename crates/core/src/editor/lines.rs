//! Logical-line commands, independent of the browser and workspace transport.
use super::{Document, Edit, EditError, LineEnding, Selection};
use std::{collections::BTreeSet, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCommand {
    MoveUp,
    MoveDown,
    Duplicate,
    DuplicateAbove,
    Delete,
    InsertAbove,
    InsertBelow,
}

#[derive(Clone, Debug)]
pub(super) struct Line {
    pub start: usize,
    pub body_end: usize,
    pub end: usize,
}

pub(super) fn lines(text: &str) -> Vec<Line> {
    let mut result = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let body_end = if line.ends_with("\r\n") {
            end - 2
        } else if line.ends_with('\n') {
            end - 1
        } else {
            end
        };
        result.push(Line {
            start,
            body_end,
            end,
        });
        start = end;
    }
    if text.is_empty() || text.ends_with('\n') {
        result.push(Line {
            start,
            body_end: start,
            end: start,
        });
    }
    result
}
pub(super) fn row_at(lines: &[Line], position: usize) -> usize {
    lines
        .partition_point(|line| line.start <= position)
        .saturating_sub(1)
}
pub(super) fn selected_rows(lines: &[Line], selections: &[Selection]) -> Vec<Range<usize>> {
    let mut selected = BTreeSet::new();
    for selection in selections {
        let range = selection.range();
        let first = row_at(lines, range.start);
        let mut last = row_at(lines, range.end);
        if !range.is_empty() && range.end == lines[last].start {
            last = last.saturating_sub(1);
        }
        selected.extend(first..=last);
    }
    let mut blocks: Vec<Range<usize>> = Vec::new();
    for row in selected {
        if let Some(block) = blocks.last_mut().filter(|block| block.end == row) {
            block.end += 1;
        } else {
            blocks.push(row..row + 1);
        }
    }
    blocks
}

impl Document {
    pub fn line_command(
        &mut self,
        command: LineCommand,
        ending: Option<LineEnding>,
    ) -> Result<bool, EditError> {
        let rows = lines(&self.text);
        let blocks = selected_rows(&rows, &self.selections);
        let ending = ending
            .unwrap_or_else(|| LineEnding::detect(&self.text))
            .text();
        match command {
            LineCommand::MoveUp | LineCommand::MoveDown => {
                let mut order: Vec<_> = (0..rows.len()).collect();
                for block in &blocks {
                    if command == LineCommand::MoveUp && block.start > 0 {
                        order[block.start - 1..block.end].rotate_left(1);
                    } else if command == LineCommand::MoveDown && block.end < rows.len() {
                        order[block.start..=block.end].rotate_right(1);
                    }
                }
                let mut text = String::new();
                let mut destinations = vec![
                    Line {
                        start: 0,
                        body_end: 0,
                        end: 0
                    };
                    rows.len()
                ];
                for (position, &original) in order.iter().enumerate() {
                    let start = text.len();
                    text.push_str(&self.text[rows[original].start..rows[original].body_end]);
                    let body_end = text.len();
                    // Separators belong to logical row positions, including the
                    // final row; moving an unterminated last line is safe.
                    text.push_str(&self.text[rows[position].body_end..rows[position].end]);
                    destinations[original] = Line {
                        start,
                        body_end,
                        end: text.len(),
                    };
                }
                let map = |position, previous| {
                    let mut row = row_at(&rows, position);
                    if previous && row > 0 && position == rows[row].start {
                        row -= 1;
                        return destinations[row].end;
                    }
                    destinations[row].start
                        + (position - rows[row].start).min(rows[row].body_end - rows[row].start)
                };
                let selections = self
                    .selections
                    .iter()
                    .map(|selection| {
                        let range = selection.range();
                        Selection {
                            anchor: map(
                                selection.anchor,
                                !range.is_empty() && selection.anchor == range.end,
                            ),
                            head: map(
                                selection.head,
                                !range.is_empty() && selection.head == range.end,
                            ),
                        }
                    })
                    .collect();
                let start = self
                    .text
                    .chars()
                    .zip(text.chars())
                    .take_while(|(left, right)| left == right)
                    .map(|(ch, _)| ch.len_utf8())
                    .sum::<usize>();
                let suffix = self.text[start..]
                    .chars()
                    .rev()
                    .zip(text[start..].chars().rev())
                    .take_while(|(left, right)| left == right)
                    .map(|(ch, _)| ch.len_utf8())
                    .sum::<usize>();
                self.apply(
                    vec![Edit::replace(
                        start..self.text.len() - suffix,
                        &text[start..text.len() - suffix],
                    )],
                    selections,
                    None,
                )
            }
            LineCommand::Delete => {
                let edits = blocks
                    .iter()
                    .map(|block| {
                        let start = if block.end == rows.len() && block.start > 0 {
                            rows[block.start - 1].body_end
                        } else {
                            rows[block.start].start
                        };
                        Edit::replace(start..rows[block.end - 1].end, "")
                    })
                    .collect();
                self.apply_mapped(edits)
            }
            LineCommand::Duplicate | LineCommand::DuplicateAbove => {
                let mut edits = Vec::new();
                let mut copies = Vec::new();
                let mut inserted = 0;
                for block in &blocks {
                    let start = rows[block.start].start;
                    let end = rows[block.end - 1].end;
                    let separator = if rows[block.end - 1].body_end == end {
                        ending
                    } else {
                        ""
                    };
                    let above = command == LineCommand::DuplicateAbove;
                    let text = if above {
                        format!("{}{separator}", &self.text[start..end])
                    } else {
                        format!("{separator}{}", &self.text[start..end])
                    };
                    let position = if above { start } else { end };
                    copies.push((
                        start..end,
                        position + inserted + if above { 0 } else { separator.len() },
                    ));
                    inserted += text.len();
                    edits.push(Edit::replace(position..position, text));
                }
                let selections = self
                    .selections
                    .iter()
                    .map(|selection| {
                        let range = selection.range();
                        let (source, destination) = copies
                            .iter()
                            .find(|(source, _)| {
                                source.start <= range.start && range.end <= source.end
                            })
                            .unwrap();
                        Selection {
                            anchor: destination + selection.anchor - source.start,
                            head: destination + selection.head - source.start,
                        }
                    })
                    .collect();
                self.apply(edits, selections, None)
            }
            LineCommand::InsertAbove | LineCommand::InsertBelow => {
                let heads: BTreeSet<_> = self
                    .selections
                    .iter()
                    .map(|selection| row_at(&rows, selection.head))
                    .collect();
                let changes = heads
                    .into_iter()
                    .map(|row| {
                        let line = &rows[row];
                        let prefix: String = self.text[line.start..line.body_end]
                            .chars()
                            .take_while(|ch| matches!(ch, ' ' | '\t'))
                            .collect();
                        if command == LineCommand::InsertAbove {
                            (
                                Edit::replace(line.start..line.start, format!("{prefix}{ending}")),
                                Selection::caret(prefix.len()),
                            )
                        } else if line.body_end < line.end {
                            (
                                Edit::replace(line.end..line.end, format!("{prefix}{ending}")),
                                Selection::caret(prefix.len()),
                            )
                        } else {
                            (
                                Edit::replace(line.end..line.end, format!("{ending}{prefix}")),
                                Selection::caret(ending.len() + prefix.len()),
                            )
                        }
                    })
                    .collect();
                self.apply_caret_edits(changes)
            }
        }
    }

    pub fn duplicate_selections(&mut self) -> Result<bool, EditError> {
        if self
            .selections
            .iter()
            .any(|selection| selection.range().is_empty())
        {
            return self.line_command(LineCommand::Duplicate, None);
        }
        let changes = self
            .selections
            .iter()
            .map(|selection| {
                let range = selection.range();
                let length = range.len();
                (
                    Edit::replace(range.end..range.end, &self.text[range]),
                    if selection.anchor <= selection.head {
                        Selection {
                            anchor: 0,
                            head: length,
                        }
                    } else {
                        Selection {
                            anchor: length,
                            head: 0,
                        }
                    },
                )
            })
            .collect();
        self.apply_caret_edits(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn move_duplicate_delete_preserve_unicode_selection_and_eof() {
        let mut doc = Document::new("a\r\n😀");
        doc.set_selections(vec![Selection { anchor: 7, head: 3 }])
            .unwrap();
        doc.line_command(LineCommand::MoveUp, None).unwrap();
        assert_eq!(doc.text(), "😀\r\na");
        assert_eq!(doc.selections(), &[Selection { anchor: 4, head: 0 }]);
        doc.undo();
        doc.line_command(LineCommand::Duplicate, None).unwrap();
        assert_eq!(doc.text(), "a\r\n😀\r\n😀");
        assert_eq!(
            doc.selections(),
            &[Selection {
                anchor: 13,
                head: 9
            }]
        );
        doc.undo();
        doc.line_command(LineCommand::Delete, None).unwrap();
        assert_eq!(doc.text(), "a");
        doc.undo();
        assert_eq!(doc.text(), "a\r\n😀");
    }
    #[test]
    fn line_boundaries_multiple_blocks_and_blank_lines_are_deliberate() {
        let mut doc = Document::new("a\nb\nc\nd\n");
        doc.set_selections(vec![Selection { anchor: 2, head: 4 }, Selection::caret(6)])
            .unwrap();
        doc.line_command(LineCommand::MoveUp, None).unwrap();
        assert_eq!(doc.text(), "b\na\nd\nc\n");
        assert_eq!(doc.selections()[0], Selection { anchor: 0, head: 2 });
        doc.undo();
        doc.set_selections(vec![Selection::caret(doc.text().len())])
            .unwrap();
        doc.line_command(LineCommand::Delete, None).unwrap();
        assert_eq!(doc.text(), "a\nb\nc\nd");
        let mut doc = Document::new("  x");
        doc.line_command(LineCommand::InsertBelow, Some(LineEnding::CrLf))
            .unwrap();
        assert_eq!(doc.text(), "  x\r\n  ");
        assert_eq!(doc.selections()[0].head, 7);
    }
    #[test]
    fn every_logical_line_round_trips_moves_and_duplicate_delete() {
        for original in ["a\n\n😀", "a\r\nb\r\n", "aaa\nb\naaa\nlast"] {
            let rows = lines(original);
            for row in &rows {
                let mut doc = Document::new(original);
                doc.set_selections(vec![Selection::caret(row.start)])
                    .unwrap();
                doc.line_command(LineCommand::Duplicate, None).unwrap();
                doc.line_command(LineCommand::Delete, None).unwrap();
                assert_eq!(doc.text(), original);
            }
            for row in rows.iter().skip(1) {
                let mut doc = Document::new(original);
                doc.set_selections(vec![Selection::caret(row.start)])
                    .unwrap();
                doc.line_command(LineCommand::MoveUp, None).unwrap();
                doc.line_command(LineCommand::MoveDown, None).unwrap();
                assert_eq!(doc.text(), original);
                assert_eq!(doc.selections(), &[Selection::caret(row.start)]);
            }
        }
    }

    #[test]
    fn snippet_duplication_is_one_undoable_transaction() {
        let mut doc = Document::new("a😀z");
        doc.set_selections(vec![Selection { anchor: 5, head: 1 }])
            .unwrap();
        doc.duplicate_selections().unwrap();
        assert_eq!(doc.text(), "a😀😀z");
        assert_eq!(doc.selections(), &[Selection { anchor: 9, head: 5 }]);
        doc.undo();
        assert_eq!(doc.text(), "a😀z");
    }
}
