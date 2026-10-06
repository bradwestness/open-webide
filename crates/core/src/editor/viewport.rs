//! A document row window; browser adapters supply measured scroll geometry.
use std::{collections::HashMap, hash::Hash, ops::Range};

pub const MAX_MEASURE_ROWS: usize = 128;
pub const MAX_MEASURE_BYTES: usize = 64 * 1024;
/// Give native input and visible paint a turn during cold preparation.
pub const MAX_MEASURE_BATCHES_PER_FRAME: usize = 8;

/// A window inside an exactly measured wrapped logical line. Reject irregular
/// heights rather than estimating where the browser placed its wrap boundaries.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "The finite positive row count is bounded before conversion"
)]
pub fn wrapped_paint_window(
    bytes: usize,
    measured_height: f64,
    line_height: f64,
    scroll: f64,
    viewport_height: f64,
) -> Option<EditorViewport> {
    if bytes <= MAX_MEASURE_BYTES
        || bytes > super::MAX_EDITOR_BYTES
        || !measured_height.is_finite()
        || !line_height.is_finite()
        || !(1.0..=4096.0).contains(&line_height)
        || !scroll.is_finite()
        || !viewport_height.is_finite()
        || viewport_height <= 0.0
    {
        return None;
    }
    let rows = (measured_height / line_height).round();
    if !(1.0..=1_000_000_000.0 / line_height).contains(&rows)
        || (rows * line_height - measured_height).abs() > 0.5
    {
        return None;
    }
    let window =
        EditorViewport::unwrapped(rows as usize, scroll, viewport_height, line_height, 0.0);
    (window.rows.len() < rows as usize).then_some(window)
}

/// Small projections can be measured in their visible paint. Larger ones use
/// temporary batches instead of allocating a full document of styled DOM nodes.
pub const fn needs_measured_batches(rows: usize, bytes: usize) -> bool {
    rows > MAX_MEASURE_ROWS || bytes > MAX_MEASURE_BYTES
}

/// Keep a complete logical row together, including an individually long row.
/// The first row may exceed the byte budget, but never the editor admission cap.
/// Finer rendering inside such rows is a separate visual-row concern.
pub fn row_measurement_batch(lengths: impl IntoIterator<Item = usize>) -> usize {
    let mut rows = 0;
    let mut bytes = 0_usize;
    for length in lengths.into_iter().take(MAX_MEASURE_ROWS) {
        if length > super::MAX_EDITOR_BYTES {
            break;
        }
        let Some(next) = bytes.checked_add(length) else {
            break;
        };
        if rows > 0 && next > MAX_MEASURE_BYTES {
            break;
        }
        bytes = next;
        rows += 1;
    }
    rows
}

/// Exact unchanged paint rows retain their measured height after a transaction.
/// Adapters measure only missing rows; no hashes or wrapping estimates are used.
#[derive(Clone, Debug)]
pub struct RowMeasurementPlan {
    heights: Vec<Option<f64>>,
    next: usize,
    completed: usize,
}
impl RowMeasurementPlan {
    pub fn new(rows: usize) -> Option<Self> {
        (rows <= super::MAX_EDITOR_LINES).then(|| Self {
            heights: vec![None; rows],
            next: 0,
            completed: 0,
        })
    }
    pub fn reuse<T: Eq + Hash>(
        previous: &[T],
        current: &[T],
        measured: &MeasuredRows,
    ) -> Option<Self> {
        if previous.len() != measured.len() {
            return None;
        }
        let mut plan = Self::new(current.len())?;
        let prefix = previous
            .iter()
            .zip(current)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = previous
            .iter()
            .rev()
            .zip(current.iter().rev())
            .take(previous.len().min(current.len()) - prefix)
            .take_while(|(a, b)| a == b)
            .count();
        for row in 0..prefix {
            plan.heights[row] = Some(measured.top(row + 1)? - measured.top(row)?);
        }
        for offset in 0..suffix {
            let old = previous.len() - 1 - offset;
            plan.heights[current.len() - 1 - offset] =
                Some(measured.top(old + 1)? - measured.top(old)?);
        }
        plan.completed = prefix + suffix;
        if current.len() - plan.completed > MAX_MEASURE_ROWS {
            let mut known = HashMap::<&T, Option<f64>>::new();
            for (row, key) in previous
                .iter()
                .enumerate()
                .take(previous.len() - suffix)
                .skip(prefix)
            {
                let height = measured.top(row + 1)? - measured.top(row)?;
                known
                    .entry(key)
                    .and_modify(|value| {
                        // Context-sensitive or rounded duplicates are not reusable.
                        if value.is_some_and(|old| old.to_bits() != height.to_bits()) {
                            *value = None;
                        }
                    })
                    .or_insert(Some(height));
            }
            for (row, key) in current
                .iter()
                .enumerate()
                .take(current.len() - suffix)
                .skip(prefix)
            {
                if let Some(Some(height)) = known.get(key) {
                    plan.heights[row] = Some(*height);
                    plan.completed += 1;
                }
            }
        }
        Some(plan)
    }
    pub fn completed(&self) -> usize {
        self.completed
    }
    pub fn pending_batch(&mut self, lengths: &[usize]) -> Option<Range<usize>> {
        if lengths.len() != self.heights.len() {
            return None;
        }
        while self.next < self.heights.len() && self.heights[self.next].is_some() {
            self.next += 1;
        }
        let start = self.next;
        let count = row_measurement_batch(
            lengths[start..]
                .iter()
                .copied()
                .zip(&self.heights[start..])
                .take_while(|(_, height)| height.is_none())
                .map(|(length, _)| length),
        );
        (count > 0).then_some(start..start + count)
    }
    pub fn record(&mut self, rows: Range<usize>, heights: &[f64]) -> bool {
        if rows.start != self.next
            || rows.end > self.heights.len()
            || rows.len() != heights.len()
            || heights.is_empty()
            || MeasuredRows::new(heights.iter().copied()).is_none()
            || self.heights[rows.clone()].iter().any(Option::is_some)
        {
            return false;
        }
        for (row, height) in rows.clone().zip(heights) {
            self.heights[row] = Some(*height);
        }
        self.completed += rows.len();
        self.next = rows.end;
        true
    }
    pub fn finish(self) -> Option<MeasuredRows> {
        MeasuredRows::new(self.heights.into_iter().collect::<Option<Vec<_>>>()?)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorViewport {
    pub rows: Range<usize>,
    pub top: f64,
    pub height: f64,
}

/// Exact browser-measured logical row heights. Shared Rust validates the
/// measurements and chooses windows; it never estimates text wrapping.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredRows(std::sync::Arc<[f64]>);
impl MeasuredRows {
    pub fn new(heights: impl IntoIterator<Item = f64>) -> Option<Self> {
        let mut offsets = vec![0.0];
        for height in heights {
            if !height.is_finite() || height <= 0.0 || offsets.len() > super::MAX_EDITOR_LINES {
                return None;
            }
            let end = offsets.last()? + height;
            if !end.is_finite() || end > 1_000_000_000.0 {
                return None;
            }
            offsets.push(end);
        }
        Some(Self(offsets.into()))
    }
    pub fn len(&self) -> usize {
        self.0.len() - 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn top(&self, row: usize) -> Option<f64> {
        self.0.get(row).copied()
    }
    pub fn height(&self) -> f64 {
        *self.0.last().unwrap()
    }
    pub fn window(&self, scroll: f64, height: f64, padding: f64) -> EditorViewport {
        let scroll = if scroll.is_finite() {
            scroll.max(0.0)
        } else {
            0.0
        };
        let height = if height.is_finite() {
            height.max(1.0)
        } else {
            1.0
        };
        let padding = if padding.is_finite() {
            padding.max(0.0)
        } else {
            0.0
        };
        let top = (scroll - padding).max(0.0).min(self.height());
        let first = self
            .0
            .partition_point(|offset| *offset <= top)
            .saturating_sub(1)
            .min(self.len().saturating_sub(1));
        let end = self
            .0
            .partition_point(|offset| *offset < top + height)
            .min(self.len());
        let start = first.saturating_sub(8);
        let end = end.saturating_add(8).min(self.len());
        EditorViewport {
            rows: start..end,
            top: self.0[start],
            height: self.height(),
        }
    }
}

impl EditorViewport {
    /// Keep overscan on both sides and clamp stale scroll positions after edits.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "finite nonnegative pixel divisions saturate to usize and then clamp to the row count"
    )]
    pub fn unwrapped(rows: usize, scroll: f64, height: f64, row_height: f64, padding: f64) -> Self {
        let row_height = if row_height.is_finite() && row_height > 0.0 {
            row_height.clamp(1.0, 4096.0)
        } else {
            19.5
        };
        let scroll = if scroll.is_finite() {
            scroll.max(0.0)
        } else {
            0.0
        };
        let height = if height.is_finite() {
            height.max(row_height)
        } else {
            row_height
        };
        let padding = if padding.is_finite() {
            padding.max(0.0)
        } else {
            0.0
        };
        let first = (((scroll - padding).max(0.0) / row_height).floor() as usize)
            .min(rows.saturating_sub(1));
        let count = (height / row_height).ceil() as usize;
        let start = first.saturating_sub(8);
        let end = first.saturating_add(count).saturating_add(9).min(rows);
        Self {
            rows: start..end,
            top: start as f64 * row_height,
            height: rows as f64 * row_height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disjoint_edits_reuse_exact_interior_rows_and_skip_ambiguous_duplicates() {
        let old = (0..200).map(|row| format!("row {row}")).collect::<Vec<_>>();
        let mut current = old.clone();
        current[20] = "changed first".into();
        current[170] = "changed second".into();
        let measured = MeasuredRows::new(std::iter::repeat_n(20.0, 200)).unwrap();
        let mut plan = RowMeasurementPlan::reuse(&old, &current, &measured).unwrap();
        assert_eq!(plan.completed(), 198);
        let lengths = vec![1; 200];
        assert_eq!(plan.pending_batch(&lengths), Some(20..21));
        assert!(plan.record(20..21, &[40.0]));
        assert_eq!(plan.pending_batch(&lengths), Some(170..171));
        assert!(plan.record(170..171, &[60.0]));
        assert!(plan.pending_batch(&lengths).is_none());
        assert!(plan.finish().is_some());
        let old = vec!["duplicate"; 200];
        let current = vec!["other"; 201]
            .into_iter()
            .enumerate()
            .map(|(i, key)| if i == 100 { "duplicate" } else { key })
            .collect::<Vec<_>>();
        let varied =
            MeasuredRows::new((0..200).map(|i| if i == 100 { 40.0 } else { 20.0 })).unwrap();
        assert_eq!(
            RowMeasurementPlan::reuse(&old, &current, &varied)
                .unwrap()
                .completed(),
            0
        );
    }

    #[test]
    fn exact_row_reuse_tracks_insert_delete_and_changed_paint() {
        let old = ["a", "b", "c", "last"];
        let measured = MeasuredRows::new([20.0, 40.0, 60.0, 20.0]).unwrap();
        for (keys, expected, batch) in [
            (vec!["a", "new", "b", "c", "last"], 4, Some(1..2)),
            (vec!["a", "c", "last"], 3, None),
            (vec!["a", "changed style", "c", "last"], 3, Some(1..2)),
            (old.to_vec(), 4, None),
        ] {
            let mut plan = RowMeasurementPlan::reuse(&old, &keys, &measured).unwrap();
            assert_eq!(plan.completed(), expected);
            assert_eq!(plan.pending_batch(&vec![1; keys.len()]), batch);
            if let Some(batch) = batch {
                assert!(!plan.record(batch.clone(), &[f64::NAN]));
                assert_eq!(plan.completed(), expected);
                assert!(plan.record(batch, &[80.0]));
            }
            let result = plan.finish().unwrap();
            assert_eq!(result.len(), keys.len());
            assert!(
                (result.top(keys.len()).unwrap() - result.top(keys.len() - 1).unwrap() - 20.0)
                    .abs()
                    < f64::EPSILON
            );
        }
        assert!(RowMeasurementPlan::reuse(&old[..2], &old, &measured).is_none());
        assert!(RowMeasurementPlan::new(super::super::MAX_EDITOR_LINES + 1).is_none());
        assert!(RowMeasurementPlan::new(1).unwrap().finish().is_none());
    }

    #[test]
    fn measurement_batches_bound_rows_and_bytes_without_splitting_long_rows() {
        assert_eq!(
            row_measurement_batch(std::iter::repeat(0)),
            MAX_MEASURE_ROWS
        );
        assert_eq!(row_measurement_batch([32_768, 32_768, 1]), 2);
        assert_eq!(row_measurement_batch([MAX_MEASURE_BYTES + 1, 1]), 1);
        assert_eq!(row_measurement_batch([usize::MAX]), 0);
        assert!(needs_measured_batches(1, MAX_MEASURE_BYTES + 1));
        assert!(!needs_measured_batches(MAX_MEASURE_ROWS, MAX_MEASURE_BYTES));
        assert!(needs_measured_batches(MAX_MEASURE_ROWS + 1, 0));
    }
    #[test]
    fn measured_windows_use_actual_wrap_heights_and_clamp_stale_scroll() {
        let rows =
            MeasuredRows::new((0..100).map(|row| if row == 20 { 195.0 } else { 19.5 })).unwrap();
        let view = rows.window(12.0 + 390.0 + 97.5, 39.0, 12.0);
        assert_eq!(view.rows, 12..29);
        assert!((view.top - 234.0).abs() < 0.001);
        assert!((rows.top(21).unwrap() - 585.0).abs() < 0.001);
        assert_eq!(rows.window(f64::MAX, 39.0, 12.0).rows, 91..100);
        assert_eq!(
            MeasuredRows::new([]).unwrap().window(0.0, 100.0, 12.0).rows,
            0..0
        );
        for height in [0.0, -1.0, f64::NAN, f64::INFINITY, 1_000_000_001.0] {
            assert!(MeasuredRows::new([height]).is_none());
        }
        assert!(
            MeasuredRows::new(std::iter::repeat_n(
                19.5,
                super::super::MAX_EDITOR_LINES + 1
            ))
            .is_none()
        );
    }

    #[test]
    fn window_bounds_follow_scroll_and_recover_after_document_shrinks() {
        let view = EditorViewport::unwrapped(100_000, 19_512.0, 390.0, 19.5, 12.0);
        assert_eq!(view.rows, 992..1029);
        assert!((view.top - 19_344.0).abs() < f64::EPSILON);
        assert!((view.height - 1_950_000.0).abs() < f64::EPSILON);
        let shrunk = EditorViewport::unwrapped(4, 19_512.0, 390.0, 19.5, 12.0);
        assert_eq!(shrunk.rows, 0..4);
        assert_eq!(
            EditorViewport::unwrapped(0, 0.0, 390.0, 19.5, 12.0).rows,
            0..0
        );
    }
    #[test]
    fn invalid_geometry_never_produces_nonfinite_offsets_or_escaping_rows() {
        for value in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            -1.0,
            0.0,
        ] {
            let view = EditorViewport::unwrapped(100, value, value, value, value);
            assert!(view.rows.start <= view.rows.end && view.rows.end <= 100);
            assert!(view.top.is_finite() && view.height.is_finite());
        }
    }

    #[test]
    fn wrapped_fragments_use_exact_height_and_bounded_visual_overscan() {
        let window = wrapped_paint_window(100_000, 195_000.0, 19.5, 19_500.0, 390.0).unwrap();
        assert_eq!(window.rows, 992..1029);
        assert!((window.height - 195_000.0).abs() < f64::EPSILON);
        assert!((window.top - 19_344.0).abs() < f64::EPSILON);
        let end = wrapped_paint_window(100_000, 195_000.0, 19.5, f64::MAX, 390.0).unwrap();
        assert_eq!(end.rows, 9991..10_000);
        for (bytes, height, line, scroll, viewport) in [
            (64 * 1024, 195_000.0, 19.5, 0.0, 390.0),
            (
                super::super::MAX_EDITOR_BYTES + 1,
                195_000.0,
                19.5,
                0.0,
                390.0,
            ),
            (100_000, 195_001.0, 19.5, 0.0, 390.0),
            (100_000, f64::INFINITY, 19.5, 0.0, 390.0),
            (100_000, 195_000.0, 0.0, 0.0, 390.0),
            (100_000, 195_000.0, 19.5, f64::NAN, 390.0),
            (100_000, 195_000.0, 19.5, 0.0, 0.0),
            (100_000, 19.5, 19.5, 0.0, 390.0),
        ] {
            assert!(wrapped_paint_window(bytes, height, line, scroll, viewport).is_none());
        }
    }
}
