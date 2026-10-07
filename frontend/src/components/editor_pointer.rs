//! Source pointer policy uses DOM hit coordinates through the shared editor facade.
use super::{
    editor::{current_editor_target, render_editor_selection},
    editor_motion::MotionAdapter,
};
use crate::{state::workspace::WorkspaceState, state_actions::editor::EditorActions};
use leptos::prelude::*;
use openwebide_core::editor::PointerSelection;

struct Gesture {
    project: i64,
    path: String,
    source: String,
    epoch: u64,
    read: u64,
    account: u64,
    pointer: PointerSelection,
    point: (f64, f64),
    #[cfg(target_arch = "wasm32")]
    time: f64,
    projection_revision: u64,
}

#[derive(Clone, Copy)]
pub(super) struct PointerAdapter {
    actions: EditorActions,
    workspace: WorkspaceState,
    input: NodeRef<leptos::html::Textarea>,
    ready: RwSignal<bool>,
    motion: MotionAdapter,
    gesture: StoredValue<Option<Gesture>>,
    generation: StoredValue<u64>,
}
impl PointerAdapter {
    pub fn new(
        actions: EditorActions,
        workspace: WorkspaceState,
        input: NodeRef<leptos::html::Textarea>,
        ready: RwSignal<bool>,
        motion: MotionAdapter,
    ) -> Self {
        let adapter = Self {
            actions,
            workspace,
            input,
            ready,
            motion,
            gesture: StoredValue::new(None),
            generation: StoredValue::new(0),
        };
        let movement =
            window_event_listener(leptos::ev::mousemove, move |event| adapter.drag(&event));
        let release = window_event_listener(leptos::ev::mouseup, move |_| {
            adapter.gesture.set_value(None);
        });
        let blur =
            window_event_listener(leptos::ev::blur, move |_| adapter.gesture.set_value(None));
        on_cleanup(move || {
            movement.remove();
            release.remove();
            blur.remove();
        });
        adapter
    }
    #[cfg(target_arch = "wasm32")]
    fn offset(
        self,
        input: &web_sys::HtmlTextAreaElement,
        x: f64,
        y: f64,
        dragging: bool,
    ) -> Option<usize> {
        let offset = if dragging {
            crate::viewport::editor_caret_from_drag_point(input, x, y)?
        } else {
            crate::viewport::editor_caret_from_point(input, x, y)?
        };
        let projection = self.actions.projection()?;
        projection
            .source_offset(projection.textarea_to_byte(offset as usize))
            .ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn offset(
        self,
        _input: &web_sys::HtmlTextAreaElement,
        _x: f64,
        _y: f64,
        _dragging: bool,
    ) -> Option<usize> {
        None
    }

    pub fn start(self, event: &web_sys::MouseEvent) {
        self.gesture.set_value(None);
        self.generation
            .update_value(|generation| *generation = generation.wrapping_add(1));
        if event.button() != 0
            || event.alt_key()
            || self.actions.is_composing()
            || !self.ready.get_untracked()
        {
            return;
        }
        let Some(input) = self
            .input
            .get_untracked()
            .filter(|input| current_editor_target(self.actions, input))
        else {
            return;
        };
        if !self.motion.flush(&input) {
            event.prevent_default();
            return;
        }
        let Some(offset) = self.offset(&input, event.client_x(), event.client_y(), false) else {
            return;
        };
        event.prevent_default();
        let (Some(project), Some(path)) = (
            self.workspace.active_project.get_untracked(),
            self.workspace.open_file.get_untracked(),
        ) else {
            return;
        };
        let source = self.actions.source();
        match self.actions.begin_pointer_selection(
            project,
            &path,
            &source,
            offset,
            event.detail().unsigned_abs(),
            event.shift_key(),
        ) {
            Ok(Some(pointer)) => {
                self.gesture.set_value(Some(Gesture {
                    project,
                    path,
                    source,
                    pointer,
                    epoch: self.workspace.pending_epoch.get_untracked(),
                    read: self.workspace.editor_read_revision.get_untracked(),
                    account: self.actions.account_generation(),
                    point: (event.client_x(), event.client_y()),
                    #[cfg(target_arch = "wasm32")]
                    time: js_sys::Date::now(),
                    projection_revision: self.workspace.editor_projection_revision.get_untracked(),
                }));
                self.render(&input);
                self.schedule(self.generation.get_value());
                let options = web_sys::FocusOptions::new();
                options.set_prevent_scroll(true);
                let _ = input.focus_with_options(&options);
            }
            Err(error) => self.motion.error.set(Some(error.to_string())),
            Ok(None) => {}
        }
    }
    fn render(self, input: &web_sys::HtmlTextAreaElement) {
        self.motion.error.set(None);
        if let Some(selection) = self.actions.selection(&self.actions.source()) {
            render_editor_selection(self.actions, input, selection, false);
        }
    }
    fn drag(self, event: &web_sys::MouseEvent) {
        if !self.gesture.with_value(Option::is_some) {
            return;
        }
        if event.buttons() & 1 == 0 {
            self.gesture.set_value(None);
            return;
        }
        let Some(input) = self
            .input
            .get_untracked()
            .filter(|input| current_editor_target(self.actions, input))
        else {
            self.gesture.set_value(None);
            return;
        };
        if self.stale() {
            self.gesture.set_value(None);
            return;
        }
        self.gesture.update_value(|gesture| {
            if let Some(gesture) = gesture {
                gesture.point = (event.client_x(), event.client_y());
            }
        });
        event.prevent_default();
        self.select_at(&input, event.client_x(), event.client_y());
    }
    fn stale(self) -> bool {
        self.gesture.with_value(|gesture| {
            gesture.as_ref().is_some_and(|gesture| {
                self.actions.is_composing()
                    || self.workspace.pending_epoch.get_untracked() != gesture.epoch
                    || self.workspace.editor_read_revision.get_untracked() != gesture.read
                    || self.actions.account_generation() != gesture.account
                    || !self.actions.is_current(gesture.project, &gesture.path)
                    || self.workspace.editor_projection_revision.get_untracked()
                        != gesture.projection_revision
            })
        })
    }
    fn select_at(self, input: &web_sys::HtmlTextAreaElement, x: f64, y: f64) {
        if !self.ready.get_untracked() {
            return;
        }
        let result = self.gesture.with_value(|gesture| {
            let gesture = gesture.as_ref()?;
            let offset = self.offset(input, x, y, true)?;
            Some(self.actions.drag_pointer_selection(
                gesture.project,
                &gesture.path,
                &gesture.source,
                &gesture.pointer,
                offset,
            ))
        });
        match result {
            Some(Ok(Some(_))) => self.render(input),
            Some(
                Err(openwebide_core::editor::SelectionError::Edit(
                    openwebide_core::editor::EditError::StaleContext,
                ))
                | Ok(None),
            ) => self.gesture.set_value(None),
            Some(Err(error)) => {
                self.gesture.set_value(None);
                self.motion.error.set(Some(error.to_string()));
            }
            None => {}
        }
    }
    fn schedule(self, generation: u64) {
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if self.motion.error.is_disposed() || self.generation.get_value() != generation {
                return;
            }
            untrack(|| self.scroll_frame());
            if self.gesture.with_value(Option::is_some) {
                self.schedule(generation);
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn scroll_frame(self) {}
    #[cfg(target_arch = "wasm32")]
    fn scroll_frame(self) {
        use openwebide_core::editor::selection_scroll_delta;
        let Some(input) = self
            .input
            .get_untracked()
            .filter(|input| current_editor_target(self.actions, input))
        else {
            self.gesture.set_value(None);
            return;
        };
        if self.stale() {
            self.gesture.set_value(None);
            return;
        }
        let now = js_sys::Date::now();
        let Some((point, elapsed)) = self.gesture.with_value(|gesture| {
            gesture
                .as_ref()
                .map(|gesture| (gesture.point, now - gesture.time))
        }) else {
            return;
        };
        self.gesture.update_value(|gesture| {
            if let Some(gesture) = gesture {
                gesture.time = now;
            }
        });
        let bounds = crate::viewport::editor_scroll(&input).get_bounding_client_rect();
        if bounds.width() <= 2.0 || bounds.height() <= 2.0 {
            return;
        }
        let scroll = crate::viewport::editor_scroll(&input);
        let dx = selection_scroll_delta(point.0, bounds.left(), bounds.right(), elapsed);
        let dy = selection_scroll_delta(point.1, bounds.top(), bounds.bottom(), elapsed);
        if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
            return;
        }
        crate::viewport::set_editor_scroll_left(&input, scroll.scroll_left() + dx);
        crate::viewport::set_editor_scroll_top(&input, scroll.scroll_top() + dy);
        // Hit only visible paint. Scroll events prepare the next window; missing
        // or superseded paint is retried on the next frame rather than guessed.
        self.select_at(
            &input,
            point.0.clamp(bounds.left() + 1.0, bounds.right() - 1.0),
            point.1.clamp(bounds.top() + 1.0, bounds.bottom() - 1.0),
        );
    }
}
