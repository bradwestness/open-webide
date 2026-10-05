//! Explicit paste indentation rebases a snippet; ordinary paste remains native.
use super::{Document, Edit, EditError, Indentation, LineEnding, Selection, indent::line_start};
impl Document {
    pub fn paste_with_indentation(
        &mut self,
        text: &str,
        indentation: Indentation,
        ending: Option<LineEnding>,
    ) -> Result<bool, EditError> {
        if text.is_empty() {
            return Ok(false);
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let pasted: Vec<_> = normalized.split('\n').collect();
        let common = pasted
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let prefix: String = line
                    .chars()
                    .take_while(|ch| matches!(ch, ' ' | '\t'))
                    .collect();
                indentation.visual_width(&prefix)
            })
            .min()
            .unwrap_or(0);
        let ending = ending
            .unwrap_or_else(|| LineEnding::detect(&self.text))
            .text();
        if normalized.trim().is_empty() {
            let text = normalized.replace('\n', ending);
            return self.apply_caret_edits(
                self.selections
                    .iter()
                    .map(|selection| {
                        (
                            Edit::replace(selection.range(), &text),
                            Selection::caret(text.len()),
                        )
                    })
                    .collect(),
            );
        }
        let changes = self
            .selections
            .iter()
            .map(|selection| {
                let range = selection.range();
                let start = line_start(&self.text, range.start);
                let before = &self.text[start..range.start];
                let base: String = before
                    .chars()
                    .take_while(|ch| matches!(ch, ' ' | '\t'))
                    .collect();
                let columns = indentation.visual_width(&base);
                let mut inserted = String::new();
                for (index, line) in pasted.iter().enumerate() {
                    if index > 0 {
                        inserted.push_str(ending);
                    }
                    let body = line.trim_start_matches([' ', '\t']);
                    if body.is_empty() {
                        continue;
                    }
                    let prefix = &line[..line.len() - body.len()];
                    let relative = indentation.visual_width(prefix).saturating_sub(common);
                    inserted.push_str(&if index == 0 {
                        indentation.columns_from(indentation.visual_width(before), relative)
                    } else {
                        indentation.columns(columns + relative)
                    });
                    inserted.push_str(body);
                }
                let caret = inserted.len();
                (Edit::replace(range, inserted), Selection::caret(caret))
            })
            .collect();
        self.apply_caret_edits(changes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_clipboard_does_not_delete_a_selection() {
        let mut doc = Document::new("keep");
        doc.set_selections(vec![Selection { anchor: 0, head: 4 }])
            .unwrap();
        let before = doc.clone();
        assert!(
            !doc.paste_with_indentation("", Indentation::default(), None)
                .unwrap()
        );
        assert_eq!(doc, before);
    }

    #[test]
    fn whitespace_only_paste_is_not_discarded() {
        let mut doc = Document::new("x");
        doc.set_selections(vec![Selection { anchor: 0, head: 1 }])
            .unwrap();
        doc.paste_with_indentation("  \n\t", Indentation::default(), Some(LineEnding::CrLf))
            .unwrap();
        assert_eq!(doc.text(), "  \r\n\t");
        doc.undo();
        assert_eq!(doc.text(), "x");
    }

    #[test]
    fn explicit_paste_rebases_relative_tabs_crlf_and_unicode_as_one_transaction() {
        let mut doc = Document::new("  here\r\nnext");
        doc.set_selections(vec![Selection { anchor: 6, head: 2 }])
            .unwrap();
        doc.paste_with_indentation(
            "\t😀\n\t\tinner\n",
            Indentation {
                style: super::super::IndentStyle::Tabs,
                width: 2,
                tab_width: 3,
            },
            None,
        )
        .unwrap();
        assert_eq!(doc.text(), "  😀\r\n\t  inner\r\n\r\nnext");
        doc.undo();
        assert_eq!(doc.text(), "  here\r\nnext");
        assert_eq!(doc.selections(), &[Selection { anchor: 6, head: 2 }]);
    }
}
