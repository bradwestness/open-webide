//! Thin DOM measurements for the shared row-height/window policy.
use super::editor::current_editor_target;
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::MeasuredRows;
use wasm_bindgen::JsCast;

pub(super) struct RowProbe(web_sys::HtmlElement);
impl Drop for RowProbe {
    fn drop(&mut self) {
        self.0.remove();
    }
}

/// One styled DOM primitive shared by height and cursor-neighborhood probes.
pub(super) fn styled_row_probe(
    input: &web_sys::HtmlTextAreaElement,
) -> Result<(RowProbe, web_sys::Element), ()> {
    let style = window()
        .get_computed_style(input)
        .map_err(|_| ())?
        .ok_or(())?;
    let probe = document()
        .create_element("div")
        .map_err(|_| ())?
        .dyn_into::<web_sys::HtmlElement>()
        .map_err(|_| ())?;
    probe.set_class_name("editor-highlight editor-row-measure");
    let gutter = input.offset_left();
    let width = crate::viewport::editor_scroll(input).client_width() + gutter;
    probe
        .set_attribute("style", &format!("position:fixed;left:-10000px;top:0;width:{width}px;height:auto;visibility:hidden;pointer-events:none;contain:layout style paint;--editor-gutter-width:{gutter}px"))
        .map_err(|_| ())?;
    for property in [
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
        "font-stretch",
        "font-variation-settings",
        "font-variant-caps",
        "font-variant-numeric",
        "line-height",
        "tab-size",
        "white-space",
        "overflow-wrap",
        "letter-spacing",
        "font-kerning",
        "font-feature-settings",
        "font-variant-ligatures",
    ] {
        probe
            .style()
            .set_property(
                property,
                &style.get_property_value(property).map_err(|_| ())?,
            )
            .map_err(|_| ())?;
    }
    let paint = document().create_element("div").map_err(|_| ())?;
    paint.set_class_name("editor-highlight-content");
    paint.set_attribute("style", &format!("transform:none;will-change:auto;min-height:0;min-width:0;width:{width}px;padding-left:{gutter}px;padding-right:{}", style.get_property_value("padding-right").map_err(|_| ())?)).map_err(|_| ())?;
    probe.append_child(&paint).map_err(|_| ())?;
    document()
        .body()
        .ok_or(())?
        .append_child(&probe)
        .map_err(|_| ())?;
    Ok((RowProbe(probe), paint))
}

/// CSS padding is a DOM primitive; dimensions and validity come from shared rows.
pub(super) fn document_extent(
    input: &web_sys::HtmlTextAreaElement,
    rows: &MeasuredRows,
) -> Option<openwebide_core::editor::DocumentExtent> {
    let style = window().get_computed_style(input).ok()??;
    let padding = |name| {
        style
            .get_property_value(name)
            .ok()?
            .trim_end_matches("px")
            .parse::<f64>()
            .ok()
    };
    rows.extent(
        padding("padding-right")? + padding("padding-left")?,
        padding("padding-top")? + padding("padding-bottom")?,
    )
}
fn extent_supported(input: &web_sys::HtmlTextAreaElement, rows: &MeasuredRows) -> bool {
    document_extent(input, rows)
        .is_some_and(|extent| crate::viewport::check_editor_extent(extent.width, extent.height))
}

/// Browser primitives only: styled HTML, exact rectangles and yielding. The
/// shared core chooses batch sizes and validates the completed height table;
/// the editor facade rechecks document/layout ownership before publication.
pub(super) async fn measure_batches(
    input: web_sys::HtmlTextAreaElement,
    scope: crate::state::workspace::EditorRowPaint,
    mut plan: openwebide_core::editor::RowMeasurementPlan,
    current: impl Fn() -> bool,
    progress: impl Fn(usize),
    geometry: impl Fn(usize, openwebide_core::editor::MeasuredRowGeometry),
    render: impl Fn(&[usize], bool) -> String,
) -> Result<Option<MeasuredRows>, ()> {
    if !current()
        || input
            .parent_element()
            .and_then(|parent| parent.get_attribute("data-editor-account"))
            .as_deref()
            != Some(scope.account_generation.to_string().as_str())
        || !input.is_connected()
        || crate::viewport::editor_scroll(&input).client_width() <= 0
        || crate::viewport::editor_scroll(&input).client_height() <= 0
    {
        return Ok(None);
    }
    let projection = &scope.projection;
    let metrics = &scope.metrics;
    let (probe, paint) = styled_row_probe(&input)?;
    probe
        .0
        .class_list()
        .add_1("editor-height-measure")
        .map_err(|_| ())?;
    for (name, value) in [
        ("data-measure-view", scope.view_revision),
        ("data-measure-layout", scope.layout_epoch),
        ("data-measure-font", scope.font_epoch),
        ("data-measure-read", scope.read_revision),
        ("data-measure-source", scope.epoch),
        ("data-measure-account", scope.account_generation),
    ] {
        probe
            .0
            .set_attribute(name, &value.to_string())
            .map_err(|_| ())?;
    }
    probe
        .0
        .set_attribute("data-measure-prepared", &scope.prepared_source.to_string())
        .map_err(|_| ())?;
    let lengths = projection
        .lines()
        .iter()
        .map(|line| line.source.len())
        .collect::<Vec<_>>();
    progress(plan.completed());
    let mut batches = 0_usize;
    while let Some(range) = plan.pending_batch(&lengths) {
        if !current() || !input.is_connected() || metrics_identity(&input).as_ref() != Some(metrics)
        {
            return Ok(None);
        }
        let start = range.start;
        let end = range.end;
        let count = range.len();
        let rows = projection.lines()[start..end]
            .iter()
            .map(|line| line.source_line)
            .collect::<Vec<_>>();
        paint.set_inner_html(&render(&rows, end < projection.lines().len()));
        let measured = paint
            .query_selector_all(".editor-source-line")
            .map_err(|_| ())?;
        if usize::try_from(measured.length()).ok() != Some(count) {
            return Err(());
        }
        let mut previous_bottom = None;
        let mut batch = Vec::with_capacity(count);
        let mut widths = Vec::with_capacity(count);
        for index in 0..measured.length() {
            let row: web_sys::Element = measured.item(index).ok_or(())?.unchecked_into();
            let bounds = row.get_bounding_client_rect();
            if previous_bottom.is_some_and(|bottom: f64| (bounds.top() - bottom).abs() > 0.25) {
                return Err(());
            }
            previous_bottom = Some(bounds.bottom());
            batch.push(bounds.height());
            widths.push(f64::from(row.scroll_width()));
            let logical = start + usize::try_from(index).map_err(|_| ())?;
            let line = &projection.lines()[logical];
            let end = projection
                .lines()
                .get(logical + 1)
                .map_or(projection.text().len(), |next| next.visible_start);
            let raw = &projection.text()[line.visible_start..end];
            let body = raw
                .strip_suffix("\r\n")
                .or_else(|| raw.strip_suffix('\n'))
                .unwrap_or(raw);
            let wrapped = input
                .parent_element()
                .is_some_and(|parent| parent.class_list().contains("editor-word-wrap"));
            if let Some(index) = projection.visual_line_index(logical)
                && let Some(measured) = super::editor_geometry::preparation_geometry(
                    &row, body, index, &bounds, wrapped,
                )
                && current()
            {
                geometry(logical, measured);
            }
        }
        if !plan.record_layout(range, &batch, &widths) {
            return Err(());
        }
        progress(plan.completed());
        // Release the previous batch before allowing another input/render task.
        paint.set_inner_html("");
        batches += 1;
        if end < projection.lines().len() {
            if batches.is_multiple_of(openwebide_core::editor::MAX_MEASURE_BATCHES_PER_FRAME) {
                crate::util::yield_frame().await;
            } else {
                crate::util::yield_task().await;
            }
        }
    }
    if !current() || !input.is_connected() || metrics_identity(&input).as_ref() != Some(metrics) {
        return Ok(None);
    }
    let rows = plan.finish().ok_or(())?;
    if !extent_supported(&input, &rows) {
        return Err(());
    }
    Ok(Some(rows))
}

pub(super) fn metrics_identity(input: &web_sys::HtmlTextAreaElement) -> Option<String> {
    let style = window().get_computed_style(input).ok()??;
    Some(format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        crate::viewport::editor_scroll(input).client_width(),
        crate::viewport::editor_font_identity(input),
        style.get_property_value("line-height").ok()?,
        style.get_property_value("tab-size").ok()?,
        style.get_property_value("white-space").ok()?,
        style.get_property_value("overflow-wrap").ok()?,
        style.get_property_value("padding-right").ok()?,
        input.offset_left(),
        style.get_property_value("letter-spacing").ok()?,
        style.get_property_value("font-kerning").ok()?,
        style.get_property_value("font-feature-settings").ok()?,
        style.get_property_value("font-variant-ligatures").ok()?
    ))
}

pub(super) fn update_measurements(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    overlay: &web_sys::HtmlElement,
    font_changed: bool,
    syntax: Option<(
        bool,
        std::sync::Arc<Vec<Vec<openwebide_core::highlight::Token>>>,
    )>,
    whitespace: bool,
) {
    if !current_editor_target(actions, input)
        || input
            .parent_element()
            .and_then(|parent| parent.get_attribute("data-editor-account"))
            .as_deref()
            != Some(actions.account_generation().to_string().as_str())
    {
        return;
    }
    if font_changed {
        actions.invalidate_measured_font();
    }
    if crate::viewport::editor_scroll(input).client_width() <= 0
        || crate::viewport::editor_scroll(input).client_height() <= 0
    {
        return;
    }
    let Some(projection) = actions.projection() else {
        return;
    };
    // Large or fragmented paints require full-source batch measurements. A
    // visible fragment cannot establish the dimensions of its omitted source.
    if openwebide_core::editor::needs_measured_batches(
        projection.lines().len(),
        projection.text().len(),
    ) {
        return;
    }
    let Some(metrics) = metrics_identity(input) else {
        return;
    };
    if let Some(cached) = actions.measured_rows() {
        if cached.metrics == metrics
            && cached.whitespace == whitespace
            && cached
                .syntax
                .as_ref()
                .zip(syntax.as_ref())
                .is_some_and(|(old, new)| old.0 == new.0 && std::sync::Arc::ptr_eq(&old.1, &new.1))
        {
            return;
        }
        actions.invalidate_measured_rows();
    }
    let revision = actions.view_revision();
    let Ok(Some(paint)) = overlay.query_selector(".editor-highlight-content") else {
        return;
    };
    if paint.get_attribute("data-editor-scope").as_deref()
        != Some(actions.projection_revision().to_string().as_str())
    {
        return;
    }
    let Ok(rows) = paint.query_selector_all(".editor-source-line") else {
        return;
    };
    if usize::try_from(rows.length()).ok() != Some(projection.lines().len()) {
        return;
    }
    let mut heights = Vec::with_capacity(projection.lines().len());
    let mut widths = Vec::with_capacity(projection.lines().len());
    let mut previous_bottom = None;
    for (index, line) in projection.lines().iter().enumerate() {
        let Some(row) = u32::try_from(index).ok().and_then(|index| rows.item(index)) else {
            return;
        };
        let row: web_sys::Element = row.unchecked_into();
        if row.get_attribute("data-line").as_deref()
            != Some((line.source_line + 1).to_string().as_str())
        {
            return;
        }
        let bounds = row.get_bounding_client_rect();
        if previous_bottom.is_some_and(|bottom: f64| (bounds.top() - bottom).abs() > 0.25) {
            return;
        }
        previous_bottom = Some(bounds.bottom());
        heights.push(bounds.height());
        widths.push(f64::from(row.scroll_width()));
    }
    let Some(rows) = MeasuredRows::layout(heights, widths) else {
        return;
    };
    if !extent_supported(input, &rows) {
        return;
    }
    actions.publish_measured_paint(revision, metrics, rows, syntax, whitespace);
}
