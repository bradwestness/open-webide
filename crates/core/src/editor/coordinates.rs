//! Sparse coordinates keep native offset queries bounded within long lines.
use super::EditError;
use std::sync::Arc;

const STEP_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Coordinate {
    byte: usize,
    native: usize,
    chars: usize,
}

/// Checkpoints are relative to a logical source row, so unchanged rows retain
/// their index through insertions/deletions above them and folded projections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LineCoordinates {
    native: Option<Arc<[Coordinate]>>,
    visual: Option<super::VisualLineIndex>,
}

#[derive(Default)]
pub(super) struct RowSummary {
    pub utf16_len: usize,
    pub breaks: usize,
    pub oversized: bool,
}

impl LineCoordinates {
    pub fn new(text: &str) -> Self {
        if text.len() <= STEP_BYTES {
            return Self::default();
        }
        Self::with_summary(text).0
    }

    /// Admission and native totals share the exact coordinate traversal. Long
    /// rows also share the visual index's character pass, without changing its
    /// grapheme-safe paint boundaries.
    pub fn with_summary(text: &str) -> (Self, RowSummary) {
        let sparse = text.len() > STEP_BYTES;
        let mut checkpoints = Vec::new();
        let mut position = Coordinate::default();
        let mut summary = RowSummary::default();
        let mut width = 0;
        let mut last = 0;
        let mut previous_cr = false;
        let mut visit = |byte: usize, ch: char| {
            // Never start a query between CR and LF: both positions have the
            // same native offset, whose inverse must select the original CR.
            if sparse && byte - last >= STEP_BYTES && !(previous_cr && ch == '\n') {
                position.byte = byte;
                checkpoints.push(position);
                last = byte;
            }
            position.chars += 1;
            summary.utf16_len += ch.len_utf16();
            if !(ch == '\r' && text.as_bytes().get(byte + 1) == Some(&b'\n')) {
                position.native += ch.len_utf16();
            }
            if matches!(ch, '\r' | '\n') {
                summary.breaks += usize::from(ch != '\n' || !previous_cr);
                width = 0;
            } else {
                width += ch.len_utf8();
                summary.oversized |= width > super::MAX_EDITOR_LINE_BYTES;
            }
            previous_cr = ch == '\r';
        };
        let body = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        let visual = (body.len() > super::MAX_MEASURE_BYTES)
            .then(|| super::VisualLineIndex::with_character_scan(body, &mut visit))
            .flatten();
        if visual.is_none() {
            for (byte, ch) in body.char_indices() {
                visit(byte, ch);
            }
        }
        for (byte, ch) in text[body.len()..].char_indices() {
            visit(body.len() + byte, ch);
        }
        (
            Self {
                native: sparse.then(|| checkpoints.into()),
                visual,
            },
            summary,
        )
    }
    pub fn visual(&self) -> Option<super::VisualLineIndex> {
        self.visual.clone()
    }
    fn checkpoints(&self) -> &[Coordinate] {
        self.native.as_deref().unwrap_or_default()
    }
    fn byte_start(&self, offset: usize) -> Coordinate {
        self.checkpoints()
            .partition_point(|point| point.byte <= offset)
            .checked_sub(1)
            .map_or_else(Coordinate::default, |index| self.checkpoints()[index])
    }
    pub fn byte_to_textarea(&self, text: &str, offset: usize) -> Result<usize, EditError> {
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(EditError::InvalidSelection);
        }
        let start = self.byte_start(offset);
        Ok(start.native + super::byte_to_textarea(&text[start.byte..], offset - start.byte)?)
    }
    pub fn textarea_to_byte(&self, text: &str, offset: usize) -> usize {
        let start = self
            .checkpoints()
            .partition_point(|point| point.native <= offset)
            .checked_sub(1)
            .map_or_else(Coordinate::default, |index| self.checkpoints()[index]);
        start.byte + super::textarea_to_byte(&text[start.byte..], offset - start.native)
    }
    pub fn chars_before(&self, text: &str, offset: usize) -> usize {
        let start = self.byte_start(offset);
        start.chars + text[start.byte..offset].chars().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_row_summaries_match_independent_byte_and_utf16_scans() {
        for text in [
            String::new(),
            "文😀\r\n\r\t".into(),
            "文😀e\u{301}\r\n".repeat(3000),
            "🇺🇸👩‍👩‍👧‍👦क्ष \rword\n".repeat(1000),
            "a".repeat(super::super::MAX_EDITOR_LINE_BYTES),
            "a".repeat(super::super::MAX_EDITOR_LINE_BYTES) + "😀\r\n",
            "a".repeat(super::super::MAX_STRUCTURE_BYTES + 1) + "\r\n",
        ] {
            let (coordinates, summary) = LineCoordinates::with_summary(&text);
            let (breaks, oversized) = super::super::capacity::row_admission(&text);
            assert_eq!(summary.utf16_len, text.encode_utf16().count());
            assert_eq!((summary.breaks, summary.oversized), (breaks, oversized));
            let body = text
                .strip_suffix("\r\n")
                .or_else(|| text.strip_suffix('\n'))
                .unwrap_or(&text);
            assert_eq!(
                coordinates.visual(),
                (body.len() > super::super::MAX_MEASURE_BYTES)
                    .then(|| super::super::VisualLineIndex::new(body))
                    .flatten()
            );
        }
    }

    #[test]
    fn long_unicode_and_crlf_queries_match_reference_and_bound_scan_distance() {
        for text in [
            "a".repeat(511) + "\r\n😀文e\u{301}\t\r" + &"文😀".repeat(1000),
            "文😀e\u{301}\t\r\n".repeat(1000),
            "a".repeat(512) + "\r\n" + &"a".repeat(512) + "\r\n",
        ] {
            let index = LineCoordinates::new(&text);
            for byte in text
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([text.len()])
            {
                assert!(byte - index.byte_start(byte).byte <= STEP_BYTES + 4);
                assert_eq!(
                    index.byte_to_textarea(&text, byte),
                    super::super::byte_to_textarea(&text, byte)
                );
                assert_eq!(
                    index.chars_before(&text, byte),
                    text[..byte].chars().count()
                );
            }
            for native in 0..=text.encode_utf16().count() + 2 {
                assert_eq!(
                    index.textarea_to_byte(&text, native),
                    super::super::textarea_to_byte(&text, native)
                );
            }
            assert_eq!(
                index.byte_to_textarea(&text, text.len() + 1),
                Err(EditError::InvalidSelection)
            );
        }
    }
}
