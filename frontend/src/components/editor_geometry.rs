//! Browser text measurements for shared visual-row cursor movement.
use super::editor::{current_editor_target, sync_highlight_scroll};
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::{MAX_VISUAL_CARETS, VisualCaret, VisualLayout, visual_line_offsets};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;

/// DOM values are rounded first and bounded well inside exact integer precision.
#[allow(
    clippy::cast_possible_truncation,
    reason = "The finite rounded DOM value is range checked before its integer conversion"
)]
fn measured_integer(value: f64) -> Option<i64> {
    if !value.is_finite() || value.abs() > 1_000_000_000_000.0 {
        return None;
    }
    Some(value as i64)
}
fn measured_row(value: f64) -> Option<usize> {
    usize::try_from(measured_integer(value)?).ok()
}

struct TextNodes(Vec<(web_sys::Node, u32, u32)>);
impl TextNodes {
    fn new(row: &web_sys::Element) -> Option<Self> {
        let mut pending = vec![web_sys::Node::from(row.clone())];
        let mut result = Vec::new();
        let mut offset = 0_u32;
        let mut visited = 0;
        while let Some(node) = pending.pop() {
            visited += 1;
            if visited > MAX_VISUAL_CARETS {
                return None;
            }
            if node.node_type() == web_sys::Node::TEXT_NODE {
                let length = u32::try_from(node.node_value()?.encode_utf16().count()).ok()?;
                result.push((node, offset, length));
                offset = offset.checked_add(length)?;
            } else {
                let children = node.child_nodes();
                pending.extend(
                    (0..children.length())
                        .rev()
                        .filter_map(|index| children.item(index)),
                );
            }
        }
        Some(Self(result))
    }
    fn position(&self, offset: u32) -> Option<(&web_sys::Node, u32)> {
        let index = self
            .0
            .partition_point(|(_, start, length)| start + length <= offset);
        let (node, start, length) = self.0.get(index).or_else(|| self.0.last())?;
        let at = offset.checked_sub(*start)?;
        (at <= *length).then_some((node, at))
    }
}

struct Glyphs {
    offsets: Vec<(usize, usize)>,
    nodes: TextNodes,
    range: web_sys::Range,
    measured: BTreeMap<usize, web_sys::DomRect>,
}
impl Glyphs {
    fn new(row: &web_sys::Element, body: &str) -> Option<Self> {
        Some(Self {
            offsets: visual_line_offsets(body).ok()?,
            nodes: TextNodes::new(row)?,
            range: document().create_range().ok()?,
            measured: BTreeMap::new(),
        })
    }
    fn rect(&mut self, index: usize) -> Option<web_sys::DomRect> {
        if let Some(rect) = self.measured.get(&index) {
            return Some(rect.clone());
        }
        if self.measured.len() >= MAX_VISUAL_CARETS {
            return None;
        }
        let (start_node, start_at) = self
            .nodes
            .position(u32::try_from(self.offsets.get(index)?.1).ok()?)?;
        let (end_node, end_at) = self
            .nodes
            .position(u32::try_from(self.offsets.get(index + 1)?.1).ok()?)?;
        self.range.set_start(start_node, start_at).ok()?;
        self.range.set_end(end_node, end_at).ok()?;
        let rects = self.range.get_client_rects()?;
        let rect = (0..rects.length())
            .filter_map(|index| rects.item(index))
            .find(|rect| rect.height() > 0.0)?;
        self.measured.insert(index, rect.clone());
        Some(rect)
    }
    fn row(&mut self, index: usize, metrics: &VisualMetrics) -> Option<usize> {
        measured_row(((self.rect(index)?.top() - metrics.top) / metrics.line_height).floor())
    }
    // Visual rows increase in source order even when a row contains bidi text.
    fn first_on_row(&mut self, minimum: usize, metrics: &VisualMetrics) -> Option<usize> {
        let mut start = 0;
        let mut end = self.offsets.len().checked_sub(1)?;
        while start < end {
            let middle = start + (end - start) / 2;
            if self.row(middle, metrics)? < minimum {
                start = middle + 1;
            } else {
                end = middle;
            }
        }
        Some(start)
    }
}

pub(super) struct VisualMetrics {
    pub identity: String,
    pub left: f64,
    pub top: f64,
    pub line_height: f64,
    pub caret_height: f64,
    pub caret_inset: f64,
    pub rows: usize,
}

pub(super) fn visual_metrics(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
) -> Option<VisualMetrics> {
    let parent = input.parent_element()?;
    let paint = parent.query_selector(".editor-highlight-content").ok()??;
    if paint.get_attribute("data-editor-scope").as_deref()
        != Some(actions.projection_revision().to_string().as_str())
    {
        return None;
    }
    let first = paint.query_selector(".editor-source-line").ok()??;
    let last = paint
        .query_selector(".editor-source-line:last-child")
        .ok()??;
    let first_rect = first.get_bounding_client_rect();
    let last_rect = last.get_bounding_client_rect();
    let style = window().get_computed_style(input).ok()??;
    let line_height: f64 = style
        .get_property_value("line-height")
        .ok()?
        .trim_end_matches("px")
        .parse()
        .ok()?;
    if !line_height.is_finite() || line_height <= 0.0 {
        return None;
    }
    let geometry = super::editor_rows::metrics_identity(input)?;
    if actions
        .measured_rows()
        .is_some_and(|rows| rows.metrics != geometry)
    {
        return None;
    }
    let identity = format!("{geometry}:{}", actions.view_revision());
    let window_top: f64 = paint.get_attribute("data-viewport-top")?.parse().ok()?;
    let document_height: f64 = paint.get_attribute("data-document-height")?.parse().ok()?;
    if !window_top.is_finite()
        || window_top < 0.0
        || !document_height.is_finite()
        || document_height < 0.0
    {
        return None;
    }
    let caret = super::editor::caret_rect(&first, 0).filter(|rect| rect.height() > 0.0);
    Some(VisualMetrics {
        identity,
        left: first_rect.left(),
        top: first_rect.top() - window_top,
        line_height,
        caret_height: caret.as_ref().map_or(line_height, web_sys::DomRect::height),
        caret_inset: caret
            .as_ref()
            .map_or(0.0, |caret| caret.top() - first_rect.top()),
        rows: measured_row(
            (if document_height > 0.0 {
                document_height
            } else {
                last_rect.bottom() - first_rect.top()
            } / line_height)
                .round(),
        )?,
    })
}

pub(super) fn visual_layout(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
) -> Option<VisualLayout> {
    if !current_editor_target(actions, input) {
        return None;
    }
    let source = actions.source();
    let projection = actions.projection()?;
    let parent = input.parent_element()?;
    let paint = parent.query_selector(".editor-highlight-content").ok()??;
    let overlay = paint
        .parent_element()?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()?;
    sync_highlight_scroll(input, &overlay);
    let painted = paint.text_content()?;
    let start: usize = paint.get_attribute("data-textarea-start")?.parse().ok()?;
    let end = start.checked_add(painted.encode_utf16().count())?;
    let start_byte = projection.textarea_to_byte(start);
    let end_byte = projection.textarea_to_byte(end);
    if !parent.class_list().contains("highlight-ready")
        || projection.byte_to_textarea(start_byte).ok()? != start
        || projection.byte_to_textarea(end_byte).ok()? != end
        || projection.text()[start_byte..end_byte]
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            != painted
    {
        return None;
    }
    let metrics = visual_metrics(actions, input)?;
    if metrics.rows == 0 {
        return None;
    }
    measured_layout(
        actions,
        input,
        &source,
        projection,
        Some(&metrics),
        |index| {
            parent
                .query_selector(&format!(".editor-source-line[data-line='{}']", index + 1))
                .ok()
                .flatten()
        },
    )
}

fn selected_lines(
    actions: EditorActions,
    source: &str,
    projection: &openwebide_core::editor::FoldProjection,
) -> Option<BTreeMap<usize, Vec<(usize, usize)>>> {
    let mut selected = BTreeMap::<usize, Vec<(usize, usize)>>::new();
    for (caret, selection) in actions.selections(source).iter().enumerate() {
        let visible = projection.visible_selection(*selection).ok()?;
        let index = projection
            .lines()
            .partition_point(|line| line.visible_start <= visible.head)
            .saturating_sub(1);
        for row in index.saturating_sub(1)..=(index + 1).min(projection.lines().len() - 1) {
            selected.entry(row).or_default();
        }
        selected
            .entry(index)
            .or_default()
            .push((caret, visible.head));
    }
    Some(selected)
}

struct RowMeasurement<'a> {
    actions: EditorActions,
    source: &'a str,
    projection: &'a openwebide_core::editor::FoldProjection,
    identity: &'a str,
    line_height: f64,
}
impl RowMeasurement<'_> {
    fn sample(
        &self,
        index: usize,
        cursors: &[(usize, usize)],
        row: web_sys::Element,
    ) -> Option<(openwebide_core::editor::VisualLineRows, Vec<VisualCaret>)> {
        use openwebide_core::editor::{VisualLineRows, visual_probe_rows, visual_row_id};
        let line = &self.projection.lines()[index];
        let raw = &self.source[line.source.clone()];
        let body = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        let painted = row.text_content()?;
        if painted.strip_suffix('\n').unwrap_or(&painted) != body.replace('\r', "\n") {
            return None;
        }
        let bounds = row.get_bounding_client_rect();
        let rows = measured_row((bounds.height() / self.line_height).round())?;
        if rows == 0 || (bounds.height() - rows as f64 * self.line_height).abs() > 0.5 {
            return None;
        }
        let metrics = VisualMetrics {
            identity: self.identity.into(),
            left: bounds.left(),
            top: bounds.top(),
            line_height: self.line_height,
            caret_height: self.line_height,
            caret_inset: 0.0,
            rows,
        };
        let mut glyphs = if body.is_empty() {
            None
        } else {
            Some(Glyphs::new(&row, body)?)
        };
        let mut current = Vec::new();
        for &(caret, head) in cursors {
            let within =
                if let Some(goal) = self.actions.visual_caret(self.source, caret, self.identity) {
                    goal.row.checked_sub(line.visible_start)?
                } else if body.is_empty() {
                    0
                } else if body.ends_with('\r') && head == line.visible_start + body.len() {
                    rows - 1
                } else {
                    let glyphs = glyphs.as_mut()?;
                    let byte = head
                        .saturating_sub(line.visible_start)
                        .min(glyphs.offsets.last()?.0);
                    let glyph = glyphs
                        .offsets
                        .partition_point(|(at, _)| *at <= byte)
                        .saturating_sub(1)
                        .min(glyphs.offsets.len() - 2);
                    glyphs.row(glyph, &metrics)?
                };
            current.push(within);
        }
        let mut carets = Vec::new();
        for wanted in visual_probe_rows(rows, &current).ok()? {
            let id = visual_row_id(self.projection, index, wanted).ok()?;
            if body.is_empty() || (body.ends_with('\r') && wanted + 1 == rows) {
                carets.push(VisualCaret {
                    offset: line.visible_start + body.len(),
                    column: 0,
                    row: id,
                });
                continue;
            }
            let glyphs = glyphs.as_mut()?;
            let start = glyphs.first_on_row(wanted, &metrics)?;
            let end = glyphs.first_on_row(wanted + 1, &metrics)?;
            if carets.len() + (end - start) * 2 > MAX_VISUAL_CARETS {
                return None;
            }
            for glyph in start..end {
                let rect = glyphs.rect(glyph)?;
                for (offset, x) in [
                    (glyphs.offsets[glyph].0, rect.left()),
                    (glyphs.offsets[glyph + 1].0, rect.right()),
                ] {
                    carets.push(VisualCaret {
                        offset: line.visible_start + offset,
                        column: measured_integer(((x - bounds.left()) * 64.0).round())?,
                        row: id,
                    });
                }
            }
        }
        Some((VisualLineRows { line: index, rows }, carets))
    }
}

fn measurement_context<'a>(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    source: &'a str,
    projection: &'a openwebide_core::editor::FoldProjection,
    identity: &'a str,
) -> Option<RowMeasurement<'a>> {
    let style = window().get_computed_style(input).ok()??;
    let line_height: f64 = style
        .get_property_value("line-height")
        .ok()?
        .trim_end_matches("px")
        .parse()
        .ok()?;
    if !line_height.is_finite() || line_height <= 0.0 {
        return None;
    }
    Some(RowMeasurement {
        actions,
        source,
        projection,
        identity,
        line_height,
    })
}

fn measured_layout(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    source: &str,
    projection: openwebide_core::editor::FoldProjection,
    global: Option<&VisualMetrics>,
    row_element: impl Fn(usize) -> Option<web_sys::Element>,
) -> Option<VisualLayout> {
    let geometry = super::editor_rows::metrics_identity(input)?;
    let revision = actions.view_revision();
    let identity = format!("{geometry}:{revision}");
    let context = measurement_context(actions, input, source, &projection, &identity)?;
    let mut lines = Vec::new();
    let mut carets = Vec::new();
    for (index, cursors) in selected_lines(actions, source, &projection)? {
        let row = row_element(projection.lines()[index].source_line)?;
        if let (Some(global), Some(measured)) = (global, actions.measured_rows())
            && (row.get_bounding_client_rect().top() - global.top - measured.rows.top(index)?).abs()
                > 0.5
        {
            return None;
        }
        let (line, sampled) = context.sample(index, &cursors, row)?;
        lines.push(line);
        carets.extend(sampled);
        if carets.len() > MAX_VISUAL_CARETS {
            return None;
        }
    }
    if actions.view_revision() != revision
        || super::editor_rows::metrics_identity(input).as_ref() != Some(&geometry)
        || !current_editor_target(actions, input)
    {
        return None;
    }
    VisualLayout::neighborhood(source, projection, identity, &lines, carets).ok()
}

/// Cold measurement uses the same exact row sampler as warm paint, with bounded
/// temporary DOM batches. No document-height table is needed for neighbor links.
pub(super) fn neighborhood_layout(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    render: impl Fn(&[usize], bool) -> String,
) -> Option<VisualLayout> {
    if !current_editor_target(actions, input)
        || input.client_width() <= 0
        || input.client_height() <= 0
    {
        return None;
    }
    let revision = actions.view_revision();
    let geometry = super::editor_rows::metrics_identity(input)?;
    let identity = format!("{geometry}:{revision}");
    let source = actions.source();
    let projection = actions.projection()?;
    let selected = selected_lines(actions, &source, &projection)?
        .into_iter()
        .collect::<Vec<_>>();
    let context = measurement_context(actions, input, &source, &projection, &identity)?;
    let (_probe, paint) = super::editor_rows::styled_row_probe(input).ok()?;
    let mut lines = Vec::new();
    let mut carets = Vec::new();
    let mut start = 0;
    while start < selected.len() {
        let count = openwebide_core::editor::row_measurement_batch(
            selected[start..]
                .iter()
                .map(|(index, _)| projection.lines()[*index].source.len()),
        );
        if count == 0 {
            return None;
        }
        let batch = &selected[start..start + count];
        let indices = batch
            .iter()
            .map(|(index, _)| projection.lines()[*index].source_line)
            .collect::<Vec<_>>();
        let suffix = batch.last()?.0 + 1 < projection.lines().len();
        paint.set_inner_html(&render(&indices, suffix));
        for (index, cursors) in batch {
            let row = paint
                .query_selector(&format!(
                    ".editor-source-line[data-line='{}']",
                    projection.lines()[*index].source_line + 1
                ))
                .ok()??;
            let (line, sampled) = context.sample(*index, cursors, row)?;
            lines.push(line);
            carets.extend(sampled);
            if carets.len() > MAX_VISUAL_CARETS {
                return None;
            }
        }
        paint.set_inner_html("");
        start += count;
    }
    if actions.view_revision() != revision
        || super::editor_rows::metrics_identity(input).as_ref() != Some(&geometry)
        || !current_editor_target(actions, input)
    {
        return None;
    }
    VisualLayout::neighborhood(&source, projection, identity, &lines, carets).ok()
}

/// Translate a line-relative caret ID only when painting into a real viewport.
pub(super) fn caret_top(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    metrics: &VisualMetrics,
    id: usize,
) -> Option<f64> {
    let projection = actions.projection()?;
    let index = projection
        .lines()
        .partition_point(|line| line.visible_start <= id)
        .checked_sub(1)?;
    let within = id.checked_sub(projection.lines()[index].visible_start)?;
    let top = if let Some(rows) = actions.measured_rows() {
        metrics.top + rows.rows.top(index)?
    } else {
        input
            .parent_element()?
            .query_selector(&format!(
                ".editor-source-line[data-line='{}']",
                projection.lines()[index].source_line + 1
            ))
            .ok()??
            .get_bounding_client_rect()
            .top()
    };
    Some(top + within as f64 * metrics.line_height + metrics.caret_inset)
}
