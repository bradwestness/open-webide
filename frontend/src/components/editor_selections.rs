//! Secondary selections reuse syntax paint metrics; editing stays in the facade.
use super::editor::{caret_rect, current_editor_target, text_position};
use crate::{state::workspace::WorkspaceState, state_actions::editor::EditorActions};
use leptos::prelude::*;

#[derive(Clone)]
struct Mark {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    caret: bool,
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
        actions.selections(&actions.source()).len().max(1)
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
        let selections = actions.selections(&source);
        leptos::leptos_dom::helpers::queue_microtask(move || {
            if marks.is_disposed()
                || workspace.pending_epoch.get_untracked() != epoch
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
            let mut next = Vec::new();
            let mut visual_primary = false;
            let metrics = (ready && actions.preferences().word_wrap)
                .then(|| super::editor_geometry::visual_metrics(actions, &input))
                .flatten();
            if ready
                && selections.len() > 1
                && let Some(parent) = input.parent_element()
                && let Some(projection) = actions.projection()
            {
                let viewport = parent.get_bounding_client_rect();
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
                        });
                        continue;
                    }
                    if index == 0 {
                        continue;
                    }
                    for line in projection.lines() {
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
                        let Ok(Some(row)) = parent.query_selector(&format!(
                            ".editor-source-line[data-line='{}']",
                            line.source_line + 1
                        )) else {
                            continue;
                        };
                        let bounds = row.get_bounding_client_rect();
                        if bounds.bottom() < viewport.top() || bounds.top() > viewport.bottom() {
                            continue;
                        }
                        let start = range.start.max(line.source.start);
                        let end = range.end.min(line.source.end);
                        let body = &source[line.source.clone()];
                        let column = u32::try_from(
                            openwebide_core::editor::byte_to_textarea(
                                body,
                                start - line.source.start,
                            )
                            .unwrap_or(0),
                        )
                        .unwrap_or(u32::MAX);
                        let add = |rect: web_sys::DomRect, caret: bool, next: &mut Vec<Mark>| {
                            if rect.height() > 0.0 {
                                next.push(Mark {
                                    left: rect.left() - viewport.left(),
                                    top: rect.top() - viewport.top(),
                                    width: if caret { 2.0 } else { rect.width().max(2.0) },
                                    height: rect.height(),
                                    caret,
                                });
                            }
                        };
                        if contains_caret {
                            let rect = caret_rect(&row, column)
                                .filter(|rect| rect.height() > 0.0)
                                .unwrap_or(bounds);
                            add(rect, true, &mut next);
                        } else {
                            let mut start_column = column;
                            let mut end_column = u32::try_from(
                                openwebide_core::editor::byte_to_textarea(
                                    body,
                                    end - line.source.start,
                                )
                                .unwrap_or(0),
                            )
                            .unwrap_or(u32::MAX);
                            if let Some((node, at)) = text_position(row.as_ref(), &mut start_column)
                                && let Some((end_node, end_at)) =
                                    text_position(row.as_ref(), &mut end_column)
                                && let Ok(dom_range) = document().create_range()
                                && dom_range.set_start(&node, at).is_ok()
                                && dom_range.set_end(&end_node, end_at).is_ok()
                                && let Some(rects) = dom_range.get_client_rects()
                            {
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
            let _ = input
                .class_list()
                .toggle_with_force("editor-visual-carets", visual_primary);
            marks.set(merge_marks(next));
        });
    });
    view! {
        <div class="editor-selection-layer" aria-hidden="true">{move || marks.get().into_iter().map(|mark| view! {
            <span class=if mark.caret { "editor-secondary-caret" } else { "editor-secondary-selection" }
                style=format!("left:{}px;top:{}px;width:{}px;height:{}px", mark.left, mark.top, mark.width, mark.height)/>
        }).collect_view()}</div>
        <span class="sr-only" role="status" aria-live="polite">{move || format!("{} editor cursor{}", count.get(), if count.get() == 1 { "" } else { "s" })}</span>
    }
}
