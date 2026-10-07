//! Thin frame/paint/measurement adapter for the shared ordered motion queue.
use super::{
    editor::{EditorPaint, current_editor_target, render_editor_selection, reveal_editor_caret},
    editor_geometry::visual_layout,
};
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::{SelectionError, SelectionMotion};

#[derive(Clone, Copy)]
pub(super) struct MotionAdapter {
    pub actions: EditorActions,
    pub paint: RwSignal<Option<EditorPaint>>,
    pub error: RwSignal<Option<String>>,
}
impl MotionAdapter {
    pub fn queue(
        self,
        input: &web_sys::HtmlTextAreaElement,
        motion: SelectionMotion,
        extend: bool,
    ) {
        let Some(project) = input
            .get_attribute("data-editor-project")
            .and_then(|project| project.parse().ok())
        else {
            return;
        };
        let Some(path) = input.get_attribute("data-editor-path") else {
            return;
        };
        match self
            .actions
            .queue_motion(project, &path, &self.actions.source(), motion, extend)
        {
            Ok(Some((ticket, true))) => self.schedule(ticket, input.clone()),
            Err(error) => self.error.set(Some(error.to_string())),
            _ => {}
        }
    }
    pub fn page(self, input: &web_sys::HtmlTextAreaElement, down: bool, extend: bool) {
        let Some(project) = input
            .get_attribute("data-editor-project")
            .and_then(|project| project.parse().ok())
        else {
            return;
        };
        let Some(path) = input.get_attribute("data-editor-path") else {
            return;
        };
        let viewport = (
            f64::from(crate::viewport::editor_scroll(input).client_height()),
            super::editor::editor_row_height(input),
        );
        match self.actions.queue_page_motion(
            project,
            &path,
            &self.actions.source(),
            down,
            extend,
            viewport,
        ) {
            Ok(Some((ticket, true))) => self.schedule(ticket, input.clone()),
            Err(error) => self.error.set(Some(error.to_string())),
            _ => {}
        }
    }

    fn schedule(self, ticket: u64, input: web_sys::HtmlTextAreaElement) {
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if self.error.is_disposed() || !current_editor_target(self.actions, &input) {
                self.actions.cancel_queued_motion(Some(ticket));
                return;
            }
            self.drain(ticket, &input, false);
        });
    }
    pub fn flush(self, input: &web_sys::HtmlTextAreaElement) -> bool {
        self.actions
            .queued_motion_ticket()
            .is_none_or(|ticket| self.drain(ticket, input, true))
    }
    fn fail(self, ticket: u64, error: SelectionError) -> bool {
        self.actions.cancel_queued_motion(Some(ticket));
        self.error.set(Some(error.to_string()));
        false
    }
    fn drain(self, ticket: u64, input: &web_sys::HtmlTextAreaElement, immediate: bool) -> bool {
        while let Some(request) = self.actions.next_queued_motion(ticket) {
            let mut layout = request
                .needs_layout()
                .then(|| visual_layout(self.actions, input))
                .flatten();
            if immediate
                && request.needs_layout()
                && layout.is_none()
                && let Some(paint) = self.paint.get_untracked()
            {
                paint.flush.run(());
                layout = visual_layout(self.actions, input);
            }
            if request.needs_layout()
                && layout.is_none()
                && let Some(paint) = self.paint.get_untracked()
            {
                layout = paint.neighborhood.run(());
            }
            if request.needs_layout() && layout.is_none() {
                if immediate {
                    return self.fail(ticket, SelectionError::LayoutUnavailable);
                }
                if let Err(error) = self.actions.wait_for_motion_layout(ticket) {
                    return self.fail(ticket, error);
                }
                self.schedule(ticket, input.clone());
                return false;
            }
            match self.actions.apply_queued_motion(ticket, layout.as_ref()) {
                Ok(Some(selections)) => {
                    self.error.set(None);
                    if let Some(selection) = selections.first() {
                        render_editor_selection(self.actions, input, *selection, false);
                        reveal_editor_caret(self.actions, input, self.paint);
                    }
                }
                Ok(None) => return true,
                Err(error) => return self.fail(ticket, error),
            }
        }
        true
    }
}
