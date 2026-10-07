//! Source selections reuse syntax paint metrics; editing stays in the facade.
use super::editor::{caret_rect, current_editor_target};
use super::editor_paint;
use crate::{state::workspace::WorkspaceState, state_actions::editor::EditorActions};
use leptos::prelude::*;

#[derive(Clone)]
struct Mark {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    caret: bool,
    primary: bool,
}

// DOM ranges include both token spans and their text rectangles. Merge each
// line band so translucent selection paint has the same opacity across tokens.
fn merge_marks(marks: Vec<Mark>) -> Vec<Mark> {
    let (mut carets, mut ranges): (Vec<_>, Vec<_>) = marks.into_iter().partition(|mark| mark.caret);
    ranges.sort_by(|first, second| {
        first
            .top
            .total_cmp(&second.top)
            .then(first.height.total_cmp(&second.height))
            .then(first.left.total_cmp(&second.left))
    });
    let mut merged: Vec<Mark> = Vec::new();
    for mark in ranges {
        if let Some(previous) = merged.last_mut()
            && (previous.top - mark.top).abs() < 0.5
            && (previous.height - mark.height).abs() < 0.5
            && previous.primary == mark.primary
            && mark.left <= previous.left + previous.width + 0.5
        {
            previous.width =
                (previous.left + previous.width).max(mark.left + mark.width) - previous.left;
        } else {
            merged.push(mark);
        }
    }
    carets.extend(merged);
    carets
}

#[component]
pub(super) fn SelectionOverlay(
    textarea: NodeRef<leptos::html::Textarea>,
    ready: RwSignal<bool>,
    layout_revision: RwSignal<u64>,
) -> impl IntoView {
    let workspace = expect_context::<WorkspaceState>();
    let actions = EditorActions::new(workspace);
    let marks = RwSignal::new(Vec::<Mark>::new());
    let count = Memo::new(move |_| {
        workspace.editor_documents.track();
        workspace.content.track();
        workspace.open_file.track();
        workspace.active_project.track();
        actions.selection_count().max(1)
    });
    Effect::new(move || {
        workspace.editor_documents.track();
        layout_revision.track();
        let ready = ready.get();
        let source = workspace.content.get();
        let key = workspace
            .active_project
            .get()
            .zip(workspace.open_file.get());
        let epoch = workspace.pending_epoch.get();
        let read_revision = workspace.editor_read_revision.get();
        let account_generation = actions.account_generation();
        let selections = actions.selections(&source);
        leptos::leptos_dom::helpers::queue_microtask(move || {
            if marks.is_disposed()
                || workspace.pending_epoch.get_untracked() != epoch
                || workspace.editor_read_revision.get_untracked() != read_revision
                || actions.account_generation() != account_generation
                || workspace
                    .active_project
                    .get_untracked()
                    .zip(workspace.open_file.get_untracked())
                    != key
                || actions.source() != source
                || actions.selections(&source) != selections
            {
                return;
            }
            let Some(Some(input)) = textarea.try_get_untracked() else {
                return;
            };
            if !current_editor_target(actions, &input) {
                return;
            }
            let painted_scope = input
                .parent_element()
                .and_then(|parent| {
                    parent
                        .query_selector(".editor-highlight-content")
                        .ok()
                        .flatten()
                })
                .and_then(|paint| paint.get_attribute("data-editor-scope"));
            if painted_scope != input.get_attribute("data-editor-scope") {
                let _ = input.class_list().remove_1("editor-visual-carets");
                let _ = input.class_list().remove_1("editor-source-selections");
                marks.set(Vec::new());
                return;
            }
            let mut next = Vec::new();
            let mut visual_primary = false;
            let metrics = (ready && actions.preferences().word_wrap)
                .then(|| super::editor_geometry::visual_metrics(actions, &input))
                .flatten();
            if ready
                && let Some(parent) = input.parent_element()
                && let Some(projection) = actions.projection()
            {
                let viewport = parent.get_bounding_client_rect();
                let rows = parent
                    .query_selector_all(".editor-highlight .editor-source-line")
                    .ok();
                for (index, selection) in selections.iter().enumerate() {
                    let range = selection.range();
                    if range.is_empty()
                        && let Some(metrics) = &metrics
                        && let Some(caret) = actions.visual_caret(&source, index, &metrics.identity)
                        && let Some(top) =
                            super::editor_geometry::caret_top(actions, &input, metrics, caret.row)
                    {
                        visual_primary |= index == 0;
                        next.push(Mark {
                            left: metrics.left + caret.column as f64 / 64.0 - viewport.left(),
                            top: top - viewport.top(),
                            width: 2.0,
                            height: metrics.caret_height,
                            caret: true,
                            primary: index == 0,
                        });
                        continue;
                    }
                    let Some(rows) = rows.as_ref() else {
                        continue;
                    };
                    for row_index in 0..rows.length() {
                        let Some(row) = rows.item(row_index).and_then(|row| {
                            web_sys::wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(row).ok()
                        }) else {
                            continue;
                        };
                        let Some(source_line) = row
                            .get_attribute("data-line")
                            .and_then(|line| line.parse::<usize>().ok())
                            .and_then(|line| line.checked_sub(1))
                        else {
                            continue;
                        };
                        let line_index = projection
                            .lines()
                            .partition_point(|line| line.source_line < source_line);
                        let Some(line) = projection
                            .lines()
                            .get(line_index)
                            .filter(|line| line.source_line == source_line)
                        else {
                            continue;
                        };
                        let contains_caret = range.is_empty()
                            && range.start >= line.source.start
                            && (range.start < line.source.end
                                || range.start == source.len()
                                    && projection
                                        .lines()
                                        .last()
                                        .is_some_and(|last| last.source_line == line.source_line));
                        if !contains_caret
                            && (range.is_empty()
                                || range.start >= line.source.end
                                || range.end <= line.source.start)
                        {
                            continue;
                        }
                        let bounds = row.get_bounding_client_rect();
                        if bounds.bottom() < viewport.top() || bounds.top() > viewport.bottom() {
                            continue;
                        }
                        let start = range.start.max(line.source.start);
                        let end = range.end.min(line.source.end);
                        let native_column = |at| {
                            projection
                                .byte_to_textarea(line.visible_start + at - line.source.start)
                                .ok()
                                .and_then(|offset| offset.checked_sub(line.textarea_start))
                                .and_then(|offset| u32::try_from(offset).ok())
                        };
                        let Some(column) = native_column(start) else {
                            continue;
                        };
                        let add = |rect: web_sys::DomRect, caret: bool, next: &mut Vec<Mark>| {
                            if rect.height() > 0.0 {
                                next.push(Mark {
                                    left: rect.left() - viewport.left(),
                                    top: rect.top() - viewport.top(),
                                    width: if caret { 2.0 } else { rect.width().max(2.0) },
                                    height: rect.height(),
                                    caret,
                                    primary: index == 0,
                                });
                            }
                        };
                        if contains_caret {
                            if !editor_paint::covers(&row, column) {
                                continue;
                            }
                            let rect = caret_rect(&row, column)
                                .filter(|rect| rect.height() > 0.0)
                                .unwrap_or(bounds);
                            visual_primary |= index == 0;
                            add(rect, true, &mut next);
                        } else {
                            let Some(end_column) = native_column(end) else {
                                continue;
                            };
                            for dom_range in editor_paint::ranges(&row, column..end_column) {
                                if let Some(rects) = dom_range.get_client_rects() {
                                    for index in 0..rects.length() {
                                        if let Some(rect) = rects.item(index) {
                                            add(rect, false, &mut next);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let _ = input
                .class_list()
                .toggle_with_force("editor-visual-carets", visual_primary);
            let primary_selection = next.iter().any(|mark| mark.primary && !mark.caret);
            let _ = input
                .class_list()
                .toggle_with_force("editor-source-selections", primary_selection);
            marks.set(merge_marks(next));
        });
    });
    view! {
        <div class="editor-selection-layer" aria-hidden="true">{move || marks.get().into_iter().map(|mark| view! {
            <span class=match (mark.primary, mark.caret) {
                (true, true) => "editor-primary-caret", (true, false) => "editor-primary-selection",
                (false, true) => "editor-secondary-caret", (false, false) => "editor-secondary-selection",
            }
                data-primary=mark.primary.then_some("true")
                style=format!("left:{}px;top:{}px;width:{}px;height:{}px", mark.left, mark.top, mark.width, mark.height)/>
        }).collect_view()}</div>
        <span class="sr-only" role="status" aria-live="polite">{move || format!("{} editor cursor{}", count.get(), if count.get() == 1 { "" } else { "s" })}</span>
    }
}
