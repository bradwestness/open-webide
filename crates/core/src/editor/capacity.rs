//! Shared full-editor admission and bounded read-only text pages.
use std::ops::Range;

pub const MAX_EDITOR_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EDITOR_LINES: usize = 100_000;
pub const MAX_EDITOR_LINE_BYTES: usize = 1024 * 1024;
pub const TEXT_PAGE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorLimit {
    Bytes,
    Lines,
    LineBytes,
}
impl std::fmt::Display for EditorLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Bytes => "File exceeds the full editor's 8 MiB limit",
            Self::Lines => "File exceeds the full editor's 100,000 line limit",
            Self::LineBytes => "File contains a line longer than the full editor's 1 MiB limit",
        })
    }
}

/// Check before allocating line/projection metadata. LF and standalone CR both
/// count as display breaks; CRLF counts once. The scan allocates no metadata and
/// stops at the first exceeded limit.
pub fn editor_limit(source: &str) -> Option<EditorLimit> {
    editor_limit_parts(&[source])
}

/// Validate a proposed transaction without joining its unchanged source pieces.
pub(super) fn editor_limit_parts(parts: &[&str]) -> Option<EditorLimit> {
    if parts
        .iter()
        .try_fold(0_usize, |size, part| size.checked_add(part.len()))
        .is_none_or(|size| size > MAX_EDITOR_BYTES)
    {
        return Some(EditorLimit::Bytes);
    }
    let mut lines = 1;
    let mut width = 0;
    let mut carriage_return = false;
    for byte in parts.iter().flat_map(|part| part.bytes()) {
        match byte {
            b'\r' | b'\n' => {
                if byte != b'\n' || !carriage_return {
                    lines += 1;
                    if lines > MAX_EDITOR_LINES {
                        return Some(EditorLimit::Lines);
                    }
                }
                width = 0;
            }
            _ => {
                width += 1;
                if width > MAX_EDITOR_LINE_BYTES {
                    return Some(EditorLimit::LineBytes);
                }
            }
        }
        carriage_return = byte == b'\r';
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPage {
    pub index: usize,
    pub pages: usize,
    pub range: Range<usize>,
}
impl TextPage {
    /// Adjacent pages cover the exact source without splitting UTF-8 characters.
    /// The source remains separate; a viewer cannot accidentally save a page.
    pub fn new(source: &str, requested: usize) -> Self {
        let pages = source.len().div_ceil(TEXT_PAGE_BYTES).max(1);
        let index = requested.min(pages - 1);
        let boundary = |offset: usize| {
            let mut offset = offset.min(source.len());
            while !source.is_char_boundary(offset) {
                offset -= 1;
            }
            offset
        };
        Self {
            index,
            pages,
            range: boundary(index * TEXT_PAGE_BYTES)..boundary((index + 1) * TEXT_PAGE_BYTES),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proposed_parts_preserve_admission_priority_and_cross_piece_line_endings() {
        for source in [
            "".to_string(),
            "文😀\r\nx\r\n\r".to_string(),
            "\r\n".repeat(MAX_EDITOR_LINES),
            "x".repeat(MAX_EDITOR_LINE_BYTES + 1),
            "x".repeat(MAX_EDITOR_BYTES + 1),
        ] {
            for at in [
                0,
                source.len() / 2,
                source.len().saturating_sub(1),
                source.len(),
            ] {
                let at = source.floor_char_boundary(at);
                assert_eq!(
                    editor_limit_parts(&[&source[..at], "", &source[at..]]),
                    editor_limit(&source)
                );
            }
        }
        assert_eq!(editor_limit_parts(&["\r", "\n"]), None);
    }

    #[test]
    fn admission_bounds_bytes_rows_and_single_lines_without_confusing_crlf() {
        assert_eq!(
            editor_limit(&"x".repeat(MAX_EDITOR_BYTES + 1)),
            Some(EditorLimit::Bytes)
        );
        assert_eq!(editor_limit(&"\n".repeat(MAX_EDITOR_LINES - 1)), None);
        for ending in ["\n", "\r", "\r\n"] {
            assert_eq!(
                editor_limit(&ending.repeat(MAX_EDITOR_LINES)),
                Some(EditorLimit::Lines)
            );
        }
        assert_eq!(editor_limit(&"x".repeat(MAX_EDITOR_LINE_BYTES)), None);
        assert_eq!(
            editor_limit(&"x".repeat(MAX_EDITOR_LINE_BYTES + 1)),
            Some(EditorLimit::LineBytes)
        );
        assert_eq!(
            editor_limit(&format!(
                "{}\r\n{}",
                "x".repeat(MAX_EDITOR_LINE_BYTES),
                "x".repeat(MAX_EDITOR_LINE_BYTES)
            )),
            None
        );
    }

    #[test]
    fn interactive_transactions_reject_capacity_before_indexing_and_keep_history() {
        use crate::editor::{Document, Edit, EditError, Selection};
        let mut document = Document::for_editor("base").unwrap();
        document
            .apply(
                vec![Edit::replace(4..4, "!")],
                vec![Selection::caret(5)],
                None,
            )
            .unwrap();
        let before = document.clone();
        for text in [
            "x".repeat(MAX_EDITOR_LINE_BYTES + 1),
            "\n".repeat(MAX_EDITOR_LINES),
        ] {
            let expected = editor_limit(&text).unwrap();
            assert_eq!(
                document.apply(
                    vec![Edit::replace(0..5, &text)],
                    vec![Selection::caret(0)],
                    None
                ),
                Err(EditError::Capacity(expected))
            );
            assert_eq!(document, before);
        }
        assert!(document.undo());
        assert_eq!(document.text(), "base");
        assert!(document.redo());
        assert_eq!(document.text(), "base!");
        assert_eq!(
            Document::for_editor("\r".repeat(MAX_EDITOR_LINES)),
            Err(EditError::Capacity(EditorLimit::Lines))
        );
    }

    proptest::proptest! {
        #[test]
        fn pages_cover_unicode_source_exactly(source in ".{0,25000}") {
            let first = TextPage::new(&source, 0);
            let mut rebuilt = String::new();
            let mut previous_end = 0;
            for index in 0..first.pages {
                let page = TextPage::new(&source, index);
                proptest::prop_assert_eq!(page.range.start, previous_end);
                proptest::prop_assert!(page.range.len() <= TEXT_PAGE_BYTES + 3);
                rebuilt.push_str(&source[page.range.clone()]);
                previous_end = page.range.end;
            }
            proptest::prop_assert_eq!(rebuilt, source.clone());
            proptest::prop_assert_eq!(TextPage::new(&source, usize::MAX).index, first.pages - 1);
        }
    }
}
