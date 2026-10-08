//! Source-owned continuation and overlap validation for bounded paragraph probes.
use super::{GlyphRectangle, HorizontalGeometry, MAX_MEASURE_BYTES, VisualLineIndex};
use std::{ops::Range, sync::Arc};

pub const MAX_PARAGRAPH_PROBE_BYTES: usize = 16 * 1024;
const CONTEXT_BYTES: usize = 1024;
const TOLERANCE: f64 = 0.25;

#[derive(Clone, Debug)]
pub struct ParagraphProbe {
    pub bytes: Range<usize>,
    pub native_start: usize,
    pub glyph_start: usize,
    pub origin: f64,
}

/// Exact completed probes, retained with the facade's immutable source/style scope.
#[derive(Clone, Debug)]
pub struct ParagraphMeasurements {
    runs: Arc<[usize]>,
    records: Vec<ParagraphMeasurement>,
}
impl ParagraphMeasurements {
    /// Original paint-run start at or before a source byte. The caller retains
    /// the exact source/style scope that supplied these boundaries.
    pub fn paint_run_start(&self, byte: usize) -> Option<usize> {
        if byte > *self.runs.last()? {
            return None;
        }
        let end = self.runs.partition_point(|end| *end <= byte);
        Some(end.checked_sub(1).map_or(0, |index| self.runs[index]))
    }
}
#[derive(Clone, Debug)]
struct ParagraphMeasurement {
    bytes: Range<usize>,
    width: f64,
    height: f64,
    scroll_width: f64,
    rectangles: Arc<[GlyphRectangle]>,
}
const MAX_RETAINED_RECTANGLES: usize = 128 * 1024;

/// Adapters measure real styled text; this policy never estimates character
/// widths. A failed overlap, unavailable paint boundary or stale source requires
/// the caller's complete-paragraph fallback.
pub struct ParagraphMeasurementPlan<'a> {
    body: &'a str,
    index: VisualLineIndex,
    probe: ParagraphProbe,
    continuation: Option<(usize, usize)>,
    expected: Vec<GlyphRectangle>,
    anchors: Vec<GlyphRectangle>,
    dimensions: Option<(f64, f64)>,
    scroll_width: f64,
    finished: bool,
    has_tabs: bool,
    local_origin: f64,
    retried: bool,
    runs: Arc<[usize]>,
    records: Vec<ParagraphMeasurement>,
    retained_rectangles: usize,
}
impl<'a> ParagraphMeasurementPlan<'a> {
    pub fn new(body: &'a str, index: VisualLineIndex) -> Option<Self> {
        let runs = index.text_run_boundaries().collect();
        Self::with_run_boundaries(body, index, runs)
    }
    pub fn with_run_boundaries(
        body: &'a str,
        index: VisualLineIndex,
        runs: Vec<usize>,
    ) -> Option<Self> {
        Self::with_shared_run_boundaries(body, index, runs.into())
    }
    /// Original paint boundaries can be shared with source-owned viewport paint
    /// before any dimensions or glyph measurements have been completed.
    pub fn with_shared_run_boundaries(
        body: &'a str,
        index: VisualLineIndex,
        runs: Arc<[usize]>,
    ) -> Option<Self> {
        if body.len() <= MAX_MEASURE_BYTES
            || !index.source_paint_eligible()
            || index
                .anchor_glyphs()
                .take(super::MAX_ROW_GEOMETRY_ANCHORS + 1)
                .count()
                > super::MAX_ROW_GEOMETRY_ANCHORS
        {
            return None;
        }
        if runs.last().copied() != Some(body.len())
            || runs.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return None;
        }
        let probe = Self::probe_at(body, &index, &runs, 0, 0.0)?;
        let continuation = Self::continuation(body, &index, &runs, &probe)?;
        Some(Self {
            body,
            index,
            probe,
            continuation,
            expected: Vec::new(),
            anchors: Vec::new(),
            dimensions: None,
            scroll_width: 0.0,
            finished: false,
            has_tabs: body.as_bytes().contains(&b'\t'),
            local_origin: 0.0,
            retried: false,
            runs,
            records: Vec::new(),
            retained_rectangles: 0,
        })
    }
    /// Replay only complete unchanged source/paint runs. The caller proves the
    /// style/environment prefix; normal record validation still checks every
    /// target and overlap. The first changed probe is always freshly measured.
    pub fn reuse_prefix(
        &mut self,
        old_body: &str,
        old: &ParagraphMeasurements,
        style_end: usize,
    ) -> usize {
        let mut reused = 0;
        let mut validated_runs = 0;
        for record in &old.records {
            let end = record.bytes.end;
            let Some(probe) = self.probe() else { break };
            if record.bytes != probe.bytes
                || end >= self.body.len()
                || end > style_end
                || old_body.get(record.bytes.clone()) != self.body.get(record.bytes.clone())
            {
                break;
            }
            let old_end = old.runs.partition_point(|byte| *byte <= end);
            let new_end = self.runs.partition_point(|byte| *byte <= end);
            if old_end != new_end
                || old
                    .runs
                    .get(validated_runs..old_end)
                    .zip(self.runs.get(validated_runs..new_end))
                    .is_none_or(|(old, new)| old != new)
                || !self.record_measurement(
                    record.width,
                    record.height,
                    record.scroll_width,
                    &record.rectangles,
                    Some(&record.rectangles),
                )
            {
                break;
            }
            // Immutable run tables keep the previously checked prefix valid.
            validated_runs = old_end;
            reused += 1;
        }
        reused
    }
    /// Replay unchanged probes only after an exact measured reconnection. No
    /// translation is admitted: source/run offsets, glyph targets, dimensions,
    /// origin and every incoming overlap rectangle must match the retained probe.
    pub fn reuse_suffix(
        &mut self,
        old_body: &str,
        old: &ParagraphMeasurements,
        style_start: usize,
    ) -> usize {
        if old_body.len() != self.body.len() || self.expected.is_empty() {
            return 0;
        }
        let mut reused = 0;
        while let Some(probe) = self.probe().cloned() {
            if probe.bytes.start < style_start {
                break;
            }
            let Some(record) = old
                .records
                .binary_search_by_key(&probe.bytes.start, |record| record.bytes.start)
                .ok()
                .map(|index| &old.records[index])
                .filter(|record| record.bytes == probe.bytes)
            else {
                break;
            };
            if self.dimensions != Some((record.width, record.height))
                || old_body.get(probe.bytes.clone()) != self.body.get(probe.bytes.clone())
                || record
                    .rectangles
                    .iter()
                    .find(|rect| rect.glyph == probe.glyph_start)
                    .is_none_or(|rect| rect.left.to_bits() != probe.origin.to_bits())
                || self.expected.iter().any(|expected| {
                    record
                        .rectangles
                        .binary_search_by_key(&expected.glyph, |rect| rect.glyph)
                        .ok()
                        .is_none_or(|index| record.rectangles[index] != *expected)
                })
            {
                break;
            }
            let run_slice = |runs: &[usize]| {
                let start = runs.partition_point(|byte| *byte <= probe.bytes.start);
                let end = runs.partition_point(|byte| *byte <= probe.bytes.end);
                start..end
            };
            if old.runs[run_slice(&old.runs)] != self.runs[run_slice(&self.runs)]
                || !self.record_measurement(
                    record.width,
                    record.height,
                    record.scroll_width,
                    &record.rectangles,
                    Some(&record.rectangles),
                )
            {
                break;
            }
            reused += 1;
        }
        reused
    }
    fn boundary(
        body: &str,
        index: &VisualLineIndex,
        runs: &[usize],
        start: usize,
        end: usize,
    ) -> Option<usize> {
        let last = runs.partition_point(|byte| *byte <= end);
        runs[..last]
            .iter()
            .rev()
            .take_while(|byte| **byte > start)
            .find_map(|byte| {
                let byte = *byte;
                let glyph = index.index_at_byte(body, byte)?;
                (index.at(body, glyph)?.0 == byte).then_some(byte)
            })
    }
    fn probe_at(
        body: &str,
        index: &VisualLineIndex,
        runs: &[usize],
        start: usize,
        origin: f64,
    ) -> Option<ParagraphProbe> {
        let limit = (start + MAX_PARAGRAPH_PROBE_BYTES).min(body.len());
        let end = if limit == body.len() {
            limit
        } else {
            Self::boundary(body, index, runs, start, limit)?
        };
        let glyph_start = index.index_at_byte(body, start)?;
        let (byte, native_start) = index.at(body, glyph_start)?;
        (byte == start && end > start).then_some(ParagraphProbe {
            bytes: start..end,
            native_start,
            glyph_start,
            origin,
        })
    }
    fn continuation(
        body: &str,
        index: &VisualLineIndex,
        runs: &[usize],
        probe: &ParagraphProbe,
    ) -> Option<Option<(usize, usize)>> {
        if probe.bytes.end == body.len() {
            return Some(None);
        }
        let end = probe.bytes.end;
        let start = Self::boundary(
            body,
            index,
            runs,
            probe.bytes.start + CONTEXT_BYTES,
            end.checked_sub(CONTEXT_BYTES)?,
        )?;
        let proof_end = Self::boundary(
            body,
            index,
            runs,
            start,
            end.checked_sub(CONTEXT_BYTES / 2)?,
        )?;
        (proof_end > start).then_some(Some((start, proof_end)))
    }
    pub fn probe(&self) -> Option<&ParagraphProbe> {
        (!self.finished).then_some(&self.probe)
    }
    pub fn local_origin(&self) -> f64 {
        self.local_origin
    }
    pub fn retry_origin(&mut self, rectangles: &[GlyphRectangle]) -> bool {
        if self.finished || self.retried {
            return false;
        }
        let Some(adjustment) = self.tab_origin_adjustment(rectangles) else {
            return false;
        };
        let phase = self.local_origin + adjustment;
        if !phase.is_finite() || !(0.0..=4096.0).contains(&phase) {
            return false;
        }
        self.local_origin = phase;
        self.retried = true;
        true
    }
    /// Fit the tab grid from actual overlap measurements. Font fallback can
    /// change a tab's space advance, so a standalone tab cannot define the grid
    /// for a mixed-font paragraph. Only a first discrepancy at a tab admits one
    /// retry; the complete overlap still has to pass normal validation.
    fn tab_origin_adjustment(&self, rectangles: &[GlyphRectangle]) -> Option<f64> {
        for old in &self.expected {
            let new = rectangles.get(
                rectangles
                    .binary_search_by_key(&old.glyph, |rect| rect.glyph)
                    .ok()?,
            )?;
            let same_position = (old.left - new.left).abs() <= TOLERANCE
                && (old.top - new.top).abs() <= TOLERANCE
                && (old.height - new.height).abs() <= TOLERANCE;
            let difference = new.width - old.width;
            if !same_position {
                return None;
            }
            if difference.abs() > TOLERANCE {
                let byte = self.index.at(self.body, old.glyph)?.0;
                return (self.body.as_bytes().get(byte) == Some(&b'\t') && difference.is_finite())
                    .then_some(difference);
            }
        }
        None
    }
    /// Sparse document anchors plus every overlap glyph, in exact source order.
    pub fn targets(&self) -> Option<Vec<usize>> {
        if self.finished {
            return None;
        }
        let commit_end = match self.continuation {
            Some((byte, _)) => self.index.index_at_byte(self.body, byte)?,
            None => self.index.len() - 1,
        };
        let mut targets = self
            .index
            .anchor_glyphs_in(self.probe.glyph_start..commit_end)
            .collect::<Vec<_>>();
        targets.extend(self.expected.iter().map(|rect| rect.glyph));
        if let Some((start, end)) = self.continuation {
            targets.extend(
                self.index.index_at_byte(self.body, start)?
                    ..self.index.index_at_byte(self.body, end)?,
            );
        }
        targets.sort_unstable();
        targets.dedup();
        Some(targets)
    }
    /// Rectangles already include the probe's measured horizontal origin.
    pub fn record(
        &mut self,
        width: f64,
        height: f64,
        scroll_width: f64,
        rectangles: &[GlyphRectangle],
    ) -> bool {
        self.record_measurement(width, height, scroll_width, rectangles, None)
    }

    fn record_measurement(
        &mut self,
        width: f64,
        height: f64,
        scroll_width: f64,
        rectangles: &[GlyphRectangle],
        retained: Option<&Arc<[GlyphRectangle]>>,
    ) -> bool {
        if self.finished
            || !width.is_finite()
            || !height.is_finite()
            || !scroll_width.is_finite()
            || width <= 0.0
            || height <= 0.0
            || scroll_width < width
            || width > 1_000_000_000.0
            || height > 1_000_000.0
            || scroll_width > 1_000_000_000.0
            || self.dimensions.is_some_and(|(old_width, old_height)| {
                (old_width - width).abs() > TOLERANCE || (old_height - height).abs() > TOLERANCE
            })
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
        if self.expected.iter().any(|old| {
            lookup(old.glyph).is_none_or(|new| {
                (old.left - new.left).abs() > TOLERANCE
                    || (old.top - new.top).abs() > TOLERANCE
                    || (old.width - new.width).abs() > TOLERANCE
                    || (old.height - new.height).abs() > TOLERANCE
            })
        }) {
            return false;
        }
        let next = if let Some((byte, proof_end)) = self.continuation {
            let Some(glyph) = self.index.index_at_byte(self.body, byte) else {
                return false;
            };
            let Some(origin) = lookup(glyph).map(|rect| rect.left) else {
                return false;
            };
            let Some(probe) = Self::probe_at(self.body, &self.index, &self.runs, byte, origin)
            else {
                return false;
            };
            let Some(continuation) = Self::continuation(self.body, &self.index, &self.runs, &probe)
            else {
                return false;
            };
            let Some(end_glyph) = self.index.index_at_byte(self.body, proof_end) else {
                return false;
            };
            Some((
                probe,
                continuation,
                rectangles
                    .iter()
                    .copied()
                    .filter(|rect| (glyph..end_glyph).contains(&rect.glyph))
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
        self.retained_rectangles = self.retained_rectangles.saturating_add(rectangles.len());
        if self.retained_rectangles <= MAX_RETAINED_RECTANGLES {
            self.records.push(ParagraphMeasurement {
                bytes: self.probe.bytes.clone(),
                width,
                height,
                scroll_width,
                rectangles: retained.map_or_else(|| rectangles.into(), Arc::clone),
            });
        } else {
            // Bound retention without changing measurement or fallback behavior.
            self.records.clear();
        }
        self.dimensions = Some((width, height));
        self.scroll_width = self.scroll_width.max(scroll_width);
        if let Some((probe, continuation, expected, _)) = next {
            self.probe = probe;
            self.continuation = continuation;
            self.expected = expected;
            // A small measured-DOM margin permits tab corrections in either
            // direction without enormous global browser coordinates.
            self.local_origin =
                self.probe.origin.fract() + if self.has_tabs { 1024.0 } else { 0.0 };
            self.retried = false;
        } else {
            self.finished = true;
        }
        true
    }
    pub fn finish(self) -> Option<(f64, HorizontalGeometry)> {
        self.finish_with_measurements()
            .map(|(width, geometry, _)| (width, geometry))
    }
    pub fn finish_with_measurements(
        self,
    ) -> Option<(f64, HorizontalGeometry, ParagraphMeasurements)> {
        if !self.finished {
            return None;
        }
        let (width, height) = self.dimensions?;
        Some((
            self.scroll_width,
            HorizontalGeometry::new(self.index.len() - 1, width, height, self.anchors)?,
            ParagraphMeasurements {
                runs: self.runs,
                records: self.records,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangles(plan: &ParagraphMeasurementPlan<'_>) -> Vec<GlyphRectangle> {
        plan.targets()
            .unwrap()
            .into_iter()
            .map(|glyph| GlyphRectangle {
                glyph,
                left: f64::from(u32::try_from(glyph).unwrap()) * 7.0,
                top: 0.0,
                width: 7.0,
                height: 15.0,
            })
            .collect()
    }

    fn measured(source: &str) -> (f64, HorizontalGeometry, ParagraphMeasurements) {
        let mut plan =
            ParagraphMeasurementPlan::new(source, VisualLineIndex::new(source).unwrap()).unwrap();
        while let Some(probe) = plan.probe().cloned() {
            let end = plan.index.index_at_byte(source, probe.bytes.end).unwrap();
            let rects = rectangles(&plan);
            assert!(plan.record(
                244.0,
                15.0,
                (f64::from(u32::try_from(end).unwrap()) * 7.0).max(244.0),
                &rects
            ));
        }
        plan.finish_with_measurements().unwrap()
    }
    #[test]
    fn completed_geometry_keeps_shared_original_run_boundaries() {
        let body = "word 文😀 ".repeat(8000);
        let index = VisualLineIndex::new(&body).unwrap();
        let runs: Arc<[usize]> = index.text_run_boundaries().collect::<Vec<_>>().into();
        let mut plan =
            ParagraphMeasurementPlan::with_shared_run_boundaries(&body, index, runs.clone())
                .unwrap();
        assert!(Arc::ptr_eq(&plan.runs, &runs));
        while let Some(probe) = plan.probe().cloned() {
            let end = plan.index.index_at_byte(&body, probe.bytes.end).unwrap();
            let rects = rectangles(&plan);
            assert!(plan.record(
                244.0,
                15.0,
                (f64::from(u32::try_from(end).unwrap()) * 7.0).max(244.0),
                &rects,
            ));
        }
        let (_, _, completed) = plan.finish_with_measurements().unwrap();
        assert!(Arc::ptr_eq(&completed.runs, &runs));
    }

    #[test]
    fn retained_paint_runs_locate_original_unicode_boundaries() {
        let source = format!(
            "{}e{}tail",
            "文😀e\u{301} words ".repeat(5000),
            "\u{301}".repeat(600)
        );
        let (_, _, measurements) = measured(&source);
        for run in super::super::visual_text_run_ranges(&source) {
            assert_eq!(measurements.paint_run_start(run.start), Some(run.start));
            assert_eq!(measurements.paint_run_start(run.end - 1), Some(run.start));
            assert!(source.is_char_boundary(run.start));
        }
        assert_eq!(
            measurements.paint_run_start(source.len()),
            Some(source.len())
        );
        assert_eq!(measurements.paint_run_start(source.len() + 1), None);
    }
    #[test]
    fn changed_paragraph_prefix_reuses_exact_probes_and_finishes_with_fresh_overlaps() {
        let old = "文😀 words ".repeat(12000);
        let (_, _, retained) = measured(&old);
        let at = old.len() * 3 / 4;
        let at = old.floor_char_boundary(at);
        let changed = format!("{}fresh {}", &old[..at], &old[at..]);
        let mut plan =
            ParagraphMeasurementPlan::new(&changed, VisualLineIndex::new(&changed).unwrap())
                .unwrap();
        let reused = plan.reuse_prefix(&old, &retained, old.len());
        assert!(reused > 1);
        assert!(plan.probe().unwrap().bytes.start < at);
        assert!(plan.probe().unwrap().bytes.end > at);
        let before = plan.probe().unwrap().bytes.clone();
        let mut wrong = rectangles(&plan);
        wrong[0].left += 1.0;
        assert!(!plan.record(244.0, 15.0, 1_000_000.0, &wrong));
        assert_eq!(plan.probe().unwrap().bytes, before);
        while let Some(probe) = plan.probe().cloned() {
            let end = plan.index.index_at_byte(&changed, probe.bytes.end).unwrap();
            let rects = rectangles(&plan);
            assert!(plan.record(
                244.0,
                15.0,
                (f64::from(u32::try_from(end).unwrap()) * 7.0).max(244.0),
                &rects
            ));
        }
        let (width, geometry, logs) = plan.finish_with_measurements().unwrap();
        let (fresh_width, fresh, _) = measured(&changed);
        assert_eq!(width.to_bits(), fresh_width.to_bits());
        assert_eq!(geometry, fresh);
        assert!(Arc::ptr_eq(
            &logs.records[0].rectangles,
            &retained.records[0].rectangles
        ));
    }
    #[test]
    fn retained_prefix_stops_at_a_later_changed_paint_boundary() {
        let source = "word space ".repeat(10000);
        let (_, _, retained) = measured(&source);
        for insert in [true, false] {
            let boundary = retained.records[1].bytes.end + 1;
            let mut runs = retained.runs.to_vec();
            let at = runs.binary_search(&boundary).unwrap_err();
            let changed = if insert {
                runs.insert(at, boundary);
                boundary
            } else {
                runs[at] += 1;
                runs[at]
            };
            let mut plan = ParagraphMeasurementPlan::with_run_boundaries(
                &source,
                VisualLineIndex::new(&source).unwrap(),
                runs,
            )
            .unwrap();
            assert_eq!(plan.reuse_prefix(&source, &retained, source.len()), 2);
            assert!(plan.probe().unwrap().bytes.contains(&changed));
            assert_eq!(plan.records.len(), 2);
            for (current, old) in plan.records.iter().zip(&retained.records) {
                assert!(Arc::ptr_eq(&current.rectangles, &old.rectangles));
            }
        }
    }

    #[test]
    fn paragraph_reuse_rejects_changed_style_runs_source_and_dimensions() {
        let old = "word space ".repeat(10000);
        let (_, _, retained) = measured(&old);
        let index = VisualLineIndex::new(&old).unwrap();
        let mut plan = ParagraphMeasurementPlan::new(&old, index.clone()).unwrap();
        assert_eq!(plan.reuse_prefix(&old, &retained, 0), 0);
        let mut runs = retained.runs.to_vec();
        runs.insert(0, 1);
        let mut plan =
            ParagraphMeasurementPlan::with_run_boundaries(&old, index.clone(), runs).unwrap();
        assert_eq!(plan.reuse_prefix(&old, &retained, old.len()), 0);
        let changed = format!("changed{old}");
        let mut plan =
            ParagraphMeasurementPlan::new(&changed, VisualLineIndex::new(&changed).unwrap())
                .unwrap();
        assert_eq!(plan.reuse_prefix(&old, &retained, old.len()), 0);
        let mut bad = retained.clone();
        bad.records[0].width = f64::NAN;
        let mut plan = ParagraphMeasurementPlan::new(&old, index).unwrap();
        assert_eq!(plan.reuse_prefix(&old, &bad, old.len()), 0);
        assert_eq!(plan.probe().unwrap().bytes.start, 0);
    }

    #[test]
    fn unchanged_suffix_requires_exact_measured_reconnection() {
        let old = format!("a{}", "文😀 words ".repeat(12000));
        let (old_width, geometry, retained) = measured(&old);
        let changed = format!("z{}", &old[1..]);
        let make = || {
            ParagraphMeasurementPlan::new(&changed, VisualLineIndex::new(&changed).unwrap())
                .unwrap()
        };
        let mut plan = make();
        assert_eq!(plan.reuse_suffix(&old, &retained, 1), 0);
        let probe = plan.probe().unwrap().clone();
        let end = plan.index.index_at_byte(&changed, probe.bytes.end).unwrap();
        let rects = rectangles(&plan);
        let width = (f64::from(u32::try_from(end).unwrap()) * 7.0).max(244.0);
        assert!(plan.record(244.0, 15.0, width, &rects));
        assert!(plan.reuse_suffix(&old, &retained, 1) > 2);
        let (next_width, next, reused) = plan.finish_with_measurements().unwrap();
        assert_eq!(next_width.to_bits(), old_width.to_bits());
        assert_eq!(next, geometry);
        assert!(Arc::ptr_eq(
            &reused.records[1].rectangles,
            &retained.records[1].rectangles
        ));
        for fractional_shift in [0.125, 1.0] {
            let mut plan = make();
            let mut moved = rects.clone();
            for rect in &mut moved {
                rect.left += fractional_shift;
            }
            assert!(plan.record(244.0, 15.0, width, &moved));
            assert_eq!(plan.reuse_suffix(&old, &retained, 1), 0);
            assert!(plan.probe().is_some());
        }
        let mut plan = make();
        assert!(plan.record(244.0, 15.0, width, &rects));
        assert_eq!(plan.reuse_suffix(&old, &retained, changed.len()), 0);
        let different = format!("{changed}x");
        let mut plan =
            ParagraphMeasurementPlan::new(&different, VisualLineIndex::new(&different).unwrap())
                .unwrap();
        assert!(plan.record(244.0, 15.0, width, &rects));
        assert_eq!(plan.reuse_suffix(&old, &retained, 0), 0);
    }

    #[test]
    fn probes_cover_admitted_source_and_preserve_complete_coordinates() {
        for source in [
            "word space ".repeat(95_000),
            "文😀e\u{301}\t words ".repeat(40_000),
            "x \u{301}word safe 🇺🇸👩‍👩‍👧‍👦 ".repeat(5000),
        ] {
            let index = VisualLineIndex::new(&source).unwrap();
            let mut plan = ParagraphMeasurementPlan::new(&source, index.clone()).unwrap();
            let mut previous = None;
            while let Some(probe) = plan.probe().cloned() {
                assert!(probe.bytes.len() <= MAX_PARAGRAPH_PROBE_BYTES);
                assert!(previous.is_none_or(|old| probe.bytes.start > old));
                assert_eq!(
                    index.at(&source, probe.glyph_start),
                    Some((probe.bytes.start, probe.native_start))
                );
                let end_glyph = index.index_at_byte(&source, probe.bytes.end).unwrap();
                assert_eq!(index.at(&source, end_glyph).unwrap().0, probe.bytes.end);
                let rectangles = rectangles(&plan);
                let width = f64::from(u32::try_from(end_glyph).unwrap()) * 7.0;
                assert!(plan.record(244.0, 15.0, width.max(244.0), &rectangles));
                previous = Some(probe.bytes.start);
            }
            let (width, geometry) = plan.finish().unwrap();
            assert_eq!(
                width.to_bits(),
                (f64::from(u32::try_from(index.len() - 1).unwrap()) * 7.0).to_bits()
            );
            let anchors = geometry.anchors(0..index.len() - 1).unwrap();
            let mut expected = index.anchor_glyphs().collect::<Vec<_>>();
            expected.dedup();
            assert_eq!(
                anchors.iter().map(|rect| rect.glyph).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn mismatched_overlap_or_dimensions_never_advance_the_source() {
        let source = "word space ".repeat(10_000);
        let index = VisualLineIndex::new(&source).unwrap();
        let mut plan = ParagraphMeasurementPlan::new(&source, index).unwrap();
        let first = rectangles(&plan);
        assert!(plan.record(244.0, 15.0, 200_000.0, &first));
        let before = plan.probe().unwrap().bytes.clone();
        let valid = rectangles(&plan);
        let mut bad = valid.clone();
        bad[0].left += 0.5;
        assert!(!plan.record(244.0, 15.0, 300_000.0, &bad));
        assert!(!plan.record(244.0, 16.0, 300_000.0, &valid));
        assert!(!plan.record(244.0, 15.0, f64::NAN, &valid));
        assert!(!plan.record(244.0, 15.0, 300_000.0, &valid[1..]));
        assert_eq!(plan.probe().unwrap().bytes, before);
        assert!(plan.record(244.0, 15.0, 300_000.0, &valid));
        assert!(plan.finish().is_none());
    }

    #[test]
    fn tab_grid_retry_preserves_source_and_requires_a_complete_second_proof() {
        let source = "word \t space ".repeat(10_000);
        let index = VisualLineIndex::new(&source).unwrap();
        let mut plan = ParagraphMeasurementPlan::new(&source, index.clone()).unwrap();
        let first = rectangles(&plan);
        assert!(plan.record(244.0, 15.0, 200_000.0, &first));
        let before = plan.probe().unwrap().bytes.clone();
        let valid = rectangles(&plan);
        let mut bad = valid.clone();
        let glyph = plan
            .expected
            .iter()
            .find(|rect| source.as_bytes()[index.at(&source, rect.glyph).unwrap().0] == b'\t')
            .unwrap()
            .glyph;
        bad.iter_mut()
            .find(|rect| rect.glyph == glyph)
            .unwrap()
            .width += 1.0;
        let phase = plan.local_origin();
        assert!(plan.retry_origin(&bad));
        assert!((plan.local_origin() - phase - 1.0).abs() < f64::EPSILON);
        assert_eq!(plan.probe().unwrap().bytes, before);
        assert!(!plan.retry_origin(&bad));
        assert!(!plan.record(244.0, 15.0, 300_000.0, &bad));
        assert_eq!(plan.probe().unwrap().bytes, before);
        assert!(plan.record(244.0, 15.0, 300_000.0, &valid));
    }

    #[test]
    fn unsupported_paragraphs_require_complete_measurement() {
        for source in [
            "short".into(),
            "word שלום ".repeat(10_000),
            format!("e{}", "\u{301}".repeat(50_000)),
        ] {
            let index = VisualLineIndex::new(&source).unwrap();
            assert!(ParagraphMeasurementPlan::new(&source, index).is_none());
        }
    }
}
