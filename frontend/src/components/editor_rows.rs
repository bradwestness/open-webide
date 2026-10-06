//! Thin DOM measurements for the shared row-height/window policy.
use super::editor::current_editor_target;
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::MeasuredRows;
use wasm_bindgen::JsCast;

struct RowProbe(web_sys::HtmlElement);
impl Drop for RowProbe {
    fn drop(&mut self) {
        self.0.remove();
    }
}

/// Browser primitives only: styled HTML, exact rectangles and yielding. The
/// shared core chooses batch sizes and validates the completed height table;
/// the editor facade rechecks document/layout ownership before publication.
pub(super) async fn measure_batches(
    input: web_sys::HtmlTextAreaElement,
    projection: openwebide_core::editor::FoldProjection,
    metrics: String,
    current: impl Fn() -> bool,
    progress: impl Fn(usize),
    render: impl Fn(&[usize], bool) -> String,
) -> Result<Option<MeasuredRows>, ()> {
    if !current()
        || !input.is_connected()
        || input.client_width() <= 0
        || input.client_height() <= 0
    {
        return Ok(None);
    }
    let style = window()
        .get_computed_style(&input)
        .map_err(|_| ())?
        .ok_or(())?;
    let probe = document()
        .create_element("div")
        .map_err(|_| ())?
        .dyn_into::<web_sys::HtmlElement>()
        .map_err(|_| ())?;
    probe.set_class_name("editor-highlight editor-row-measure");
    let gutter = input.offset_left();
    let width = input.client_width() + gutter;
    probe
        .set_attribute("style", &format!("position:fixed;left:-10000px;top:0;width:{width}px;height:auto;visibility:hidden;pointer-events:none;contain:layout style paint;--editor-gutter-width:{gutter}px"))
        .map_err(|_| ())?;
    for property in [
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
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
    let _probe = RowProbe(probe);
    let mut heights = Vec::with_capacity(projection.lines().len());
    let mut batches = 0_usize;
    while heights.len() < projection.lines().len() {
        if !current()
            || !input.is_connected()
            || metrics_identity(&input).as_ref() != Some(&metrics)
        {
            return Ok(None);
        }
        let start = heights.len();
        let count = openwebide_core::editor::row_measurement_batch(
            projection.lines()[start..]
                .iter()
                .map(|line| line.source.len()),
        );
        if count == 0 {
            return Err(());
        }
        let end = start + count;
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
        for index in 0..measured.length() {
            let row: web_sys::Element = measured.item(index).ok_or(())?.unchecked_into();
            let bounds = row.get_bounding_client_rect();
            if previous_bottom.is_some_and(|bottom: f64| (bounds.top() - bottom).abs() > 0.25) {
                return Err(());
            }
            previous_bottom = Some(bounds.bottom());
            batch.push(bounds.height());
        }
        MeasuredRows::new(batch.iter().copied()).ok_or(())?;
        heights.extend(batch);
        progress(heights.len());
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
    if !current() || !input.is_connected() || metrics_identity(&input).as_ref() != Some(&metrics) {
        return Ok(None);
    }
    let rows = MeasuredRows::new(heights).ok_or(())?;
    let expected = (rows.height() + 24.0).max(f64::from(input.client_height()));
    if (f64::from(input.scroll_height()) - expected).abs() > 2.0 {
        return Err(());
    }
    Ok(Some(rows))
}

pub(super) fn metrics_identity(input: &web_sys::HtmlTextAreaElement) -> Option<String> {
    let style = window().get_computed_style(input).ok()??;
    Some(format!(
        "{}:{}:{}:{}:{}:{}:{}",
        input.client_width(),
        style.get_property_value("font").ok()?,
        style.get_property_value("line-height").ok()?,
        style.get_property_value("tab-size").ok()?,
        style.get_property_value("white-space").ok()?,
        style.get_property_value("overflow-wrap").ok()?,
        style.get_property_value("padding-right").ok()?
    ))
}

pub(super) fn update_measurements(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
    overlay: &web_sys::HtmlElement,
    font_changed: bool,
) {
    if !current_editor_target(actions, input) {
        return;
    }
    if font_changed {
        actions.invalidate_measured_rows();
    }
    if input.client_width() <= 0 || input.client_height() <= 0 {
        return;
    }
    let Some(projection) = actions.projection() else {
        return;
    };
    if !actions.preferences().word_wrap && projection.has_uniform_rows() {
        return;
    }
    let Some(metrics) = metrics_identity(input) else {
        return;
    };
    if let Some(cached) = actions.measured_rows() {
        if cached.metrics == metrics {
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
    }
    let Some(rows) = MeasuredRows::new(heights) else {
        return;
    };
    let expected_height = (rows.height() + 24.0).max(f64::from(input.client_height()));
    // Refuse to virtualize a paint surface that disagrees with native layout,
    // including a browser's physical scroll-height limit.
    if (f64::from(input.scroll_height()) - expected_height).abs() > 2.0 {
        return;
    }
    actions.publish_measured_rows(revision, metrics, rows);
}
