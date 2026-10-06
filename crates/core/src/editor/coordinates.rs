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
impl LineCoordinates {
    pub fn new(text: &str) -> Self {
        if text.len() <= STEP_BYTES {
            return Self::default();
        }
        let mut checkpoints = Vec::new();
        let mut chars = text.char_indices().peekable();
        let mut position = Coordinate::default();
        let mut last = 0;
        let mut previous_cr = false;
        while let Some((byte, ch)) = chars.next() {
            // Never start a query between CR and LF: both positions have the
            // same native offset, whose inverse must select the original CR.
            if byte - last >= STEP_BYTES && !(previous_cr && ch == '\n') {
                position.byte = byte;
                checkpoints.push(position);
                last = byte;
            }
            position.chars += 1;
            if !(ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n')) {
                position.native += ch.len_utf16();
            }
            previous_cr = ch == '\r';
        }
        let body = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        Self {
            native: Some(checkpoints.into()),
            visual: (body.len() > super::MAX_MEASURE_BYTES)
                .then(|| super::VisualLineIndex::new(body))
                .flatten(),
        }
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
