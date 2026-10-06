//! Clipboard fragments are shared source policy. Adapters only read/write MIME data.
use super::{Document, EditError, MAX_DOCUMENT_BYTES, MAX_SELECTIONS, Selection};
use serde::{Deserialize, Serialize};

pub const CLIPBOARD_SELECTIONS_MIME: &str = "application/x-openwebide-selections";
const MAX_METADATA_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardContent {
    pub text: String,
    pub metadata: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    length: usize,
    fingerprint: u64,
    ranges: Vec<[usize; 2]>,
}

fn fingerprint(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn fragments<'a>(text: &'a str, metadata: &str, count: usize) -> Option<Vec<&'a str>> {
    if metadata.len() > MAX_METADATA_BYTES
        || count > MAX_SELECTIONS
        || text.len() > MAX_DOCUMENT_BYTES + MAX_SELECTIONS
    {
        return None;
    }
    let metadata: Metadata = serde_json::from_str(metadata).ok()?;
    if metadata.version != 1
        || metadata.length != text.len()
        || metadata.fingerprint != fingerprint(text)
        || metadata.ranges.len() != count
    {
        return None;
    }
    let mut result = Vec::with_capacity(count);
    let mut end = 0;
    let mut has_text = false;
    for [start, next_end] in metadata.ranges {
        let fragment = text.get(start..next_end)?;
        if fragment.is_empty() {
            if start != end {
                return None;
            }
        } else {
            let expected = end + usize::from(has_text);
            if start != expected || has_text && text.get(end..start) != Some("\n") {
                return None;
            }
            end = next_end;
            has_text = true;
        }
        result.push(fragment);
    }
    (end == text.len()).then_some(result)
}

impl Document {
    pub fn clipboard_content(&self) -> Result<ClipboardContent, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        let mut text = String::new();
        let mut ranges = Vec::with_capacity(self.selections.len());
        let mut has_text = false;
        for selection in &self.selections {
            let fragment = &self.text[selection.range()];
            if !fragment.is_empty() {
                if has_text {
                    text.push('\n');
                }
                let start = text.len();
                text.push_str(fragment);
                ranges.push([start, text.len()]);
                has_text = true;
            } else {
                ranges.push([text.len(), text.len()]);
            }
        }
        let metadata = serde_json::to_string(&Metadata {
            version: 1,
            length: text.len(),
            fingerprint: fingerprint(&text),
            ranges,
        })
        .expect("clipboard metadata contains only bounded integers");
        Ok(ClipboardContent { text, metadata })
    }

    /// Foreign, stripped or invalid metadata falls back to plain-text paste.
    pub fn paste_clipboard(
        &mut self,
        text: &str,
        metadata: Option<&str>,
    ) -> Result<bool, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        if let Some(fragments) =
            metadata.and_then(|metadata| fragments(text, metadata, self.selections.len()))
        {
            return self.replace_fragments(&fragments);
        }
        self.paste_selections(text)
    }

    pub fn paste_clipboard_with_indentation(
        &mut self,
        text: &str,
        metadata: Option<&str>,
        indentation: super::Indentation,
        ending: Option<super::LineEnding>,
    ) -> Result<bool, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        if let Some(fragments) =
            metadata.and_then(|metadata| fragments(text, metadata, self.selections.len()))
        {
            self.paste_fragments_with_indentation(&fragments, indentation, ending)
        } else {
            self.paste_with_indentation(text, indentation, ending)
        }
    }

    pub(super) fn replace_fragments(&mut self, fragments: &[&str]) -> Result<bool, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        if fragments.len() != self.selections.len() {
            return Err(EditError::InvalidSelection);
        }
        let remaining = self
            .selections
            .iter()
            .fold(self.text.len(), |size, selection| {
                size - selection.range().len()
            });
        let output = fragments.iter().try_fold(remaining, |size, fragment| {
            size.checked_add(fragment.len())
                .ok_or(EditError::OutputTooLarge)
        })?;
        if output > MAX_DOCUMENT_BYTES {
            return Err(EditError::OutputTooLarge);
        }
        self.apply_grouped_caret_edits(
            self.selections
                .iter()
                .zip(fragments)
                .map(|(selection, fragment)| {
                    (
                        super::Edit::replace(selection.range(), *fragment),
                        Selection::caret(fragment.len()),
                    )
                })
                .collect(),
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_indentation_preserves_each_fragment_and_groups_undo() {
        let mut copied = Document::new("  first\n    nested\n---\n\tsecond\n\t\tchild");
        let split = copied.text().find("\n---\n").unwrap();
        copied
            .set_selections(vec![
                Selection {
                    anchor: 0,
                    head: split,
                },
                Selection {
                    anchor: split + 5,
                    head: copied.text().len(),
                },
            ])
            .unwrap();
        let clipboard = copied.clipboard_content().unwrap();
        let original = "  \n    ";
        let mut target = Document::new(original);
        target
            .set_selections(vec![Selection::caret(2), Selection::caret(7)])
            .unwrap();
        let before = target.clone();
        target
            .paste_clipboard_with_indentation(
                &clipboard.text,
                Some(&clipboard.metadata),
                super::super::Indentation {
                    width: 2,
                    tab_width: 2,
                    ..Default::default()
                },
                Some(super::super::LineEnding::CrLf),
            )
            .unwrap();
        assert_eq!(
            target.text(),
            "  first\r\n    nested\n    second\r\n      child"
        );
        assert!(target.undo());
        assert_eq!(target.text(), before.text());
        assert_eq!(target.selections(), before.selections());
        target.begin_composition(None);
        let composing = target.clone();
        assert_eq!(
            target.paste_clipboard_with_indentation(
                &clipboard.text,
                Some(&clipboard.metadata),
                super::super::Indentation {
                    width: 2,
                    tab_width: 2,
                    ..Default::default()
                },
                None,
            ),
            Err(EditError::CompositionActive)
        );
        assert_eq!(target, composing);
    }
    #[test]
    fn multiline_fragments_round_trip_primary_order_direction_unicode_and_crlf() {
        let source = "文\r\na\n---\n😀\nb";
        let mut doc = Document::new(source);
        let selections = vec![
            Selection {
                anchor: source.len(),
                head: 11,
            },
            Selection { anchor: 0, head: 7 },
        ];
        doc.set_selections(selections.clone()).unwrap();
        let clipboard = doc.clipboard_content().unwrap();
        assert_eq!(clipboard.text, "😀\nb\n文\r\na\n");
        doc.replace_selections("", None).unwrap();
        assert!(
            doc.paste_clipboard(&clipboard.text, Some(&clipboard.metadata))
                .unwrap()
        );
        assert_eq!(doc.text(), source);
        doc.undo();
        assert_eq!(doc.text(), "---\n");
        doc.undo();
        assert_eq!(doc.selections(), selections);
    }
    #[test]
    fn empty_fragments_remain_empty_and_invalid_metadata_falls_back() {
        let mut doc = Document::new("foo\nbar");
        doc.set_selections(vec![Selection::caret(0), Selection { anchor: 4, head: 7 }])
            .unwrap();
        let clipboard = doc.clipboard_content().unwrap();
        assert_eq!(clipboard.text, "bar");
        doc.paste_clipboard(&clipboard.text, Some(&clipboard.metadata))
            .unwrap();
        assert_eq!(doc.text(), "foo\nbar");
        for metadata in ["{}", "[]", "{broken", &"x".repeat(MAX_METADATA_BYTES + 1)] {
            let mut target = Document::new("xx");
            target
                .set_selections(vec![Selection::caret(0), Selection::caret(2)])
                .unwrap();
            target.paste_clipboard("a\nb", Some(metadata)).unwrap();
            assert_eq!(target.text(), "axxb");
        }
        let mut changed = Document::new("xx");
        changed
            .set_selections(vec![Selection::caret(0), Selection::caret(2)])
            .unwrap();
        changed
            .paste_clipboard("changed", Some(&clipboard.metadata))
            .unwrap();
        assert_eq!(changed.text(), "changedxxchanged");
    }
    #[test]
    fn malformed_ranges_cannot_split_utf8_skip_text_or_exceed_the_selection_count() {
        for ranges in [
            vec![[0, 1], [1, 4]],
            vec![[0, 0], [3, 4]],
            vec![[0, 3], [0, 3]],
        ] {
            let text = "文x";
            let metadata = serde_json::to_string(&Metadata {
                version: 1,
                length: text.len(),
                fingerprint: fingerprint(text),
                ranges,
            })
            .unwrap();
            assert!(fragments(text, &metadata, 2).is_none());
        }
        let mut doc = Document::new("ab");
        doc.set_selections(vec![Selection::caret(0), Selection::caret(2)])
            .unwrap();
        doc.begin_composition(None);
        let before = doc.clone();
        assert_eq!(
            doc.paste_clipboard("x", None),
            Err(EditError::CompositionActive)
        );
        assert_eq!(doc, before);
    }
}
