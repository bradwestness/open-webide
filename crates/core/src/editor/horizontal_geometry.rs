//! Exact styled glyph anchors used to prepare source slices before browser layout.
use std::ops::Range;

pub const MAX_HORIZONTAL_ANCHORS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphRectangle {
    pub glyph: usize,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}
impl GlyphRectangle {
    fn valid(self) -> bool {
        [self.left, self.top, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.left.abs() <= 1_000_000_000.0
            && self.top.abs() <= 1_000_000.0
            && (0.0..=1_000_000_000.0).contains(&self.width)
            && self.height > 0.0
            && self.height <= 1_000_000.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HorizontalGeometry {
    anchors: Vec<GlyphRectangle>,
    glyphs: usize,
    pub width: f64,
    pub height: f64,
}
impl HorizontalGeometry {
    /// The adapter supplies rectangles relative to the exact styled logical row.
    /// Retain this only within an immutable source/font/layout scope.
    pub fn new(
        glyphs: usize,
        width: f64,
        height: f64,
        anchors: Vec<GlyphRectangle>,
    ) -> Option<Self> {
        if glyphs == 0
            || anchors.is_empty()
            || anchors.len() > MAX_HORIZONTAL_ANCHORS
            || anchors.capacity() > MAX_HORIZONTAL_ANCHORS
            || !width.is_finite()
            || !(0.0..=1_000_000_000.0).contains(&width)
            || !height.is_finite()
            || !(0.0..=1_000_000.0).contains(&height)
            || height == 0.0
            || anchors.first()?.glyph != 0
            || anchors.last()?.glyph != glyphs - 1
            || anchors.iter().any(|a| !a.valid() || a.glyph >= glyphs)
            || anchors.windows(2).any(|pair| {
                pair[0].glyph >= pair[1].glyph
                    || pair[0].left > pair[1].left
                    || pair[0].left + pair[0].width > pair[1].left + pair[1].width
            })
        {
            return None;
        }
        Some(Self {
            anchors,
            glyphs,
            width,
            height,
        })
    }
    /// Include a measured anchor before the window for shaping context and a
    /// complete anchor glyph after it for validation. The adapter can reject an
    /// exceptional source slice (for example an indivisible huge cluster).
    pub fn source_interval(&self, columns: Range<f64>) -> Option<Range<usize>> {
        if !columns.start.is_finite()
            || !columns.end.is_finite()
            || columns.start < 0.0
            || columns.end <= columns.start
        {
            return None;
        }
        let last = self.anchors.last()?;
        if columns.start > last.left + last.width {
            return Some(self.glyphs..self.glyphs);
        }
        let start = self
            .anchors
            .partition_point(|a| a.left + a.width < columns.start)
            .saturating_sub(1);
        let end = self
            .anchors
            .partition_point(|a| a.left < columns.end)
            .min(self.anchors.len() - 1);
        Some(self.anchors[start].glyph..self.anchors[end].glyph + 1)
    }
    pub fn anchors(&self, source: Range<usize>) -> Option<&[GlyphRectangle]> {
        if source.start > source.end || source.end > self.glyphs {
            return None;
        }
        let first = self.anchors.partition_point(|a| a.glyph < source.start);
        let last = self.anchors.partition_point(|a| a.glyph < source.end);
        Some(&self.anchors[first..last])
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.anchors.capacity() * std::mem::size_of::<GlyphRectangle>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn anchor(glyph: usize, left: f64) -> GlyphRectangle {
        GlyphRectangle {
            glyph,
            left,
            top: 2.0,
            width: 8.0,
            height: 15.0,
        }
    }
    #[test]
    fn anchored_windows_keep_source_context_validation_and_offscreen_gaps() {
        let geometry = HorizontalGeometry::new(
            1001,
            8010.0,
            20.0,
            vec![
                anchor(0, 0.0),
                anchor(250, 2000.0),
                anchor(500, 4000.0),
                anchor(750, 6000.0),
                anchor(1000, 8000.0),
            ],
        )
        .unwrap();
        assert_eq!(geometry.source_interval(2100.0..3900.0), Some(250..501));
        assert_eq!(
            geometry.anchors(250..501).unwrap(),
            &[anchor(250, 2000.0), anchor(500, 4000.0)]
        );
        assert_eq!(geometry.source_interval(0.0..400.0), Some(0..251));
        assert_eq!(geometry.source_interval(7990.0..9000.0), Some(750..1001));
        assert_eq!(geometry.source_interval(9000.0..10000.0), Some(1001..1001));
        assert_eq!(geometry.anchors(1001..1001).unwrap(), &[]);
        assert!(geometry.anchors(1001..1002).is_none());
        assert!(geometry.source_interval(f64::NAN..500.0).is_none());
        assert!(geometry.source_interval(100.0..10.0).is_none());
    }
    #[test]
    fn incomplete_nonmonotonic_oversized_or_invalid_measurements_require_fallback() {
        for points in [
            vec![],
            vec![anchor(1, 0.0)],
            vec![anchor(0, 0.0)],
            vec![anchor(0, 10.0), anchor(1, 0.0)],
            vec![anchor(0, 0.0), anchor(0, 1.0)],
            vec![anchor(0, 0.0), anchor(1, f64::NAN)],
            vec![anchor(0, 0.0), anchor(1, f64::INFINITY)],
        ] {
            assert!(HorizontalGeometry::new(2, 20.0, 20.0, points).is_none());
        }
        assert!(HorizontalGeometry::new(1, 10.0, f64::INFINITY, vec![anchor(0, 0.0)]).is_none());
        let points: Vec<_> = (0..MAX_HORIZONTAL_ANCHORS + 1)
            .map(|i| anchor(i, 0.0))
            .collect();
        assert!(HorizontalGeometry::new(points.len(), 10.0, 20.0, points).is_none());
        let mut reserved = Vec::with_capacity(MAX_HORIZONTAL_ANCHORS + 1);
        reserved.push(anchor(0, 0.0));
        assert!(HorizontalGeometry::new(1, 10.0, 20.0, reserved).is_none());
        // Styled glyph rectangles may overflow the logical CSS row box.
        assert!(
            HorizontalGeometry::new(2, 20.0, 20.0, vec![anchor(0, 0.0), anchor(1, 100.0)])
                .is_some()
        );
        // Ligature rectangles can overlap; only source-monotonic positions are required.
        assert!(
            HorizontalGeometry::new(2, 10.0, 20.0, vec![anchor(0, 0.0), anchor(1, 0.0)]).is_some()
        );
    }
}
