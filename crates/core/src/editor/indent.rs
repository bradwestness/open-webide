//! Indentation commands use the document transaction engine, preserving line endings.
use std::collections::BTreeSet;

use super::{Document, Edit, EditError, Selection};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndentStyle {
    #[default]
    Spaces,
    Tabs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    pub style: IndentStyle,
    pub width: usize,
}

impl Default for Indentation {
    fn default() -> Self {
        Self {
            style: IndentStyle::Spaces,
            width: 4,
        }
    }
}

impl Indentation {
    pub fn width(self) -> usize {
        self.width.clamp(1, 16)
    }
    pub fn unit(self) -> String {
        match self.style {
            IndentStyle::Spaces => " ".repeat(self.width()),
            IndentStyle::Tabs => "\t".into(),
        }
    }
}

impl Document {
    /// Indent/outdent each selected logical line once, even when selections overlap.
    pub fn indent_lines(
        &mut self,
        indentation: Indentation,
        outdent: bool,
    ) -> Result<bool, EditError> {
        let mut starts = BTreeSet::new();
        for selection in &self.selections {
            let range = selection.range();
            let start = line_start(&self.text, range.start);
            let end = if range.end > range.start && self.text[..range.end].ends_with('\n') {
                range.end - 1
            } else {
                range.end
            };
            starts.insert(start);
            for (offset, ch) in self.text[start..end].char_indices() {
                if ch == '\n' {
                    starts.insert(start + offset + 1);
                }
            }
        }
        let edits = starts
            .into_iter()
            .filter_map(|start| {
                if outdent {
                    let line = &self.text[start..];
                    let remove = if line.starts_with('\t') {
                        1
                    } else {
                        line.bytes()
                            .take_while(|byte| *byte == b' ')
                            .take(indentation.width())
                            .count()
                    };
                    (remove > 0).then(|| Edit::replace(start..start + remove, ""))
                } else {
                    Some(Edit::replace(start..start, indentation.unit()))
                }
            })
            .collect();
        self.apply_mapped(edits)
    }

    /// Tab at carets advances to the next tab stop; a selection indents whole lines.
    pub fn tab(&mut self, indentation: Indentation) -> Result<bool, EditError> {
        if self
            .selections
            .iter()
            .any(|selection| selection.anchor != selection.head)
        {
            return self.indent_lines(indentation, false);
        }
        let edits = self
            .selections
            .iter()
            .map(|selection| {
                let position = selection.head;
                let text = match indentation.style {
                    IndentStyle::Tabs => "\t".into(),
                    IndentStyle::Spaces => {
                        let width = indentation.width();
                        let column = self.text[line_start(&self.text, position)..position]
                            .chars()
                            .fold(0, |column, ch| {
                                if ch == '\t' {
                                    column + width - column % width
                                } else {
                                    column + 1
                                }
                            });
                        " ".repeat(width - column % width)
                    }
                };
                Edit::replace(position..position, text)
            })
            .collect();
        self.apply_mapped(edits)
    }

    /// Enter retains the current line's indentation and the document's line ending.
    pub fn newline(&mut self) -> Result<bool, EditError> {
        let ending = if self
            .text
            .split_once('\n')
            .is_some_and(|(prefix, _)| prefix.ends_with('\r'))
        {
            "\r\n"
        } else {
            "\n"
        };
        let edits = self
            .selections
            .iter()
            .map(|selection| {
                let range = selection.range();
                let start = line_start(&self.text, range.start);
                let prefix: String = self.text[start..range.start]
                    .chars()
                    .take_while(|ch| matches!(ch, ' ' | '\t'))
                    .collect();
                Edit::replace(range, format!("{ending}{prefix}"))
            })
            .collect();
        self.apply_mapped(edits)
    }

    fn apply_mapped(&mut self, mut edits: Vec<Edit>) -> Result<bool, EditError> {
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        let selections = self
            .selections
            .iter()
            .map(|selection| Selection {
                anchor: mapped_position(selection.anchor, &edits),
                head: mapped_position(selection.head, &edits),
            })
            .collect();
        self.apply(edits, selections, None)
    }
}

fn line_start(text: &str, position: usize) -> usize {
    text[..position].rfind('\n').map_or(0, |index| index + 1)
}

fn mapped_position(position: usize, edits: &[Edit]) -> usize {
    let mut source = 0;
    let mut target = 0;
    for edit in edits {
        if position < edit.range.start {
            return target + position - source;
        }
        target += edit.range.start - source;
        if position <= edit.range.end {
            return target + edit.text.len();
        }
        target += edit.text.len();
        source = edit.range.end;
    }
    target + position - source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_lines_indent_once_and_preserve_direction_crlf_and_final_newline() {
        let mut doc = Document::new("a\r\nb\r\nc");
        doc.set_selections(vec![
            Selection { anchor: 6, head: 0 },
            Selection { anchor: 1, head: 4 },
        ])
        .unwrap();
        doc.indent_lines(
            Indentation {
                style: IndentStyle::Spaces,
                width: 2,
            },
            false,
        )
        .unwrap();
        assert_eq!(doc.text(), "  a\r\n  b\r\nc");
        assert_eq!(
            doc.selections()[0],
            Selection {
                anchor: 10,
                head: 2
            }
        );
        assert!(doc.undo());
        assert_eq!(doc.text(), "a\r\nb\r\nc");
        doc.indent_lines(
            Indentation {
                style: IndentStyle::Tabs,
                width: 4,
            },
            false,
        )
        .unwrap();
        assert_eq!(doc.text(), "\ta\r\n\tb\r\nc");
    }

    #[test]
    fn tab_stops_outdent_and_enter_preserve_unicode_and_indentation() {
        let mut doc = Document::new("  😀value\r\n");
        doc.set_selections(vec![Selection::caret(6)]).unwrap();
        doc.tab(Indentation::default()).unwrap();
        assert_eq!(doc.text(), "  😀 value\r\n");
        doc.newline().unwrap();
        assert_eq!(doc.text(), "  😀 \r\n  value\r\n");
        doc.indent_lines(Indentation::default(), true).unwrap();
        assert_eq!(doc.text(), "  😀 \r\nvalue\r\n");
        doc.undo();
        assert_eq!(doc.text(), "  😀 \r\n  value\r\n");
    }
}
