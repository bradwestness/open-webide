//! A document row window; browser adapters supply measured scroll geometry.
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub struct EditorViewport {
    pub rows: Range<usize>,
    pub top: f64,
    pub height: f64,
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
