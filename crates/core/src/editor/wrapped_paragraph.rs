//! Exact continuation policy for bounded, source-monotonic wrapped paragraphs.
use super::{
    GlyphRectangle, MAX_MEASURE_BYTES, MAX_PARAGRAPH_PROBE_BYTES, MAX_ROW_GEOMETRY_ANCHORS,
    ParagraphProbe, VisualLineIndex, WrappedGeometry,
};
use std::{ops::Range, sync::Arc};
const TOLERANCE: f64 = 0.25;
const CONTEXT_BYTES: usize = 1024;

/// Continuations start after complete words and retain original paint-run clipping.
/// The adapter measures styled text with the exact preceding horizontal gap;
/// dense overlap must reconnect before any completed geometry can publish.
/// Unsupported seams (including an oversized unbroken word) require full layout.
pub struct WrappedParagraphPreparation<'a> {
    body: &'a str,
    index: VisualLineIndex,
    runs: Arc<[usize]>,
    probe: ParagraphProbe,
    next: Option<(usize, usize)>,
    expected: Vec<GlyphRectangle>,
    anchors: Vec<GlyphRectangle>,
    width: Option<f64>,
    scroll_width: f64,
    height: Option<f64>,
    complete: bool,
}
impl<'a> WrappedParagraphPreparation<'a> {
    pub fn new(body: &'a str, index: VisualLineIndex, runs: Arc<[usize]>) -> Option<Self> {
        if body.len() <= MAX_MEASURE_BYTES
            || !index.source_paint_eligible()
            || runs.last().copied() != Some(body.len())
            || runs.len() > super::MAX_VISUAL_CARETS
            || runs.windows(2).any(|pair| pair[0] >= pair[1])
            || runs.iter().any(|byte| !body.is_char_boundary(*byte))
            || index
                .anchor_glyphs()
                .take(MAX_ROW_GEOMETRY_ANCHORS + 1)
                .count()
                > MAX_ROW_GEOMETRY_ANCHORS
        {
            return None;
        }
        let mut result = Self {
            body,
            index,
            runs,
            probe: ParagraphProbe {
                bytes: 0..0,
                native_start: 0,
                glyph_start: 0,
                origin: 0.0,
            },
            next: None,
            expected: Vec::new(),
            anchors: Vec::new(),
            width: None,
            scroll_width: 0.0,
            height: None,
            complete: false,
        };
        result.probe = result.probe_at(0, 0.0)?;
        result.next = result.continuation(&result.probe)?;
        Some(result)
    }
    fn boundary(&self, range: Range<usize>) -> Option<usize> {
        self.body
            .as_bytes()
            .get(range.clone())?
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, byte)| **byte == b' ')
            .map(|(offset, _)| range.start + offset + 1)
            .find(|&byte| {
                self.index
                    .index_at_byte(self.body, byte)
                    .and_then(|glyph| self.index.at(self.body, glyph))
                    .is_some_and(|(at, _)| at == byte)
            })
    }
    pub fn paint_runs(&self) -> Arc<[usize]> {
        self.runs.clone()
    }
    fn probe_at(&self, start: usize, origin: f64) -> Option<ParagraphProbe> {
        let limit = (start + MAX_PARAGRAPH_PROBE_BYTES).min(self.body.len());
        let end = if limit == self.body.len() {
            limit
        } else {
            self.boundary(start..limit)?
        };
        let glyph_start = self.index.index_at_byte(self.body, start)?;
        let (at, native_start) = self.index.at(self.body, glyph_start)?;
        (at == start && end > start).then_some(ParagraphProbe {
            bytes: start..end,
            native_start,
            glyph_start,
            origin,
        })
    }
    fn continuation(&self, probe: &ParagraphProbe) -> Option<Option<(usize, usize)>> {
        if probe.bytes.end == self.body.len() {
            return Some(None);
        }
        let start = self.boundary(
            probe.bytes.start + CONTEXT_BYTES..probe.bytes.end.checked_sub(CONTEXT_BYTES)?,
        )?;
        if probe.bytes.end - start > 4 * CONTEXT_BYTES {
            return None;
        }
        let end_glyph = self
            .index
            .index_at_byte(self.body, probe.bytes.end.checked_sub(CONTEXT_BYTES / 2)?)?;
        let proof_end = self.index.at(self.body, end_glyph)?.0;
        if proof_end <= start {
            return None;
        }
        Some(Some((start, proof_end)))
    }
    pub fn probe(&self) -> Option<&ParagraphProbe> {
        (!self.complete).then_some(&self.probe)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn expected_overlap(&self) -> &[GlyphRectangle] {
        &self.expected
    }
    pub fn targets(&self) -> Option<Vec<usize>> {
        let end = self.index.index_at_byte(self.body, self.probe.bytes.end)?;
        let mut targets = self
            .index
            .anchor_glyphs_in(self.probe.glyph_start..end)
            .collect::<Vec<_>>();
        targets.extend(self.expected.iter().map(|rect| rect.glyph));
        if let Some((start, end)) = self.next {
            targets.extend(
                self.index.index_at_byte(self.body, start)?
                    ..self.index.index_at_byte(self.body, end)?,
            );
        }
        targets.sort_unstable();
        targets.dedup();
        Some(targets)
    }
    /// Raw rectangles are relative to the styled probe. Vertical translation is
    /// measured from the same first overlap glyph, never from estimated line heights.
    pub fn record(
        &mut self,
        width: f64,
        height: f64,
        scroll_width: f64,
        rectangles: &[GlyphRectangle],
    ) -> bool {
        if self.complete
            || !width.is_finite()
            || width <= 0.0
            || width > 1_000_000_000.0
            || !height.is_finite()
            || height <= 0.0
            || height > 1_000_000.0
            || !scroll_width.is_finite()
            || scroll_width < width
            || scroll_width > 1_000_000_000.0
            || self
                .width
                .is_some_and(|old| (old - width).abs() > TOLERANCE)
        {
            return false;
        }
        let Some(targets) = self.targets() else {
            return false;
        };
        if targets.len() != rectangles.len()
            || targets
                .iter()
                .zip(rectangles)
                .any(|(glyph, rect)| *glyph != rect.glyph || !rect.valid())
        {
            return false;
        }
        let lookup = |glyph| {
            rectangles
                .binary_search_by_key(&glyph, |rect| rect.glyph)
                .ok()
                .map(|at| rectangles[at])
        };
        let shift = if let Some(first) = self.expected.first() {
            let Some(current) = lookup(first.glyph) else {
                return false;
            };
            first.top - current.top
        } else {
            0.0
        };
        if self.expected.iter().any(|old| {
            lookup(old.glyph).is_none_or(|current| {
                (old.left - current.left).abs() > TOLERANCE
                    || (old.top - current.top - shift).abs() > TOLERANCE
                    || (old.width - current.width).abs() > TOLERANCE
                    || (old.height - current.height).abs() > TOLERANCE
            })
        }) {
            return false;
        }
        let global = rectangles
            .iter()
            .map(|rect| GlyphRectangle {
                top: rect.top + shift,
                ..*rect
            })
            .collect::<Vec<_>>();
        if global.iter().any(|rect| !rect.valid()) {
            return false;
        }
        let lookup = |glyph| {
            global
                .binary_search_by_key(&glyph, |rect| rect.glyph)
                .ok()
                .map(|at| global[at])
        };
        let next = if let Some((start, proof_end)) = self.next {
            let Some(glyph) = self.index.index_at_byte(self.body, start) else {
                return false;
            };
            let Some(origin) = lookup(glyph).map(|rect| rect.left) else {
                return false;
            };
            let Some(probe) = self.probe_at(start, origin) else {
                return false;
            };
            let Some(continuation) = self.continuation(&probe) else {
                return false;
            };
            let Some(end) = self.index.index_at_byte(self.body, proof_end) else {
                return false;
            };
            Some((
                probe,
                continuation,
                global
                    .iter()
                    .copied()
                    .filter(|rect| (glyph..end).contains(&rect.glyph))
                    .collect::<Vec<_>>(),
                glyph,
            ))
        } else {
            None
        };
        let commit_end = next
            .as_ref()
            .map_or(self.index.len() - 1, |(_, _, _, glyph)| *glyph);
        for glyph in self
            .index
            .anchor_glyphs_in(self.probe.glyph_start..commit_end)
        {
            let Some(rect) = lookup(glyph) else {
                return false;
            };
            if self.anchors.last().is_none_or(|last| last.glyph != glyph) {
                self.anchors.push(rect);
            }
        }
        self.width = Some(width);
        self.scroll_width = self.scroll_width.max(scroll_width);
        if let Some((probe, continuation, expected, _)) = next {
            self.probe = probe;
            self.next = continuation;
            self.expected = expected;
        } else {
            self.height = Some(height + shift);
            self.complete = true;
        }
        true
    }
    pub fn finish(self) -> Option<(f64, WrappedGeometry)> {
        if !self.complete {
            return None;
        }
        Some((
            self.scroll_width,
            WrappedGeometry::new(
                self.index.len() - 1,
                self.width?,
                self.height?,
                self.anchors,
            )?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(plan: &WrappedParagraphPreparation<'_>) -> (f64, Vec<GlyphRectangle>) {
        let probe = plan.probe().unwrap();
        let end = plan
            .index
            .index_at_byte(plan.body, probe.bytes.end)
            .unwrap();
        let offset = probe.origin / 7.0;
        let rectangles = plan
            .targets()
            .unwrap()
            .into_iter()
            .map(|glyph| {
                let position = offset + (glyph - probe.glyph_start) as f64;
                GlyphRectangle {
                    glyph,
                    left: (position % 40.0) * 7.0,
                    top: (position / 40.0).floor() * 15.0 + 2.0,
                    width: 7.0,
                    height: 15.0,
                }
            })
            .collect();
        (
            ((offset + (end - probe.glyph_start) as f64) / 40.0).ceil() * 15.0,
            rectangles,
        )
    }
    #[test]
    fn bounded_wrapped_continuations_preserve_complete_extents_and_every_anchor() {
        for body in ["word ".repeat(40_000), "word 文😀e\u{301} ".repeat(12_000)] {
            let index = VisualLineIndex::new(&body).unwrap();
            let runs = index.text_run_boundaries().collect::<Vec<_>>().into();
            let mut plan = WrappedParagraphPreparation::new(&body, index.clone(), runs).unwrap();
            let mut turns = 0;
            while plan.probe().is_some() {
                assert!(plan.probe().unwrap().bytes.len() <= MAX_PARAGRAPH_PROBE_BYTES);
                let (height, rectangles) = layout(&plan);
                assert!(plan.record(280.0, height, 280.0, &rectangles));
                turns += 1;
                assert!(turns < 100);
            }
            let (width, geometry) = plan.finish().unwrap();
            assert_eq!(width.to_bits(), 280.0_f64.to_bits());
            assert_eq!(
                geometry.height.to_bits(),
                (((index.len() - 1) as f64 / 40.0).ceil() * 15.0).to_bits()
            );
            let actual = geometry.anchors(0..index.len() - 1).unwrap();
            let mut targets = index.anchor_glyphs().collect::<Vec<_>>();
            targets.dedup();
            let expected = targets
                .into_iter()
                .map(|glyph| GlyphRectangle {
                    glyph,
                    left: (glyph % 40) as f64 * 7.0,
                    top: (glyph / 40) as f64 * 15.0 + 2.0,
                    width: 7.0,
                    height: 15.0,
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
            assert!(turns > 5);
        }
    }
    #[test]
    fn wrapped_overlap_failures_and_incomplete_jobs_cannot_publish() {
        let body = "word ".repeat(40_000);
        let index = VisualLineIndex::new(&body).unwrap();
        let runs: Arc<[usize]> = index.text_run_boundaries().collect();
        let mut plan =
            WrappedParagraphPreparation::new(&body, index.clone(), runs.clone()).unwrap();
        let (height, rectangles) = layout(&plan);
        assert!(plan.record(280.0, height, 280.0, &rectangles));
        let (height, mut changed) = layout(&plan);
        changed[0].left += 1.0;
        assert!(!plan.record(280.0, height, 280.0, &changed));
        assert!(plan.finish().is_none());
        assert!(
            WrappedParagraphPreparation::new(&body, index, runs)
                .unwrap()
                .finish()
                .is_none()
        );
        for body in ["x".repeat(100_000), "word العربية ".repeat(12_000)] {
            let index = VisualLineIndex::new(&body).unwrap();
            let runs = index.text_run_boundaries().collect();
            assert!(WrappedParagraphPreparation::new(&body, index, runs).is_none());
        }
    }
}
