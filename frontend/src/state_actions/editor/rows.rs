//! Shared ownership and exact paint comparison for incremental row measurements.
use super::EditorActions;
use crate::state::workspace::EditorRowPaint;
use leptos::prelude::*;
use openwebide_core::{
    editor::{FoldProjection, Indentation, RowMeasurementPlan},
    highlight::Token,
};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

struct PaintRow<'a> {
    tokens: &'a [Token],
    guide: usize,
    normalize_cr: bool,
    ending: bool,
}
impl PartialEq for PaintRow<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.guide == other.guide
            && self.ending == other.ending
            && self.tokens.len() == other.tokens.len()
            && self
                .tokens
                .iter()
                .zip(other.tokens)
                .enumerate()
                .all(|(index, (a, b))| {
                    let last = index + 1 == self.tokens.len();
                    let a_text = if last && self.normalize_cr {
                        a.text.strip_suffix('\r').unwrap_or(&a.text)
                    } else {
                        &a.text
                    };
                    let b_text = if last && other.normalize_cr {
                        b.text.strip_suffix('\r').unwrap_or(&b.text)
                    } else {
                        &b.text
                    };
                    a.kind == b.kind && a_text == b_text
                })
    }
}
impl Eq for PaintRow<'_> {}
impl Hash for PaintRow<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.guide.hash(state);
        self.ending.hash(state);
        self.tokens.len().hash(state);
        for (index, token) in self.tokens.iter().enumerate() {
            std::mem::discriminant(&token.kind).hash(state);
            let text = if self.normalize_cr && index + 1 == self.tokens.len() {
                token.text.strip_suffix('\r').unwrap_or(&token.text)
            } else {
                &token.text
            };
            text.hash(state);
        }
    }
}
fn paint_rows(paint: &EditorRowPaint) -> Option<Vec<PaintRow<'_>>> {
    paint
        .projection
        .lines()
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let source = line.source_line;
            Some(PaintRow {
                tokens: paint.tokens.get(source)?,
                guide: paint.guides.get(source).copied().unwrap_or(0),
                normalize_cr: paint.prepared_source && source + 1 < paint.tokens.len(),
                ending: index + 1 < paint.projection.lines().len(),
            })
        })
        .collect()
}
impl EditorActions {
    pub fn paint_window(
        self,
        row: usize,
        line_height: f64,
        scroll: (f64, f64),
        size: (f64, f64),
    ) -> Option<openwebide_core::editor::RowPaintWindow> {
        self.wrapped_paint_window(row, line_height, scroll.1, size.1)
            .map(openwebide_core::editor::RowPaintWindow::Wrapped)
            .or_else(|| {
                self.horizontal_paint_bounds(row, scroll.0, size.0)
                    .map(openwebide_core::editor::RowPaintWindow::Horizontal)
            })
    }
    pub fn horizontal_paint_bounds(
        self,
        row: usize,
        scroll: f64,
        width: f64,
    ) -> Option<std::ops::Range<f64>> {
        if self.preferences().word_wrap {
            return None;
        }
        let projection = self.projection()?;
        let line = projection.lines().get(row)?;
        let raw = &projection.text()[line.visible_start..line.visible_start + line.source.len()];
        let body = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        openwebide_core::editor::horizontal_paint_bounds(body, scroll, width)
    }
    /// Shared selection of visual paint windows; DOM adapters supply exact row
    /// geometry and source boundaries, independent of the workspace transport.
    pub fn wrapped_paint_window(
        self,
        row: usize,
        line_height: f64,
        scroll: f64,
        viewport_height: f64,
    ) -> Option<openwebide_core::editor::EditorViewport> {
        if !self.preferences().word_wrap {
            return None;
        }
        let projection = self.projection()?;
        let measured = self.measured_rows()?;
        let top = measured.rows.top(row)?;
        openwebide_core::editor::wrapped_paint_window(
            projection.lines().get(row)?.source.len(),
            measured.rows.top(row + 1)? - top,
            line_height,
            scroll - top,
            viewport_height,
        )
    }
    pub(super) fn row_paint_current(self, paint: &EditorRowPaint) -> bool {
        self.key().as_ref() == Some(&paint.key)
            && self.workspace.pending_epoch.get_untracked() == paint.epoch
            && self.workspace.editor_read_revision.get_untracked() == paint.read_revision
            && self.auth.map_or(0, |auth| auth.generation.get_untracked())
                == paint.account_generation
    }
    pub fn prepare_row_measurements(
        self,
        metrics: String,
        projection: FoldProjection,
        tokens: (bool, Arc<Vec<Vec<Token>>>),
        guides: Arc<[usize]>,
        indentation: Indentation,
        whitespace: bool,
    ) -> Option<(EditorRowPaint, RowMeasurementPlan)> {
        let paint = EditorRowPaint {
            key: self.key()?,
            epoch: self.workspace.pending_epoch.get_untracked(),
            read_revision: self.workspace.editor_read_revision.get_untracked(),
            account_generation: self.auth.map_or(0, |auth| auth.generation.get_untracked()),
            metrics,
            projection,
            tokens: tokens.1,
            prepared_source: tokens.0,
            guides,
            indentation,
            whitespace,
        };
        let reused = self.workspace.editor_row_cache.with_untracked(|cache| {
            let cache = cache.as_ref()?;
            let old = &cache.paint;
            if old.key != paint.key
                || old.epoch != paint.epoch
                || old.read_revision != paint.read_revision
                || old.account_generation != paint.account_generation
                || old.metrics != paint.metrics
                || old.indentation != paint.indentation
                || old.whitespace != paint.whitespace
            {
                return None;
            }
            RowMeasurementPlan::reuse(&paint_rows(old)?, &paint_rows(&paint)?, &cache.rows)
        });
        let plan = reused.or_else(|| RowMeasurementPlan::new(paint.projection.lines().len()))?;
        Some((paint, plan))
    }
    pub fn invalidate_measured_font(self) {
        self.workspace.editor_row_cache.set(None);
        self.invalidate_measured_rows();
    }
}
