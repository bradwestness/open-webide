//! Browser text measurements for shared visual-row cursor movement.
use super::editor::{current_editor_target, sync_highlight_scroll};
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::{MAX_VISUAL_CARETS, VisualCaret, VisualLayout, VisualLineIndex};
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
                let length = node.dyn_ref::<web_sys::Text>()?.length();
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

fn range_rectangle(
    range: &web_sys::Range,
    nodes: &TextNodes,
    start: usize,
    end: usize,
) -> Option<web_sys::DomRect> {
    let (start_node, start_at) = nodes.position(u32::try_from(start).ok()?)?;
    let (end_node, end_at) = nodes.position(u32::try_from(end).ok()?)?;
    range.set_start(start_node, start_at).ok()?;
    range.set_end(end_node, end_at).ok()?;
    let rects = range.get_client_rects()?;
    (0..rects.length())
        .filter_map(|index| rects.item(index))
        .find(|rect| rect.height() > 0.0)
}

struct Glyphs<'a> {
    body: &'a str,
    index: VisualLineIndex,
    nodes: TextNodes,
    range: web_sys::Range,
    measured: BTreeMap<usize, web_sys::DomRect>,
}
impl<'a> Glyphs<'a> {
    fn new(row: &web_sys::Element, body: &'a str, index: Option<VisualLineIndex>) -> Option<Self> {
        Some(Self {
            body,
            index: index.or_else(|| VisualLineIndex::new(body))?,
            nodes: TextNodes::new(row)?,
            range: document().create_range().ok()?,
            measured: BTreeMap::new(),
        })
    }
    fn at(&self, glyph: usize) -> Option<(usize, usize)> {
        self.index.at(self.body, glyph)
    }
    fn rect(&mut self, index: usize) -> Option<web_sys::DomRect> {
        if let Some(rect) = self.measured.get(&index) {
            return Some(rect.clone());
        }
        if self.measured.len() >= MAX_VISUAL_CARETS {
            return None;
        }
        let rect = range_rectangle(
            &self.range,
            &self.nodes,
            self.at(index)?.1,
            self.at(index + 1)?.1,
        )?;
        self.measured.insert(index, rect.clone());
        Some(rect)
    }
    fn row(&mut self, index: usize, metrics: &VisualMetrics) -> Option<usize> {
        measured_row(((self.rect(index)?.top() - metrics.top) / metrics.line_height).floor())
    }
    fn first_at_column(&mut self, minimum: f64, trailing: bool) -> Option<usize> {
        let mut first = 0;
        let mut last = self.index.len().checked_sub(1)?;
        while first < last {
            let middle = first + (last - first) / 2;
            let rect = self.rect(middle)?;
            let column = if trailing { rect.right() } else { rect.left() };
            if column < minimum {
                first = middle + 1;
            } else {
                last = middle;
            }
        }
        Some(first)
    }
    // Visual rows increase in source order even when a row contains bidi text.
    fn first_on_row(&mut self, minimum: usize, metrics: &VisualMetrics) -> Option<usize> {
        let mut start = 0;
        let mut end = self.index.len().checked_sub(1)?;
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

fn sample_geometry(
    glyphs: &mut Glyphs<'_>,
    bounds: &web_sys::DomRect,
    horizontal: bool,
) -> Option<openwebide_core::editor::MeasuredRowGeometry> {
    if !horizontal && !glyphs.index.source_paint_eligible() {
        return None;
    }
    use openwebide_core::editor::{
        GlyphRectangle, HorizontalGeometry, MAX_ROW_GEOMETRY_ANCHORS, MeasuredRowGeometry,
        WrappedGeometry,
    };
    let mut indices = glyphs.index.anchor_glyphs().collect::<Vec<_>>();
    indices.dedup();
    if indices.len() <= MAX_ROW_GEOMETRY_ANCHORS {
        let anchors = indices
            .into_iter()
            .map(|glyph| {
                let rect = glyphs.rect(glyph)?;
                Some(GlyphRectangle {
                    glyph,
                    left: rect.left() - bounds.left(),
                    top: rect.top() - bounds.top(),
                    width: rect.width(),
                    height: rect.height(),
                })
            })
            .collect::<Option<Vec<_>>>();
        anchors.and_then(|anchors| {
            if horizontal {
                HorizontalGeometry::new(
                    glyphs.index.len() - 1,
                    bounds.width(),
                    bounds.height(),
                    anchors,
                )
                .map(MeasuredRowGeometry::Horizontal)
            } else {
                WrappedGeometry::new(
                    glyphs.index.len() - 1,
                    bounds.width(),
                    bounds.height(),
                    anchors,
                )
                .map(MeasuredRowGeometry::Wrapped)
            }
        })
    } else {
        None
    }
}

/// Exact browser rectangles for one bounded source probe. The shared plan owns
/// global glyph numbering, overlap requirements and geometry admission.
pub(super) fn paragraph_rectangles(
    row: &web_sys::Element,
    body: &str,
    glyph_start: usize,
    targets: &[usize],
) -> Option<Vec<openwebide_core::editor::GlyphRectangle>> {
    if row.text_content()?.as_str() != body {
        return None;
    }
    let bounds = row.get_bounding_client_rect();
    // Continuation probes are bounded, and their overlap targets are dense.
    // Build coordinates once instead of repeatedly segmenting each sparse
    // checkpoint's prefix; keep general viewport queries on the sparse index.
    let offsets = openwebide_core::editor::visual_line_offsets(body).ok()?;
    let nodes = TextNodes::new(row)?;
    let range = document().create_range().ok()?;
    targets
        .iter()
        .map(|glyph| {
            let local = glyph.checked_sub(glyph_start)?;
            let start = offsets.get(local)?.1;
            let end = offsets.get(local.checked_add(1)?)?.1;
            let rect = range_rectangle(&range, &nodes, start, end)?;
            Some(openwebide_core::editor::GlyphRectangle {
                glyph: *glyph,
                left: rect.left() - bounds.left(),
                top: rect.top() - bounds.top(),
                width: rect.width(),
                height: rect.height(),
            })
        })
        .collect()
}

/// Reuse the styled logical row already laid out by the cold height probe.
pub(super) fn preparation_geometry(
    row: &web_sys::Element,
    body: &str,
    index: VisualLineIndex,
    bounds: &web_sys::DomRect,
    wrapped: bool,
) -> Option<openwebide_core::editor::MeasuredRowGeometry> {
    if !index.source_paint_eligible() {
        return None;
    }
    let text = row.text_content()?;
    if !openwebide_core::editor::textarea_value_matches(
        body,
        text.strip_suffix('\n').unwrap_or(&text),
    ) {
        return None;
    }
    let mut glyphs = Glyphs::new(row, body, Some(index))?;
    sample_geometry(&mut glyphs, bounds, !wrapped)
}

/// Retain the logical row's exact height while copying only its measured visual
/// interval. Cloning the DOM range preserves token/whitespace spans. Verify every
/// retained glyph after reshaping: paragraph-dependent bidi, tabs or ligatures
/// must never silently move text to a different source position.
fn window_paint_row(
    row: &web_sys::Element,
    body: &str,
    window: &openwebide_core::editor::RowPaintWindow,
    line_height: f64,
    index: Option<VisualLineIndex>,
    geometry: &mut Option<openwebide_core::editor::MeasuredRowGeometry>,
) -> Option<()> {
    let text = row.text_content()?;
    if !openwebide_core::editor::textarea_value_matches(
        body,
        text.strip_suffix('\n').unwrap_or(&text),
    ) {
        return None;
    }
    use openwebide_core::editor::RowPaintWindow;
    let bounds = row.get_bounding_client_rect();
    let mut glyphs = Glyphs::new(row, body, index)?;
    let (first, last, top, height, horizontal) = match window {
        RowPaintWindow::Wrapped(window) => {
            if (bounds.height() - window.height).abs() > 0.5 {
                return None;
            }
            let metrics = VisualMetrics {
                identity: String::new(),
                left: bounds.left(),
                top: bounds.top(),
                line_height,
                caret_height: line_height,
                caret_inset: 0.0,
                rows: measured_row((window.height / line_height).round())?,
            };
            (
                glyphs.first_on_row(window.rows.start, &metrics)?,
                glyphs.first_on_row(window.rows.end, &metrics)?,
                window.top,
                window.height,
                false,
            )
        }
        RowPaintWindow::Horizontal(columns) => {
            if (bounds.height() - line_height).abs() > 0.5 {
                return None;
            }
            (
                glyphs.first_at_column(bounds.left() + columns.start, true)?,
                glyphs.first_at_column(bounds.left() + columns.end, false)?,
                0.0,
                bounds.height(),
                true,
            )
        }
    };
    let sampled = sample_geometry(&mut glyphs, &bounds, horizontal);
    if horizontal && first == last {
        let original_style = row.get_attribute("style").unwrap_or_default();
        row.set_attribute(
            "style",
            &format!(
                "{original_style};height:{height}px;width:{}px",
                bounds.width()
            ),
        )
        .ok()?;
        row.set_inner_html("");
        row.set_attribute("data-paint-top", "0").ok()?;
        *geometry = sampled;
        return Some(());
    }
    if first == last || last - first > MAX_VISUAL_CARETS {
        return None;
    }
    let byte_start = glyphs.at(first)?.0;
    let byte_end = glyphs.at(last)?.0;
    let native_start = glyphs.at(first)?.1;
    let native_end = if last + 1 == glyphs.index.len() {
        row.get_attribute("data-paint-length")?.parse().ok()?
    } else {
        glyphs.at(last)?.1
    };
    let expected = (first..last)
        .map(|index| glyphs.rect(index))
        .collect::<Option<Vec<_>>>()?;
    let (start, at) = glyphs.nodes.position(u32::try_from(native_start).ok()?)?;
    let (end, to) = glyphs.nodes.position(u32::try_from(native_end).ok()?)?;
    let range = document().create_range().ok()?;
    range.set_start(start, at).ok()?;
    range.set_end(end, to).ok()?;
    let fragment = document().create_element("span").ok()?;
    fragment.set_class_name("editor-source-fragment");
    fragment
        .set_attribute("data-paint-start", &native_start.to_string())
        .ok()?;
    fragment
        .set_attribute("data-paint-end", &native_end.to_string())
        .ok()?;
    fragment
        .set_attribute(
            "style",
            &format!("position:absolute;left:0;right:0;top:{top}px;display:block"),
        )
        .ok()?;
    if horizontal {
        let gap = document().create_element("span").ok()?;
        gap.set_attribute("aria-hidden", "true").ok()?;
        gap.set_attribute(
            "style",
            &format!(
                "display:inline-block;width:{}px",
                expected.first()?.left() - bounds.left()
            ),
        )
        .ok()?;
        fragment.append_child(&gap).ok()?;
    }
    fragment
        .append_child(&range.clone_contents().ok()?.into())
        .ok()?;
    let original = row.inner_html();
    let original_style = row.get_attribute("style").unwrap_or_default();
    row.set_attribute(
        "style",
        &format!(
            "{original_style};height:{height}px;width:{}px",
            bounds.width()
        ),
    )
    .ok()?;
    row.set_inner_html("");
    let valid = (|| {
        row.append_child(&fragment).ok()?;
        let mut painted = Glyphs::new(&fragment, &body[byte_start..byte_end], None)?;
        for (index, old) in expected.iter().enumerate() {
            let new = painted.rect(index)?;
            if (new.left() - old.left()).abs() > 0.5
                || (new.top() - old.top()).abs() > 0.5
                || (new.width() - old.width()).abs() > 0.5
                || (new.height() - old.height()).abs() > 0.5
            {
                return None;
            }
        }
        ((row.get_bounding_client_rect().height() - bounds.height()).abs() <= 0.5).then_some(())
    })();
    if valid.is_none() {
        row.set_inner_html(&original);
        let _ = row.set_attribute("style", &original_style);
    } else {
        let _ = row.set_attribute("data-paint-top", &top.to_string());
        if horizontal {
            let _ = row.set_attribute(
                "data-paint-left",
                &(expected.first()?.left() - bounds.left()).to_string(),
            );
        }
    }
    if valid.is_some() {
        *geometry = sampled;
    }
    valid
}

/// Crop source before asking the browser for layout, using exact anchors from
/// this immutable styled row. Validate retained anchors and restore complete
/// source on any reshaping discrepancy.
fn anchored_row(
    row: &web_sys::Element,
    body: &str,
    index: &VisualLineIndex,
    window: &openwebide_core::editor::RowPaintWindow,
    line_height: f64,
    geometry: &openwebide_core::editor::MeasuredRowGeometry,
) -> Option<()> {
    let interval = geometry.source_interval(window, line_height)?;
    let original = row.inner_html();
    let original_style = row.get_attribute("style").unwrap_or_default();
    if interval.is_empty() {
        row.set_attribute(
            "style",
            &format!(
                "{original_style};height:{}px;width:{}px",
                geometry.height(),
                geometry.width()
            ),
        )
        .ok()?;
        row.set_inner_html("");
        row.set_attribute("data-paint-top", "0").ok()?;
        return Some(());
    }
    let (start_byte, start_native) = index.at(body, interval.start)?;
    let (end_byte, end_native) = index.at(body, interval.end)?;
    let source = body.get(start_byte..end_byte)?;
    if source.len() > openwebide_core::editor::MAX_MEASURE_BYTES {
        return None;
    }
    let end_native = if interval.end + 1 == index.len() {
        row.get_attribute("data-paint-length")?
            .parse::<usize>()
            .ok()?
    } else {
        end_native
    };
    let anchors = geometry.anchors(interval.clone())?;
    let left = anchors.first()?.left;
    let top = if matches!(window, openwebide_core::editor::RowPaintWindow::Wrapped(_)) {
        (anchors.first()?.top / line_height).floor() * line_height
    } else {
        0.0
    };
    let source_start = row
        .get_attribute("data-source-start")
        .map(|offset| offset.parse::<usize>().ok())
        .unwrap_or(Some(0))?;
    let nodes = TextNodes::new(row)?;
    let (start, at) =
        nodes.position(u32::try_from(start_native.checked_sub(source_start)?).ok()?)?;
    let (end, to) = nodes.position(u32::try_from(end_native.checked_sub(source_start)?).ok()?)?;
    let range = document().create_range().ok()?;
    range.set_start(start, at).ok()?;
    range.set_end(end, to).ok()?;
    let fragment = document().create_element("span").ok()?;
    fragment.set_class_name("editor-source-fragment");
    fragment
        .set_attribute("data-paint-start", &start_native.to_string())
        .ok()?;
    fragment
        .set_attribute("data-paint-end", &end_native.to_string())
        .ok()?;
    fragment
        .set_attribute(
            "style",
            &format!("position:absolute;left:0;right:0;top:{top}px;display:block"),
        )
        .ok()?;
    let gap = document().create_element("span").ok()?;
    gap.set_attribute("style", &format!("display:inline-block;width:{left}px"))
        .ok()?;
    fragment.append_child(&gap).ok()?;
    fragment
        .append_child(&range.clone_contents().ok()?.into())
        .ok()?;
    // No geometry read occurred while the complete row was attached.
    row.set_attribute(
        "style",
        &format!(
            "{original_style};height:{}px;width:{}px",
            geometry.height(),
            geometry.width()
        ),
    )
    .ok()?;
    row.set_inner_html("");
    let valid = (|| {
        row.append_child(&fragment).ok()?;
        let bounds = row.get_bounding_client_rect();
        let mut glyphs = Glyphs::new(&fragment, source, None)?;
        for anchor in anchors {
            let rect = glyphs.rect(anchor.glyph - interval.start)?;
            if (rect.left() - bounds.left() - anchor.left).abs() > 0.5
                || (rect.top() - bounds.top() - anchor.top).abs() > 0.5
                || (rect.width() - anchor.width).abs() > 0.5
                || (rect.height() - anchor.height).abs() > 0.5
            {
                return None;
            }
        }
        let full_length = row.get_attribute("data-paint-length")?;
        row.set_attribute(
            "data-paint-length",
            &(end_native - start_native).to_string(),
        )
        .ok()?;
        let mut ignored = None;
        let cropped = window_paint_row(row, source, window, line_height, None, &mut ignored);
        row.set_attribute("data-paint-length", &full_length).ok()?;
        cropped?;
        let fragments = row
            .query_selector_all(":scope > .editor-source-fragment")
            .ok()?;
        for at in 0..fragments.length() {
            let fragment = fragments.item(at)?.dyn_into::<web_sys::Element>().ok()?;
            for attribute in ["data-paint-start", "data-paint-end"] {
                let local = fragment.get_attribute(attribute)?.parse::<usize>().ok()?;
                fragment
                    .set_attribute(attribute, &(local + start_native).to_string())
                    .ok()?;
            }
        }
        Some(())
    })();
    if valid.is_none() {
        row.set_inner_html(&original);
        let _ = row.set_attribute("style", &original_style);
    }
    valid
}

/// A temporary styled probe supplies exact glyph boundaries. Source ownership,
/// layout identity and window policy come from the common editor facade.
pub(super) fn window_paint(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    render: impl Fn(&[crate::state_actions::editor::EditorRowSourceSlice]) -> String,
    windows: &[(usize, openwebide_core::editor::RowPaintWindow)],
    cache: &mut crate::state_actions::editor::EditorFragmentCache,
) -> Option<String> {
    if windows.is_empty() {
        return None;
    }
    if !current_editor_target(actions, input) {
        return None;
    }
    let identity = super::editor_rows::metrics_identity(input)?;
    if actions
        .measured_rows()
        .is_some_and(|measured| measured.metrics != identity)
    {
        return None;
    }
    let projection = actions.projection()?;
    let revision = actions.view_revision();
    let style = window().get_computed_style(input).ok()??;
    let line_height = style
        .get_property_value("line-height")
        .ok()?
        .trim_end_matches("px")
        .parse::<f64>()
        .ok()?;
    let source = actions.source();
    let (_probe, paint) = super::editor_rows::styled_row_probe(input).ok()?;
    if windows.iter().any(|(_, window)| {
        matches!(
            window,
            openwebide_core::editor::RowPaintWindow::Horizontal(_)
        )
    }) {
        paint
            .dyn_ref::<web_sys::HtmlElement>()?
            .style()
            .set_property(
                "width",
                &format!(
                    "{}px",
                    crate::viewport::editor_scroll(input).scroll_width() + input.offset_left()
                ),
            )
            .ok()?;
    }
    let slices = windows
        .iter()
        .filter_map(|(index, window)| actions.row_source_slice(cache, *index, window, line_height))
        .collect::<Vec<_>>();
    paint.set_inner_html(&render(&slices));
    let mut changed = false;
    for (index, window) in windows {
        let line = &projection.lines()[*index];
        let raw = &source[line.source.clone()];
        let body = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        let row = paint
            .query_selector(&format!(
                ".editor-source-line[data-line='{}']",
                line.source_line + 1
            ))
            .ok()??;
        if let Some(geometry) = actions.measured_row_geometry(cache, *index)
            && let Some(source_index) = projection.visual_line_index(*index)
            && anchored_row(&row, body, &source_index, window, line_height, &geometry).is_some()
        {
            changed = true;
            continue;
        }
        if slices
            .iter()
            .any(|slice| slice.source_line == line.source_line)
        {
            // Never measure a partial row as complete. The caller renders the full
            // source on a shaping mismatch; the next probe obtains fresh anchors.
            actions.forget_measured_row_geometry(cache, *index);
            return None;
        }
        let mut geometry = None;
        changed |= window_paint_row(
            &row,
            body,
            window,
            line_height,
            projection.visual_line_index(*index),
            &mut geometry,
        )
        .is_some();
        if let Some(geometry) = geometry {
            actions.retain_measured_row_geometry(cache, *index, geometry);
        }
    }
    if !changed
        || actions.view_revision() != revision
        || !current_editor_target(actions, input)
        || super::editor_rows::metrics_identity(input).as_ref() != Some(&identity)
    {
        return None;
    }
    Some(paint.inner_html())
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
    let caret_offset = first
        .query_selector(".editor-source-fragment")
        .ok()?
        .and_then(|fragment| fragment.get_attribute("data-paint-start"))
        .and_then(|offset| offset.parse().ok())
        .unwrap_or(0);
    let paint_top = first
        .get_attribute("data-paint-top")
        .and_then(|top| top.parse::<f64>().ok())
        .unwrap_or(0.0);
    let caret = super::editor::caret_rect(&first, caret_offset).filter(|rect| rect.height() > 0.0);
    Some(VisualMetrics {
        identity,
        left: first_rect.left(),
        top: first_rect.top() - window_top,
        line_height,
        caret_height: caret.as_ref().map_or(line_height, web_sys::DomRect::height),
        caret_inset: caret
            .as_ref()
            .map_or(0.0, |caret| caret.top() - first_rect.top() - paint_top),
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
        || !openwebide_core::editor::textarea_value_matches(
            &projection.text()[start_byte..end_byte],
            &painted,
        )
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
        if !openwebide_core::editor::textarea_value_matches(
            body,
            painted.strip_suffix('\n').unwrap_or(&painted),
        ) {
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
            Some(Glyphs::new(
                &row,
                body,
                self.projection.visual_line_index(index),
            )?)
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
                    let byte = head.saturating_sub(line.visible_start).min(body.len());
                    let glyph = glyphs
                        .index
                        .index_at_byte(body, byte)?
                        .min(glyphs.index.len() - 2);
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
                    (glyphs.at(glyph)?.0, rect.left()),
                    (glyphs.at(glyph + 1)?.0, rect.right()),
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
        || crate::viewport::editor_scroll(input).client_width() <= 0
        || crate::viewport::editor_scroll(input).client_height() <= 0
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

/// Reveal an omitted source caret using the same validated neighborhood layout
/// as wrapped movement, translated from probe coordinates into the viewport.
pub(super) fn layout_caret_rect(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    layout: &VisualLayout,
    offset: usize,
) -> Option<web_sys::DomRect> {
    if !current_editor_target(actions, input) {
        return None;
    }
    let metrics = visual_metrics(actions, input)?;
    let visible = actions.projection()?.visible_offset(offset).ok()?;
    let caret = layout.caret(visible)?;
    web_sys::DomRect::new_with_x_and_y_and_width_and_height(
        metrics.left + caret.column as f64 / 64.0,
        caret_top(actions, input, &metrics, caret.row)?,
        0.0,
        metrics.caret_height,
    )
    .ok()
}

/// A single source-point measurement also works when an unwrapped visual row
/// contains more glyphs than a movement neighborhood can admit.
pub(super) fn probe_caret_rect(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    offset: usize,
    render: impl Fn(&[usize], bool) -> String,
) -> Option<web_sys::DomRect> {
    if !current_editor_target(actions, input) {
        return None;
    }
    let identity = super::editor_rows::metrics_identity(input)?;
    let revision = actions.view_revision();
    let projection = actions.projection()?;
    let visible = projection.visible_offset(offset).ok()?;
    let index = projection
        .lines()
        .partition_point(|line| line.visible_start <= visible)
        .checked_sub(1)?;
    let line = &projection.lines()[index];
    let actual = input
        .parent_element()?
        .query_selector(&format!(
            ".editor-source-line[data-line='{}']",
            line.source_line + 1
        ))
        .ok()??
        .get_bounding_client_rect();
    let (_probe, paint) = super::editor_rows::styled_row_probe(input).ok()?;
    paint.set_inner_html(&render(
        &[line.source_line],
        index + 1 < projection.lines().len(),
    ));
    let row = paint.query_selector(".editor-source-line").ok()??;
    let column = projection
        .byte_to_textarea(visible)
        .ok()?
        .checked_sub(line.textarea_start)?;
    let caret = super::editor::caret_rect(&row, u32::try_from(column).ok()?)?;
    let bounds = row.get_bounding_client_rect();
    if actions.view_revision() != revision
        || !current_editor_target(actions, input)
        || super::editor_rows::metrics_identity(input).as_ref() != Some(&identity)
    {
        return None;
    }
    web_sys::DomRect::new_with_x_and_y_and_width_and_height(
        actual.left() + caret.left() - bounds.left(),
        actual.top() + caret.top() - bounds.top(),
        caret.width(),
        caret.height(),
    )
    .ok()
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
    } else if !actions.preferences().word_wrap && projection.has_uniform_rows() {
        // Fixed source-row heights also locate carets outside the painted window.
        metrics.top + f64::from(u32::try_from(index).ok()?) * metrics.line_height
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
