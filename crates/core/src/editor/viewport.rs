//! A document row window; browser adapters supply measured scroll geometry.
use std::ops::Range;

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
}
