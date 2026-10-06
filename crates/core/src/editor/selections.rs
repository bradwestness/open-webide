//! Source selection policy. Pixel measurement and native input remain adapters.
use super::{
    Document, EditError, Indentation, SearchError, SearchOptions, SearchPattern, Selection,
    Structure,
    lines::{lines, row_at},
};
use crate::highlight::Language;
use regex::Regex;
use std::{ops::Range, sync::LazyLock};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const MAX_SELECTIONS: usize = 512;
const MAX_EXPANSIONS: usize = 64;
static WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\w+").expect("constant word pattern"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionCommand {
    NextOccurrence,
    AllOccurrences,
    AddAbove,
    AddBelow,
    Expand,
    Shrink,
    Single,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError {
    Edit(EditError),
    Search(SearchError),
    TooLarge,
}
impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edit(error) => error.fmt(f),
            Self::Search(error) => error.fmt(f),
            Self::TooLarge => f.write_str("Selection commands are limited to files up to 2 MiB"),
        }
    }
}
impl std::error::Error for SelectionError {}
impl From<EditError> for SelectionError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}
impl From<SearchError> for SelectionError {
    fn from(error: SearchError) -> Self {
        Self::Search(error)
    }
}

/// Merge true overlaps and duplicate starting points, retaining the earliest
/// input selection's direction and priority. Adjacent nonempty occurrences stay
/// separate; a caret at the end of a preceding range is also a distinct edit.
pub fn normalize_selections(
    text: &str,
    selections: Vec<Selection>,
) -> Result<Vec<Selection>, EditError> {
    super::validate_selections(text, &selections)?;
    if selections.len() > MAX_SELECTIONS {
        return Err(EditError::TooManySelections);
    }
    let mut sorted: Vec<_> = selections.into_iter().enumerate().collect();
    sorted.sort_by_key(|(_, selection)| (selection.range().start, selection.range().end));
    let mut result: Vec<(usize, Selection)> = Vec::new();
    for (priority, selection) in sorted {
        let range = selection.range();
        if let Some((first, previous)) = result.last_mut() {
            let before = previous.range();
            if range.start < before.end || range.start == before.start {
                let backwards = if priority < *first {
                    selection.anchor > selection.head
                } else {
                    previous.anchor > previous.head
                };
                *first = (*first).min(priority);
                let start = before.start.min(range.start);
                let end = before.end.max(range.end);
                *previous = if backwards {
                    Selection {
                        anchor: end,
                        head: start,
                    }
                } else {
                    Selection {
                        anchor: start,
                        head: end,
                    }
                };
                continue;
            }
        }
        result.push((priority, selection));
    }
    result.sort_by_key(|(priority, _)| *priority);
    Ok(result.into_iter().map(|(_, selection)| selection).collect())
}
fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    WORDS
        .find_iter(text)
        .find(|word| word.start() <= offset && offset <= word.end())
        .map(|word| word.range())
}
pub(super) fn display_column(text: &str, tab_width: usize) -> usize {
    text.graphemes(true).fold(0, |column, grapheme| {
        column
            + if grapheme == "\t" {
                tab_width - column % tab_width
            } else {
                grapheme.width()
            }
    })
}
pub(super) fn byte_at_column(text: &str, target: usize, tab_width: usize, end: bool) -> usize {
    let mut column = 0;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let width = if grapheme == "\t" {
            tab_width - column % tab_width
        } else {
            grapheme.width()
        };
        if target == column {
            return offset;
        }
        if target < column + width {
            return offset + if end { grapheme.len() } else { 0 };
        }
        column += width;
    }
    text.len()
}
/// Rectangles use display columns, tab stops and whole grapheme clusters. Short
/// rows clamp at their body end, never into CRLF separators.
pub fn column_selections(
    text: &str,
    anchor: usize,
    head: usize,
    indentation: Indentation,
) -> Result<Vec<Selection>, SelectionError> {
    super::validate_selections(text, &[Selection { anchor, head }])?;
    if text.len() > super::MAX_STRUCTURE_BYTES {
        return Err(SelectionError::TooLarge);
    }
    let rows = lines(text);
    let anchor_row = row_at(&rows, anchor);
    let head_row = row_at(&rows, head);
    let first = anchor_row.min(head_row);
    let last = anchor_row.max(head_row);
    if last - first + 1 > MAX_SELECTIONS {
        return Err(EditError::TooManySelections.into());
    }
    let column = |row: usize, at: usize| {
        display_column(
            &text[rows[row].start..at.min(rows[row].body_end)],
            indentation.tab_width(),
        )
    };
    let from = column(anchor_row, anchor);
    let to = column(head_row, head);
    let mut result = Vec::new();
    for (row, line) in rows.iter().enumerate().take(last + 1).skip(first) {
        let body = &text[line.start..line.body_end];
        let start = line.start + byte_at_column(body, from.min(to), indentation.tab_width(), false);
        let end =
            line.start + byte_at_column(body, from.max(to), indentation.tab_width(), from != to);
        let selection = if from > to {
            Selection {
                anchor: end,
                head: start,
            }
        } else {
            Selection {
                anchor: start,
                head: end,
            }
        };
        if row == head_row {
            result.insert(0, selection);
        } else {
            result.push(selection);
        }
    }
    Ok(normalize_selections(text, result)?)
}
impl Document {
    /// Replace every selection in one transaction. Check aggregate size before
    /// copying clipboard/typed text for each cursor.
    pub fn replace_selections(
        &mut self,
        text: &str,
        group: Option<u64>,
    ) -> Result<bool, EditError> {
        let remaining = self
            .selections
            .iter()
            .fold(self.text.len(), |size, selection| {
                size - selection.range().len()
            });
        let output = text
            .len()
            .checked_mul(self.selections.len())
            .and_then(|inserted| remaining.checked_add(inserted))
            .ok_or(EditError::OutputTooLarge)?;
        if output > super::MAX_DOCUMENT_BYTES {
            return Err(EditError::OutputTooLarge);
        }
        self.apply_grouped_caret_edits(
            self.selections
                .iter()
                .map(|selection| {
                    (
                        super::Edit::replace(selection.range(), text),
                        Selection::caret(text.len()),
                    )
                })
                .collect(),
            group,
        )
    }

    /// Native clipboard adapter reads full source selections, even across folds.
    /// Input order is retained so the primary selection is copied first.
    pub fn selected_text(&self) -> String {
        self.selections
            .iter()
            .filter(|selection| selection.anchor != selection.head)
            .map(|selection| &self.text[selection.range()])
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Distribute one clipboard line per selection when their counts match.
    /// Otherwise paste the complete text into every selection. Whitespace and
    /// source line endings outside the replaced ranges stay untouched.
    pub fn paste_selections(&mut self, text: &str) -> Result<bool, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        if text.len() > super::MAX_DOCUMENT_BYTES {
            return Err(EditError::OutputTooLarge);
        }
        if self.selections.len() == 1 {
            return self.replace_selections(text, None);
        }
        let mut fragments = text
            .split('\n')
            .take(self.selections.len() + 1)
            .collect::<Vec<_>>();
        if fragments.len() != self.selections.len() {
            return self.replace_selections(text, None);
        }
        // CR belongs to a CRLF delimiter, not to the distributed line's body.
        let last = fragments.len() - 1;
        for fragment in &mut fragments[..last] {
            *fragment = fragment.strip_suffix('\r').unwrap_or(fragment);
        }
        self.replace_fragments(&fragments)
    }

    pub fn toggle_cursor(&mut self, offset: usize) -> Result<bool, EditError> {
        super::validate_selections(&self.text, &[Selection::caret(offset)])?;
        let mut selections = self.selections.clone();
        if let Some(index) = selections
            .iter()
            .position(|selection| *selection == Selection::caret(offset))
        {
            if selections.len() == 1 {
                return Ok(false);
            }
            selections.remove(index);
        } else {
            selections.insert(0, Selection::caret(offset));
        }
        let selections = normalize_selections(&self.text, selections)?;
        let changed = selections != self.selections;
        self.set_selections(selections)?;
        self.reveal_selection();
        Ok(changed)
    }
    pub fn select_columns(
        &mut self,
        anchor: usize,
        head: usize,
        indentation: Indentation,
    ) -> Result<bool, SelectionError> {
        let selections = column_selections(&self.text, anchor, head, indentation)?;
        let changed = selections != self.selections;
        self.set_selections(selections)?;
        self.reveal_selection();
        Ok(changed)
    }
    pub fn selection_command(
        &mut self,
        command: SelectionCommand,
        language: Language,
        indentation: Indentation,
    ) -> Result<bool, SelectionError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive.into());
        }
        if command == SelectionCommand::Single {
            let primary = self.selections[0];
            let changed = self.selections.len() > 1;
            self.set_selections(vec![primary])?;
            return Ok(changed);
        }
        if command == SelectionCommand::Shrink {
            let Some(before) = self.selection_history.pop() else {
                return Ok(false);
            };
            self.selections = before;
            self.motion_columns = None;
            self.reveal_selection();
            return Ok(true);
        }
        if self.text.len() > super::MAX_STRUCTURE_BYTES {
            return Err(SelectionError::TooLarge);
        }
        let primary = self.selections[0];
        let mut selections = self.selections.clone();
        match command {
            SelectionCommand::NextOccurrence | SelectionCommand::AllOccurrences => {
                let range = primary.range();
                let range = if range.is_empty() {
                    let Some(word) = word_at(&self.text, primary.head) else {
                        return Ok(false);
                    };
                    if command == SelectionCommand::NextOccurrence {
                        self.set_selections(vec![Selection {
                            anchor: word.start,
                            head: word.end,
                        }])?;
                        self.reveal_selection();
                        return Ok(true);
                    }
                    word
                } else {
                    range
                };
                let pattern =
                    SearchPattern::new(&self.text[range.clone()], SearchOptions::default())?;
                let matches = pattern.find(&self.text, None)?;
                if command == SelectionCommand::AllOccurrences {
                    if matches.len() > MAX_SELECTIONS {
                        return Err(EditError::TooManySelections.into());
                    }
                    selections = matches
                        .into_iter()
                        .map(|found| Selection {
                            anchor: found.range.start,
                            head: found.range.end,
                        })
                        .collect();
                    if let Some(index) = selections
                        .iter()
                        .position(|selection| selection.range() == range)
                    {
                        let selected = selections.remove(index);
                        selections.insert(
                            0,
                            if primary.range() == range {
                                primary
                            } else {
                                selected
                            },
                        );
                    }
                } else {
                    let remaining = |found: &&super::SearchMatch| {
                        !selections
                            .iter()
                            .any(|selection| selection.range() == found.range)
                    };
                    let next = matches
                        .iter()
                        .filter(remaining)
                        .find(|found| found.range.start >= range.end)
                        .or_else(|| matches.iter().find(remaining));
                    let Some(found) = next else {
                        return Ok(false);
                    };
                    selections.insert(
                        0,
                        Selection {
                            anchor: found.range.start,
                            head: found.range.end,
                        },
                    );
                }
            }
            SelectionCommand::AddAbove | SelectionCommand::AddBelow => {
                let rows = lines(&self.text);
                let row = row_at(&rows, primary.head);
                let next = if command == SelectionCommand::AddAbove {
                    row.checked_sub(1)
                } else {
                    (row + 1 < rows.len()).then_some(row + 1)
                };
                let Some(next) = next else {
                    return Ok(false);
                };
                let column = display_column(
                    &self.text[rows[row].start..primary.head.min(rows[row].body_end)],
                    indentation.tab_width(),
                );
                let offset = rows[next].start
                    + byte_at_column(
                        &self.text[rows[next].start..rows[next].body_end],
                        column,
                        indentation.tab_width(),
                        false,
                    );
                if selections
                    .iter()
                    .any(|selection| *selection == Selection::caret(offset))
                {
                    return Ok(false);
                }
                selections.insert(0, Selection::caret(offset));
            }
            SelectionCommand::Expand => {
                let structure = Structure::new(&self.text, language);
                let rows = lines(&self.text);
                let mut structure_ranges: Vec<_> = structure.literals().cloned().collect();
                for (open, _, mate) in &structure.brackets {
                    if let Some(close) = mate.filter(|close| open < close) {
                        structure_ranges.push(open + 1..close);
                        structure_ranges.push(*open..close + 1);
                    }
                }
                let words: Vec<_> = WORDS
                    .find_iter(&self.text)
                    .map(|word| word.range())
                    .collect();
                selections = selections
                    .iter()
                    .map(|selection| {
                        let range = selection.range();
                        let index = words.partition_point(|word| word.end < range.start);
                        let word = words
                            .get(index)
                            .filter(|word| word.start <= range.start && range.start <= word.end)
                            .cloned();
                        let first = row_at(&rows, range.start);
                        let last = row_at(&rows, range.end);
                        let expanded = word
                            .into_iter()
                            .chain(structure_ranges.iter().cloned())
                            .chain([
                                rows[first].start..rows[last].body_end,
                                rows[first].start..rows[last].end,
                                0..self.text.len(),
                            ])
                            .filter(|candidate| {
                                candidate.start <= range.start
                                    && range.end <= candidate.end
                                    && *candidate != range
                            })
                            .min_by_key(Range::len)
                            .unwrap_or(range);
                        if selection.anchor > selection.head {
                            Selection {
                                anchor: expanded.end,
                                head: expanded.start,
                            }
                        } else {
                            Selection {
                                anchor: expanded.start,
                                head: expanded.end,
                            }
                        }
                    })
                    .collect();
            }
            SelectionCommand::Single | SelectionCommand::Shrink => unreachable!(),
        }
        let selections = normalize_selections(&self.text, selections)?;
        if selections == self.selections {
            return Ok(false);
        }
        if command == SelectionCommand::Expand {
            if self.selection_history.len() == MAX_EXPANSIONS {
                self.selection_history.remove(0);
            }
            self.selection_history.push(self.selections.clone());
            self.selections = selections;
            self.motion_columns = None;
        } else {
            self.set_selections(selections)?;
        }
        self.reveal_selection();
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_lines_follow_primary_order_and_undo_with_selections() {
        let mut doc = Document::new("foo\r\nfoo");
        let selections = vec![
            Selection { anchor: 8, head: 5 },
            Selection { anchor: 0, head: 3 },
        ];
        doc.set_selections(selections.clone()).unwrap();
        doc.paste_selections("文\r\n😀").unwrap();
        assert_eq!(doc.text(), "😀\r\n文");
        assert_eq!(
            doc.selections(),
            &[Selection::caret(9), Selection::caret(4)]
        );
        doc.undo();
        assert_eq!(doc.text(), "foo\r\nfoo");
        assert_eq!(doc.selections(), selections);
        doc.paste_selections("a\nb\nc").unwrap();
        assert_eq!(doc.text(), "a\nb\nc\r\na\nb\nc");
    }
    #[test]
    fn collapsing_folds_merges_hidden_cursors_before_the_next_edit() {
        let mut doc = Document::new("header\nfirst\nsecond\nend");
        doc.fold_state_mut().set_ranges(
            vec![super::super::FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        doc.set_selections(vec![Selection::caret(9), Selection::caret(17)])
            .unwrap();
        doc.fold_command(super::super::FoldCommand::CollapseAll);
        assert_eq!(doc.selections().len(), 1);
        let before = doc.text().to_string();
        doc.replace_selections("x", None).unwrap();
        assert!(doc.undo());
        assert_eq!(doc.text(), before);
    }
    #[test]
    fn replacements_preserve_primary_order_grouped_history_and_unicode_separators() {
        let mut doc = Document::new("文 foo\r\nfoo");
        let selections = vec![
            Selection {
                anchor: 12,
                head: 9,
            },
            Selection { anchor: 4, head: 7 },
        ];
        doc.set_selections(selections.clone()).unwrap();
        assert_eq!(doc.selected_text(), "foo\nfoo");
        doc.replace_selections("😀", Some(7)).unwrap();
        assert_eq!(doc.text(), "文 😀\r\n😀");
        assert_eq!(
            doc.selections(),
            &[Selection::caret(14), Selection::caret(8)]
        );
        doc.replace_selections("!", Some(7)).unwrap();
        assert_eq!(doc.text(), "文 😀!\r\n😀!");
        assert!(doc.undo());
        assert_eq!(doc.text(), "文 foo\r\nfoo");
        assert_eq!(doc.selections(), selections);
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert_eq!(doc.text(), "文 😀!\r\n😀!");
    }

    #[test]
    fn adjacent_replacements_normalize_resulting_cursors_and_undo_atomically() {
        let mut doc = Document::new("foofoo");
        let selections = vec![
            Selection { anchor: 6, head: 3 },
            Selection { anchor: 0, head: 3 },
        ];
        doc.set_selections(selections.clone()).unwrap();
        doc.replace_selections("", None).unwrap();
        assert_eq!(doc.text(), "");
        assert_eq!(doc.selections(), &[Selection::caret(0)]);
        doc.undo();
        assert_eq!(doc.text(), "foofoo");
        assert_eq!(doc.selections(), selections);
        doc.redo();
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn aggregate_replacement_limit_preserves_redo_selections_and_revision() {
        let mut doc = Document::new("ab");
        doc.replace_selections("x", None).unwrap();
        doc.undo();
        doc.set_selections(vec![Selection::caret(0), Selection::caret(2)])
            .unwrap();
        let before = doc.clone();
        let clipboard = "x".repeat(super::super::MAX_DOCUMENT_BYTES / 2);
        assert_eq!(
            doc.replace_selections(&clipboard, None),
            Err(EditError::OutputTooLarge)
        );
        assert_eq!(doc, before);
        assert!(doc.can_redo());
        assert_eq!(
            doc.set_selections(vec![Selection::caret(0); MAX_SELECTIONS + 1]),
            Err(EditError::TooManySelections)
        );
        assert_eq!(doc, before);
    }

    proptest::proptest! {
        #[test]
        fn normalization_is_idempotent_and_never_leaves_overlaps(
            ranges in proptest::collection::vec((0usize..65, 0usize..65), 1..100)
        ) {
            let text = "x".repeat(64);
            let selections = ranges.into_iter().map(|(anchor, head)| Selection { anchor, head }).collect();
            let normalized = normalize_selections(&text, selections).unwrap();
            proptest::prop_assert_eq!(normalize_selections(&text, normalized.clone()).unwrap(), normalized.clone());
            let mut ranges: Vec<_> = normalized.iter().map(|selection| selection.range()).collect();
            ranges.sort_by_key(|range| (range.start, range.end));
            for pair in ranges.windows(2) {
                proptest::prop_assert!(pair[0].end <= pair[1].start);
                proptest::prop_assert!(pair[0].start != pair[1].start);
            }
        }
    }
    #[test]
    fn normalization_merges_overlap_without_merging_adjacent_occurrences() {
        let backwards = Selection { anchor: 6, head: 2 };
        assert_eq!(
            normalize_selections(
                "abcdef",
                vec![
                    backwards,
                    Selection { anchor: 0, head: 4 },
                    Selection::caret(3)
                ]
            )
            .unwrap(),
            vec![Selection { anchor: 6, head: 0 }]
        );
        assert_eq!(
            normalize_selections(
                "foofoo",
                vec![
                    Selection { anchor: 3, head: 6 },
                    Selection { anchor: 0, head: 3 }
                ]
            )
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            normalize_selections(
                "foo",
                vec![Selection::caret(3), Selection { anchor: 0, head: 3 }]
            )
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            normalize_selections("文", vec![Selection::caret(1)]),
            Err(EditError::InvalidSelection)
        );
    }
    #[test]
    fn occurrence_commands_wrap_preserve_primary_and_leave_text_history_clean() {
        let mut doc = Document::new("文 foo foo\r\nfoo");
        doc.set_selections(vec![Selection::caret(6)]).unwrap();
        assert!(
            doc.selection_command(
                SelectionCommand::NextOccurrence,
                Language::Rust,
                Indentation::default()
            )
            .unwrap()
        );
        assert_eq!(&doc.text()[doc.selections()[0].range()], "foo");
        assert!(
            doc.selection_command(
                SelectionCommand::NextOccurrence,
                Language::Rust,
                Indentation::default()
            )
            .unwrap()
        );
        assert_eq!(doc.selections()[0].range(), 8..11);
        assert!(
            doc.selection_command(
                SelectionCommand::NextOccurrence,
                Language::Rust,
                Indentation::default()
            )
            .unwrap()
        );
        assert_eq!(doc.selections()[0].range(), 13..16);
        assert!(
            !doc.selection_command(
                SelectionCommand::NextOccurrence,
                Language::Rust,
                Indentation::default()
            )
            .unwrap()
        );
        assert!(!doc.is_dirty());
        assert!(!doc.can_undo());
        doc.selection_command(
            SelectionCommand::Single,
            Language::Rust,
            Indentation::default(),
        )
        .unwrap();
        doc.selection_command(
            SelectionCommand::AllOccurrences,
            Language::Rust,
            Indentation::default(),
        )
        .unwrap();
        assert_eq!(doc.selections().len(), 3);
        assert_eq!(doc.selections()[0].range(), 13..16);
    }
    #[test]
    fn columns_include_whole_wide_graphemes_tabs_and_clamp_short_crlf_rows() {
        let text = "  ab\r\n\t文😀x\r\nz\r\n  cd";
        let selections = column_selections(text, 2, text.len(), Indentation::default()).unwrap();
        assert_eq!(&text[selections[0].range()], "cd");
        assert_eq!(&text[selections[1].range()], "ab");
        assert_eq!(&text[selections[2].range()], "\t");
        assert!(selections[3].range().is_empty());
        for selection in selections {
            assert!(text.is_char_boundary(selection.anchor));
            assert!(text.is_char_boundary(selection.head));
        }
        let text = "a\u{301}😀\n文x";
        let selections = column_selections(text, 3, text.len(), Indentation::default()).unwrap();
        assert_eq!(&text[selections[1].range()], "😀");
        assert_eq!(&text[selections[0].range()], "文x");
    }
    #[test]
    fn expansion_shrinks_directionally_and_edit_invalidates_the_stack() {
        let mut doc = Document::new("call(foo)");
        doc.set_selections(vec![Selection { anchor: 7, head: 6 }])
            .unwrap();
        for expected in ["foo", "(foo)", "call(foo)"] {
            assert!(
                doc.selection_command(
                    SelectionCommand::Expand,
                    Language::Rust,
                    Indentation::default()
                )
                .unwrap()
            );
            assert_eq!(&doc.text()[doc.selections()[0].range()], expected);
            assert!(doc.selections()[0].anchor > doc.selections()[0].head);
        }
        assert!(
            doc.selection_command(
                SelectionCommand::Shrink,
                Language::Rust,
                Indentation::default()
            )
            .unwrap()
        );
        assert_eq!(doc.selections()[0].range(), 4..9);
        doc.apply(
            vec![super::super::Edit::replace(0..0, "x")],
            vec![Selection::caret(1)],
            None,
        )
        .unwrap();
        assert!(
            !doc.selection_command(
                SelectionCommand::Shrink,
                Language::Rust,
                Indentation::default()
            )
            .unwrap()
        );
    }
    #[test]
    fn occurrence_limits_fail_without_selection_or_history_mutation() {
        let mut doc = Document::new("x ".repeat(MAX_SELECTIONS + 1));
        let before = doc.clone();
        assert_eq!(
            doc.selection_command(
                SelectionCommand::AllOccurrences,
                Language::Plain,
                Indentation::default()
            ),
            Err(SelectionError::Edit(EditError::TooManySelections))
        );
        assert_eq!(doc, before);
    }
    #[test]
    fn vertical_cursors_and_toggle_use_source_rows_and_tab_stops() {
        let mut doc = Document::new("\tfoo\r\n    bar\r\nx");
        doc.set_selections(vec![Selection::caret(1)]).unwrap();
        doc.selection_command(
            SelectionCommand::AddBelow,
            Language::Rust,
            Indentation::default(),
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(10));
        doc.selection_command(
            SelectionCommand::AddBelow,
            Language::Rust,
            Indentation::default(),
        )
        .unwrap();
        assert_eq!(doc.selections()[0], Selection::caret(doc.text().len()));
        assert_eq!(doc.selections().len(), 3);
        doc.toggle_cursor(1).unwrap();
        assert_eq!(doc.selections().len(), 2);
    }
}
