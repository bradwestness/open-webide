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
    plain: Option<&'a str>,
    guide: usize,
    normalize_cr: bool,
    ending: bool,
}
impl PartialEq for PaintRow<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.plain == other.plain
            && self.guide == other.guide
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
        self.plain.hash(state);
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
fn paint_row(paint: &EditorRowPaint, index: usize) -> Option<PaintRow<'_>> {
    let source = paint.projection.lines().get(index)?.source_line;
    let mut tokens = match paint.tokens.get(source) {
        Some(tokens) => tokens.as_ref(),
        None if !paint.prepared_source => &[],
        None => return None,
    };
    let normalize_cr = paint.prepared_source && source + 1 < paint.tokens.len();
    let plain = if let [token] = tokens
        && token.kind == openwebide_core::highlight::TokenKind::Plain
    {
        // Both render paths call paint_text once with the same row body. Keep
        // multiple token runs distinct: their span boundaries can affect shaping.
        let body = if normalize_cr {
            token.text.strip_suffix('\r').unwrap_or(&token.text)
        } else {
            &token.text
        };
        tokens = &[];
        Some(body)
    } else if !paint.prepared_source && paint.tokens.get(source).is_none() {
        Some(paint.projection.line_body(index)?)
    } else {
        None
    };
    Some(PaintRow {
        tokens,
        plain,
        guide: paint.guides.get(source).copied().unwrap_or(0),
        normalize_cr,
        ending: index + 1 < paint.projection.lines().len(),
    })
}
fn paint_rows(paint: &EditorRowPaint) -> Option<Vec<PaintRow<'_>>> {
    (0..paint.projection.lines().len())
        .map(|index| paint_row(paint, index))
        .collect()
}
fn same_measurement_environment(old: &EditorRowPaint, paint: &EditorRowPaint) -> bool {
    old.font_epoch == paint.font_epoch
        && old.key == paint.key
        && old.epoch == paint.epoch
        && old.read_revision == paint.read_revision
        && old.account_generation == paint.account_generation
        && old.metrics == paint.metrics
        && old.indentation == paint.indentation
        && old.whitespace == paint.whitespace
}
fn same_paint_scope(old: &EditorRowPaint, paint: &EditorRowPaint) -> bool {
    old.view_revision == paint.view_revision
        && old.layout_epoch == paint.layout_epoch
        && same_measurement_environment(old, paint)
        && old.prepared_source == paint.prepared_source
        && Arc::ptr_eq(&old.tokens, &paint.tokens)
        && Arc::ptr_eq(&old.guides, &paint.guides)
}
/// Exact source interval selected from immutable styled anchors before HTML generation.
#[derive(Clone, Debug)]
pub struct EditorRowSourceSlice {
    pub source_line: usize,
    pub bytes: std::ops::Range<usize>,
    pub native_start: usize,
    pub reaches_end: bool,
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
        projection
            .visual_line_index(row)?
            .horizontal_paint_bounds(scroll, width)
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
        self.workspace.editor_font_epoch.get_untracked() == paint.font_epoch
            && self.workspace.editor_view_revision.get_untracked() == paint.view_revision
            && self.workspace.editor_layout_epoch.get_untracked() == paint.layout_epoch
            && self.key().as_ref() == Some(&paint.key)
            && self.workspace.pending_epoch.get_untracked() == paint.epoch
            && self.workspace.editor_read_revision.get_untracked() == paint.read_revision
            && self.auth.map_or(0, |auth| auth.generation.get_untracked())
                == paint.account_generation
    }
    /// Guide queries reuse the shared source document's indexed logical rows.
    pub fn indent_guides(self, indentation: Indentation) -> Arc<[usize]> {
        let Some(key) = self.key() else {
            return Arc::from([]);
        };
        self.workspace.content.with_untracked(|source| {
            self.workspace.editor_documents.with_untracked(|documents| {
                documents
                    .get(&key)
                    .filter(|document| document.text() == source)
                    .map_or_else(
                        || Arc::from([]),
                        |document| document.indent_guide_columns(indentation),
                    )
            })
        })
    }

    fn row_paint_snapshot(
        self,
        metrics: String,
        projection: FoldProjection,
        tokens: (bool, Arc<openwebide_core::highlight::TokenRows>),
        guides: Arc<[usize]>,
        indentation: Indentation,
        whitespace: bool,
    ) -> Option<EditorRowPaint> {
        Some(EditorRowPaint {
            view_revision: self.workspace.editor_view_revision.get_untracked(),
            layout_epoch: self.workspace.editor_layout_epoch.get_untracked(),
            font_epoch: self.workspace.editor_font_epoch.get_untracked(),
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
        })
    }
    pub fn prepare_row_measurements(
        self,
        metrics: String,
        projection: FoldProjection,
        tokens: (bool, Arc<openwebide_core::highlight::TokenRows>),
        guides: Arc<[usize]>,
        indentation: Indentation,
        whitespace: bool,
    ) -> Option<(EditorRowPaint, RowMeasurementPlan)> {
        if !self.full_row_paint_ready() {
            return None;
        }
        let paint =
            self.row_paint_snapshot(metrics, projection, tokens, guides, indentation, whitespace)?;
        let current_rows = paint_rows(&paint)?;
        let reused = self.workspace.editor_row_cache.with_untracked(|cache| {
            let cache = cache.as_ref()?;
            let old = &cache.paint;
            if !same_measurement_environment(old, &paint) {
                return None;
            }
            RowMeasurementPlan::reuse_layout(&paint_rows(old)?, &current_rows, &cache.rows)
        });
        let mut plan =
            reused.or_else(|| RowMeasurementPlan::new(paint.projection.lines().len()))?;
        if !plan.share_repeated(&current_rows) {
            return None;
        }
        Some((paint, plan))
    }
    /// A native notification can arrive after geometry already used the loaded
    /// face. Retain only current, source/account-owned measurements with identical
    /// actual face availability and CSS metrics; unknown geometry still refreshes.
    pub fn font_measurements_changed(self, metrics: Option<&str>) -> bool {
        self.measured_rows()
            .is_none_or(|rows| Some(rows.metrics.as_str()) != metrics)
    }
    pub fn invalidate_measured_font(self) {
        self.workspace
            .editor_font_epoch
            .update(|epoch| *epoch = epoch.wrapping_add(1));
        self.workspace.editor_row_cache.set(None);
        self.invalidate_measured_rows();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorFragmentWindow {
    pub rows: Vec<usize>,
    pub windows: Vec<(usize, openwebide_core::editor::RowPaintWindow)>,
    pub width: i32,
    pub height: i32,
    pub trailing: bool,
}
#[derive(Default)]
pub struct EditorFragmentCache {
    scope: Option<(EditorRowPaint, u64, u64)>,
    geometry:
        std::collections::VecDeque<(usize, Arc<openwebide_core::editor::MeasuredRowGeometry>)>,
    paint: openwebide_core::editor::PaintCache<EditorFragmentWindow>,
}
impl EditorActions {
    /// Select one exact source/paint/layout scope before retaining any DOM result.
    #[allow(
        clippy::too_many_arguments,
        reason = "Immutable paint primitives accompany the adapter's measured layout"
    )]
    pub fn fragment_scope(
        self,
        cache: &mut EditorFragmentCache,
        metrics: String,
        tokens: (bool, Arc<openwebide_core::highlight::TokenRows>),
        guides: Arc<[usize]>,
        indentation: Indentation,
        whitespace: bool,
    ) -> bool {
        let paint = self.projection().and_then(|projection| {
            self.row_paint_snapshot(metrics, projection, tokens, guides, indentation, whitespace)
        });
        let Some(paint) = paint else {
            cache.scope = None;
            cache.paint.clear();
            cache.geometry.clear();
            return false;
        };
        let revision = self.workspace.editor_view_revision.get_untracked();
        let layout = self.workspace.editor_layout_epoch.get_untracked();
        let same = cache
            .scope
            .as_ref()
            .is_some_and(|(old, old_revision, old_layout)| {
                *old_revision == revision && *old_layout == layout && same_paint_scope(old, &paint)
            });
        if !same {
            let reusable = cache.scope.as_ref().and_then(|(old, _, _)| {
                // Geometry follows actual fonts and exact styled rows. Height
                // reconciliation can advance the layout scope after an equivalent
                // syntax result without changing either; no height cache is needed.
                if !same_measurement_environment(old, &paint) {
                    return None;
                }
                Some(
                    cache
                        .geometry
                        .iter()
                        .filter(|(row, _)| {
                            paint_row(old, *row)
                                .zip(paint_row(&paint, *row))
                                .is_some_and(|(old, new)| old == new)
                        })
                        .cloned()
                        .collect(),
                )
            });
            cache.paint.clear();
            cache.geometry = reusable.unwrap_or_default();
            cache.scope = Some((paint, revision, layout));
        }
        true
    }
    pub fn measured_row_geometry(
        self,
        cache: &mut EditorFragmentCache,
        row: usize,
    ) -> Option<Arc<openwebide_core::editor::MeasuredRowGeometry>> {
        let (paint, revision, layout) = cache.scope.as_ref()?;
        if !self.row_paint_current(paint)
            || *revision != self.workspace.editor_view_revision.get_untracked()
            || *layout != self.workspace.editor_layout_epoch.get_untracked()
        {
            return None;
        }
        let at = cache.geometry.iter().position(|(index, _)| *index == row)?;
        let entry = cache.geometry.remove(at)?;
        let result = entry.1.clone();
        cache.geometry.push_back(entry);
        Some(result)
    }
    pub fn row_source_slice(
        self,
        cache: &mut EditorFragmentCache,
        row: usize,
        window: &openwebide_core::editor::RowPaintWindow,
        line_height: f64,
    ) -> Option<EditorRowSourceSlice> {
        let geometry = self.measured_row_geometry(cache, row)?;
        let paint = &cache.scope.as_ref()?.0;
        let line = paint.projection.lines().get(row)?;
        let end = paint
            .projection
            .lines()
            .get(row + 1)
            .map_or(paint.projection.text().len(), |next| next.visible_start);
        let raw = paint.projection.text().get(line.visible_start..end)?;
        let body = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        let index = paint.projection.visual_line_index(row)?;
        if !index.source_paint_eligible() {
            return None;
        }
        let interval = geometry.source_interval(window, line_height)?;
        let (start, native_start) = index.at(body, interval.start)?;
        let (end, _) = index.at(body, interval.end)?;
        if end.checked_sub(start)? > openwebide_core::editor::MAX_MEASURE_BYTES {
            return None;
        }
        Some(EditorRowSourceSlice {
            source_line: line.source_line,
            bytes: start..end,
            native_start,
            reaches_end: end == body.len(),
        })
    }
    pub fn forget_measured_row_geometry(self, cache: &mut EditorFragmentCache, row: usize) {
        cache.geometry.retain(|(index, _)| *index != row);
    }
    pub fn retain_measured_row_geometry(
        self,
        cache: &mut EditorFragmentCache,
        row: usize,
        geometry: openwebide_core::editor::MeasuredRowGeometry,
    ) {
        let Some((paint, revision, layout)) = cache.scope.as_ref() else {
            return;
        };
        if !self.row_paint_current(paint)
            || *revision != self.workspace.editor_view_revision.get_untracked()
            || *layout != self.workspace.editor_layout_epoch.get_untracked()
        {
            return;
        }
        if matches!(
            geometry,
            openwebide_core::editor::MeasuredRowGeometry::Wrapped(_)
        ) && !paint
            .projection
            .visual_line_index(row)
            .is_some_and(|index| index.source_paint_eligible())
        {
            return;
        }
        cache.geometry.retain(|(index, _)| *index != row);
        if cache.geometry.len() == 8 {
            cache.geometry.pop_front();
        }
        cache.geometry.push_back((row, Arc::new(geometry)));
    }
    /// A cold probe may only add geometry to the exact styled scope it measured.
    pub fn retain_preparation_geometry(
        self,
        cache: &mut EditorFragmentCache,
        paint: &EditorRowPaint,
        row: usize,
        geometry: openwebide_core::editor::MeasuredRowGeometry,
    ) {
        if cache
            .scope
            .as_ref()
            .is_some_and(|(current, _, _)| same_paint_scope(current, paint))
        {
            self.retain_measured_row_geometry(cache, row, geometry);
        }
    }
    pub fn cached_fragment(
        self,
        cache: &mut EditorFragmentCache,
        window: &EditorFragmentWindow,
    ) -> Option<String> {
        let (paint, revision, layout) = cache.scope.as_ref()?;
        if !self.row_paint_current(paint)
            || *revision != self.workspace.editor_view_revision.get_untracked()
            || *layout != self.workspace.editor_layout_epoch.get_untracked()
        {
            return None;
        }
        cache.paint.get(window)
    }
    pub fn retain_fragment(
        self,
        cache: &mut EditorFragmentCache,
        window: EditorFragmentWindow,
        html: String,
    ) {
        if let Some((paint, revision, layout)) = cache.scope.as_ref()
            && self.row_paint_current(paint)
            && *revision == self.workspace.editor_view_revision.get_untracked()
            && *layout == self.workspace.editor_layout_epoch.get_untracked()
        {
            cache.paint.insert(window, html);
        }
    }
}
