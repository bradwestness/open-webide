//! Borrowed proposed native input: unchanged source is never joined into a value.
use super::{Edit, EditError, Selection, TextChange};
use std::ops::Range;

pub(super) struct NativeValue<'a> {
    parts: [&'a str; 3],
    len: usize,
}
impl<'a> NativeValue<'a> {
    pub fn plain(source: &'a str) -> Self {
        Self {
            parts: [source, "", ""],
            len: source.len(),
        }
    }

    pub fn replacement(source: &'a str, edit: Option<&'a Edit>) -> Result<Self, EditError> {
        let Some(edit) = edit else {
            return Ok(Self::plain(source));
        };
        source
            .get(edit.range.clone())
            .ok_or(EditError::InvalidRange)?;
        let len = source
            .len()
            .checked_sub(edit.range.len())
            .and_then(|len| len.checked_add(edit.text.len()))
            .ok_or(EditError::OutputTooLarge)?;
        Ok(Self {
            parts: [
                &source[..edit.range.start],
                &edit.text,
                &source[edit.range.end..],
            ],
            len,
        })
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    fn slices(&self, range: Range<usize>) -> Option<[&'a str; 3]> {
        if range.start > range.end || range.end > self.len {
            return None;
        }
        let mut offset = 0;
        let [first, second, third] = self.parts.map(|part| {
            let start = range.start.saturating_sub(offset).min(part.len());
            let end = range.end.saturating_sub(offset).min(part.len());
            offset += part.len();
            part.get(start..end)
        });
        Some([first?, second?, third?])
    }

    pub fn validate_selection(&self, selection: Selection) -> Result<(), EditError> {
        for offset in [selection.anchor, selection.head] {
            self.slices(offset..offset)
                .ok_or(EditError::InvalidSelection)?;
        }
        Ok(())
    }

    pub fn native_selection(&self, selection: Selection) -> Selection {
        let map = |offset| {
            super::textarea_chars_to_byte(self.parts.iter().flat_map(|part| part.chars()), offset)
        };
        Selection {
            anchor: map(selection.anchor),
            head: map(selection.head),
        }
    }

    pub fn matches(&self, range: Range<usize>, expected: &str) -> bool {
        if range.len() != expected.len() {
            return false;
        }
        let Some(parts) = self.slices(range) else {
            return false;
        };
        let mut offset = 0;
        parts.iter().all(|part| {
            let same = expected
                .get(offset..offset + part.len())
                .is_some_and(|expected| part.as_ptr() == expected.as_ptr() || *part == expected);
            offset += part.len();
            same
        })
    }

    pub fn inserted_range(&self, before: &str, span: &Range<usize>) -> Option<Range<usize>> {
        before.get(span.clone())?;
        let end = self.len.checked_sub(before.len() - span.end)?;
        (span.start <= end
            && self.matches(0..span.start, &before[..span.start])
            && self.matches(end..self.len, &before[span.end..]))
        .then_some(span.start..end)
    }

    pub fn inserted_text(&self, before: &str, span: &Range<usize>) -> Result<String, EditError> {
        let range = self
            .inserted_range(before, span)
            .ok_or(EditError::UnsupportedNativeInput)?;
        let parts = self
            .slices(range.clone())
            .ok_or(EditError::UnsupportedNativeInput)?;
        let mut text = String::with_capacity(range.len());
        for part in parts {
            text.push_str(part);
        }
        Ok(text)
    }

    pub fn change(&self, before: &str) -> Option<TextChange> {
        if self.matches(0..self.len, before) {
            return None;
        }
        let start = before
            .chars()
            .zip(self.parts.iter().flat_map(|part| part.chars()))
            .take_while(|(a, b)| a == b)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        let parts = self.slices(start..self.len)?;
        let suffix = before[start..]
            .chars()
            .rev()
            .zip(parts.iter().rev().flat_map(|part| part.chars().rev()))
            .take_while(|(a, b)| a == b)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        Some(TextChange {
            range: start..before.len() - suffix,
            new_end: self.len - suffix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_selection_preserves_crlf_across_piece_boundaries() {
        for (source, edit, expected) in [
            ("文\n😀", Edit::replace(3..3, "\r"), vec![0, 3, 5, 5, 9, 9]),
            ("\r😀", Edit::replace(1..1, "\n"), vec![0, 2, 2, 6, 6, 6]),
        ] {
            let value = NativeValue::replacement(source, Some(&edit)).unwrap();
            for (native, source) in expected.into_iter().enumerate() {
                assert_eq!(
                    value.native_selection(Selection::caret(native)),
                    Selection::caret(source)
                );
            }
        }
    }

    #[test]
    fn proposed_input_borrows_unchanged_source_and_validates_utf8_ranges() {
        let source = "文\r\n".repeat(1000);
        let edit = Edit::replace(5..5, "😀");
        let value = NativeValue::replacement(&source, Some(&edit)).unwrap();
        assert_eq!(value.parts[0].as_ptr(), source.as_ptr());
        assert_eq!(value.parts[1].as_ptr(), edit.text.as_ptr());
        assert_eq!(value.parts[2].as_ptr(), source[5..].as_ptr());
        assert_eq!(value.inserted_text(&source, &edit.range).unwrap(), "😀");
        for range in [1..2, 0..source.len() + 1, Range { start: 5, end: 0 }] {
            assert!(matches!(
                NativeValue::replacement(&source, Some(&Edit::replace(range, ""))),
                Err(EditError::InvalidRange)
            ));
        }
    }

    #[test]
    fn borrowed_native_values_match_materialized_unicode_replacements() {
        for source in ["", "fooo", "文😀\r\n文", "a\u{301}b"] {
            let boundaries: Vec<_> = source
                .char_indices()
                .map(|(at, _)| at)
                .chain([source.len()])
                .collect();
            for &start in &boundaries {
                for &end in boundaries.iter().filter(|&&end| end >= start) {
                    for text in ["", "o", "🦀\r\n", "文"] {
                        let edit = Edit::replace(start..end, text);
                        let value = NativeValue::replacement(source, Some(&edit)).unwrap();
                        let mut expected = source.to_string();
                        expected.replace_range(start..end, text);
                        assert!(value.matches(0..value.len(), &expected));
                        assert_eq!(
                            value.change(source),
                            super::super::text_change(source, &expected)
                        );
                        for offset in 0..=expected.len() + 1 {
                            assert_eq!(
                                value.validate_selection(Selection::caret(offset)).is_ok(),
                                offset <= expected.len() && expected.is_char_boundary(offset)
                            );
                        }
                        assert_eq!(value.inserted_text(source, &(start..end)).unwrap(), text);
                    }
                }
            }
        }
    }
}
