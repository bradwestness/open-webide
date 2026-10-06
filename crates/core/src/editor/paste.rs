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
        self.paste_fragments_with_indentation(
            &vec![text; self.selections.len()],
            indentation,
            ending,
        )
    }

    pub(super) fn paste_fragments_with_indentation(
        &mut self,
        fragments: &[&str],
        indentation: Indentation,
        ending: Option<LineEnding>,
    ) -> Result<bool, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        if fragments.len() != self.selections.len() {
            return Err(EditError::InvalidSelection);
        }
        let retained = self
            .selections
            .iter()
            .fold(self.text.len(), |size, selection| {
                size - selection.range().len()
            });
        let mut budget = super::MAX_DOCUMENT_BYTES
            .checked_sub(retained)
            .ok_or(EditError::OutputTooLarge)?;
        let ending = ending
            .unwrap_or_else(|| LineEnding::detect(&self.text))
            .text();
        // Repeated plain-text paste prepares the same snippet once, even with 512 cursors.
        let mut prepared: Option<(&str, String, usize, bool)> = None;
        let changes = self
            .selections
            .iter()
            .zip(fragments)
            .map(|(selection, text)| {
                if text.len() > super::MAX_DOCUMENT_BYTES {
                    return Err(EditError::OutputTooLarge);
                }
                if prepared
                    .as_ref()
                    .is_none_or(|(source, _, _, _)| !std::ptr::eq(*source, *text))
                {
                    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                    let common = normalized
                        .split('\n')
                        .filter(|line| !line.trim().is_empty())
                        .map(|line| {
                            let prefix =
                                &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
                            indentation.visual_width(prefix)
                        })
                        .min()
                        .unwrap_or(0);
                    let whitespace = normalized.trim().is_empty();
                    prepared = Some((text, normalized, common, whitespace));
                }
                let (_, normalized, common, whitespace) =
                    prepared.as_ref().expect("current snippet is prepared");
                let range = selection.range();
                let mut inserted = String::new();
                if *whitespace {
                    for (index, line) in normalized.split('\n').enumerate() {
                        if index > 0 {
                            push_bounded(&mut inserted, ending, &mut budget)?;
                        }
                        push_bounded(&mut inserted, line, &mut budget)?;
                    }
                } else {
                    let pasted: Vec<_> = normalized.split('\n').collect();
                    let start = line_start(&self.text, range.start);
                    let before = &self.text[start..range.start];
                    let base: String = before
                        .chars()
                        .take_while(|ch| matches!(ch, ' ' | '\t'))
                        .collect();
                    let columns = indentation.visual_width(&base);
                    for (index, line) in pasted.iter().enumerate() {
                        if index > 0 {
                            push_bounded(&mut inserted, ending, &mut budget)?;
                        }
                        let body = line.trim_start_matches([' ', '\t']);
                        if body.is_empty() {
                            continue;
                        }
                        let prefix = &line[..line.len() - body.len()];
                        let relative = indentation.visual_width(prefix).saturating_sub(*common);
                        let (start, width) = if index == 0 {
                            (indentation.visual_width(before), relative)
                        } else {
                            (0, columns + relative)
                        };
                        if indentation.columns_from_len(start, width) > budget {
                            return Err(EditError::OutputTooLarge);
                        }
                        push_bounded(
                            &mut inserted,
                            &indentation.columns_from(start, width),
                            &mut budget,
                        )?;
                        push_bounded(&mut inserted, body, &mut budget)?;
                    }
                }
                let caret = inserted.len();
                Ok((Edit::replace(range, inserted), Selection::caret(caret)))
            })
            .collect::<Result<Vec<_>, EditError>>()?;
        self.apply_caret_edits(changes)
    }
}
fn push_bounded(output: &mut String, text: &str, budget: &mut usize) -> Result<(), EditError> {
    *budget = budget
        .checked_sub(text.len())
        .ok_or(EditError::OutputTooLarge)?;
    output.push_str(text);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replicated_paste_limits_are_atomic_for_text_and_whitespace() {
        let mut doc = Document::new("x".repeat(super::super::MAX_SELECTIONS));
        doc.set_selections(
            (0..super::super::MAX_SELECTIONS)
                .map(Selection::caret)
                .collect(),
        )
        .unwrap();
        for clipboard in ["x".repeat(65_536), " ".repeat(65_536)] {
            let before = doc.clone();
            assert_eq!(
                doc.paste_with_indentation(&clipboard, Indentation::default(), None),
                Err(EditError::OutputTooLarge)
            );
            assert_eq!(doc, before);
        }
    }

    #[test]
    fn indentation_preflight_matches_actual_tabs_and_spaces() {
        for style in [
            super::super::IndentStyle::Spaces,
            super::super::IndentStyle::Tabs,
        ] {
            for tab_width in 1..=16 {
                let indentation = Indentation {
                    style,
                    tab_width,
                    width: 4,
                };
                for start in 0..=32 {
                    for width in 0..=64 {
                        assert_eq!(
                            indentation.columns_from_len(start, width),
                            indentation.columns_from(start, width).len()
                        );
                    }
                }
            }
        }
    }
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
