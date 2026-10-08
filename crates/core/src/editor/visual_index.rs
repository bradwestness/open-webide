//! Sparse source coordinates shared by paint and visual cursor measurements.
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

const STEP_BYTES: usize = 512;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Position {
    byte: usize,
    native: usize,
    glyph: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Coordinates {
    points: Vec<Position>,
    end: Position,
    horizontal: bool,
}

/// Immutable coordinates for one exact logical-line body. Callers retain the
/// associated source snapshot and must supply that same body to lookup methods.
/// Unchanged document rows and folded projections share the sparse allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualLineIndex(Arc<Coordinates>);
impl VisualLineIndex {
    #[cfg(any(test, feature = "test-support"))]
    pub fn shared_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn new(body: &str) -> Option<Self> {
        if body.len() > super::MAX_STRUCTURE_BYTES {
            return None;
        }
        let mut points = vec![Position::default()];
        let mut end = Position::default();
        let mut horizontal = body.len() > super::MAX_MEASURE_BYTES;
        for (byte, glyph) in body.grapheme_indices(true) {
            if byte - points.last()?.byte >= STEP_BYTES {
                points.push(Position { byte, ..end });
            }
            for ch in glyph.chars() {
                end.native += ch.len_utf16();
                if horizontal && !super::viewport::horizontal_paint_character(ch) {
                    horizontal = false;
                }
            }
            end.glyph += 1;
        }
        end.byte = body.len();
        if points.last()?.byte != end.byte {
            points.push(end);
        }
        Some(Self(Arc::new(Coordinates {
            points,
            end,
            horizontal,
        })))
    }
    /// Exact cluster checkpoints for a styled geometry adapter; omit EOF.
    pub fn anchor_glyphs(&self) -> impl Iterator<Item = usize> + '_ {
        self.0
            .points
            .iter()
            .map(|point| point.glyph)
            .filter(|glyph| *glyph < self.0.end.glyph)
            .chain(self.0.end.glyph.checked_sub(1))
    }
    /// Exact cluster checkpoints within one probe, without traversing other probes'
    /// checkpoints. Preserve the complete iterator's terminal anchor and duplicates.
    pub(super) fn anchor_glyphs_in(
        &self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = usize> + '_ {
        let end = range.end.min(self.0.end.glyph);
        let start = range.start.min(end);
        let first = self.0.points.partition_point(|point| point.glyph < start);
        let last = self.0.points.partition_point(|point| point.glyph < end);
        self.0.points[first..last]
            .iter()
            .map(|point| point.glyph)
            .chain(
                self.0
                    .end
                    .glyph
                    .checked_sub(1)
                    .filter(|glyph| range.contains(glyph)),
            )
    }
    /// The plain renderer's original grapheme-safe 512-byte run endings.
    pub fn text_run_boundaries(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.points.iter().skip(1).map(|point| point.byte)
    }
    pub fn is_text_run_boundary(&self, byte: usize) -> bool {
        self.0
            .points
            .binary_search_by_key(&byte, |point| point.byte)
            .is_ok()
    }
    /// Includes the terminal insertion point, matching visual_line_offsets.
    pub fn len(&self) -> usize {
        self.0.end.glyph + 1
    }
    pub fn is_empty(&self) -> bool {
        self.0.end.byte == 0
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Coordinates>()
            + self.0.points.capacity() * std::mem::size_of::<Position>()
    }
    /// Long source-monotonic paragraphs can be sliced before shaping. Bidi
    /// controls, reversed scripts and display breaks retain full-paragraph probes.
    pub fn source_paint_eligible(&self) -> bool {
        self.0.horizontal
    }
    pub fn horizontal_paint_bounds(&self, scroll: f64, width: f64) -> Option<std::ops::Range<f64>> {
        super::viewport::horizontal_paint_geometry(self.0.horizontal, scroll, width)
    }
    fn valid_body(&self, body: &str) -> bool {
        body.len() == self.0.end.byte
    }
    pub fn at(&self, body: &str, glyph: usize) -> Option<(usize, usize)> {
        if !self.valid_body(body) || glyph >= self.len() {
            return None;
        }
        let index = self
            .0
            .points
            .partition_point(|point| point.glyph <= glyph)
            .checked_sub(1)?;
        let point = self.0.points[index];
        let suffix = body.get(point.byte..)?;
        let byte = suffix
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain(std::iter::once(suffix.len()))
            .nth(glyph - point.glyph)?;
        Some((
            point.byte + byte,
            point.native + suffix[..byte].encode_utf16().count(),
        ))
    }
    pub fn index_at_byte(&self, body: &str, byte: usize) -> Option<usize> {
        if !self.valid_body(body) || byte > body.len() {
            return None;
        }
        let index = self
            .0
            .points
            .partition_point(|point| point.byte <= byte)
            .checked_sub(1)?;
        let point = self.0.points[index];
        let count = body
            .get(point.byte..)?
            .grapheme_indices(true)
            .take_while(|(at, _)| point.byte + at <= byte)
            .count();
        Some((point.glyph + count.saturating_sub(1)).min(self.0.end.glyph))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_anchor_ranges_match_complete_anchor_filtering() {
        for body in [
            String::new(),
            "x".into(),
            "x".repeat(513),
            "word 文😀e\u{301} ".repeat(10000),
            format!("{}e{}tail", "x".repeat(513), "\u{301}".repeat(1000)),
            "x".repeat(crate::editor::MAX_EDITOR_LINE_BYTES),
        ] {
            let index = VisualLineIndex::new(&body).unwrap();
            let anchors = index.anchor_glyphs().collect::<Vec<_>>();
            let mut boundaries = anchors.clone();
            boundaries.extend([0, 1, index.len() - 1, index.len(), usize::MAX]);
            for anchor in anchors.iter().step_by(127) {
                boundaries.extend([anchor.saturating_sub(1), anchor + 1]);
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            for (at, start) in boundaries.iter().copied().enumerate() {
                for end in boundaries.iter().copied().skip(at).step_by(31) {
                    let range = start..end;
                    assert_eq!(
                        index.anchor_glyphs_in(range.clone()).collect::<Vec<_>>(),
                        anchors
                            .iter()
                            .copied()
                            .filter(|glyph| range.contains(glyph))
                            .collect::<Vec<_>>()
                    );
                }
                assert_eq!(index.anchor_glyphs_in(start..start).count(), 0);
            }
            assert_eq!(
                index
                    .anchor_glyphs_in(std::ops::Range {
                        start: usize::MAX,
                        end: 0
                    })
                    .count(),
                0
            );
        }
    }
    #[test]
    fn sparse_glyph_queries_match_complete_cluster_coordinates() {
        for body in [
            "文😀e\u{301}\t words ".repeat(7000),
            "🇺🇸🇫🇷🇦🇺👩‍👩‍👧‍👦क्ष abc ".repeat(3000),
            format!("{}e{}tail", "x".repeat(513), "\u{301}".repeat(1000)),
            "\r literal ".repeat(7000),
            String::new(),
        ] {
            let index = VisualLineIndex::new(&body).unwrap();
            let expected = crate::editor::visual_line_offsets(&body).unwrap();
            let run_starts = crate::editor::visual_text_run_ranges(&body)
                .map(|run| run.start)
                .chain(std::iter::once(body.len()))
                .collect::<Vec<_>>();
            for byte in 0..=body.len() {
                assert_eq!(
                    index.is_text_run_boundary(byte),
                    run_starts.binary_search(&byte).is_ok()
                );
            }
            assert_eq!(index.len(), expected.len());
            for (glyph, &(byte, native)) in expected.iter().enumerate().step_by(31) {
                assert_eq!(index.at(&body, glyph), Some((byte, native)));
                assert_eq!(index.index_at_byte(&body, byte), Some(glyph));
                if let Some(&(next, _)) = expected.get(glyph + 1) {
                    for within in byte..next {
                        assert_eq!(index.index_at_byte(&body, within), Some(glyph));
                    }
                }
            }
            assert_eq!(
                index.at(&body, expected.len() - 1),
                expected.last().copied()
            );
            assert_eq!(
                index.index_at_byte(&body, body.len()),
                Some(expected.len() - 1)
            );
            assert!(index.at(&body, expected.len()).is_none());
            assert!(index.at("mismatched length", 0).is_none());
            assert_eq!(
                index.horizontal_paint_bounds(1000.0, 400.0),
                crate::editor::horizontal_paint_bounds(&body, 1000.0, 400.0)
            );
        }
        let ascii = "x".repeat(crate::editor::MAX_EDITOR_LINE_BYTES);
        let index = VisualLineIndex::new(&ascii).unwrap();
        assert_eq!(index.len(), ascii.len() + 1);
        assert!(index.retained_bytes() < 128 * 1024);
        assert!(
            VisualLineIndex::new(&"x".repeat(crate::editor::MAX_STRUCTURE_BYTES + 1)).is_none()
        );
    }
}
