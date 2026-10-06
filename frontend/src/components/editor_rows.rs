//! Thin DOM measurements for the shared row-height/window policy.
use super::editor::current_editor_target;
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::MeasuredRows;
use wasm_bindgen::JsCast;

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
