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
}

#[derive(Clone, Copy)]
pub(super) struct PointerAdapter {
    actions: EditorActions,
    workspace: WorkspaceState,
    input: NodeRef<leptos::html::Textarea>,
    ready: RwSignal<bool>,
    motion: MotionAdapter,
    gesture: StoredValue<Option<Gesture>>,
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
        event: &web_sys::MouseEvent,
    ) -> Option<usize> {
        let offset =
            crate::viewport::editor_caret_from_point(input, event.client_x(), event.client_y())?;
        let projection = self.actions.projection()?;
        projection
            .source_offset(projection.textarea_to_byte(offset as usize))
            .ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn offset(
        self,
        _input: &web_sys::HtmlTextAreaElement,
        _event: &web_sys::MouseEvent,
    ) -> Option<usize> {
        None
    }

    pub fn start(self, event: &web_sys::MouseEvent) {
        self.gesture.set_value(None);
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
        let Some(offset) = self.offset(&input, event) else {
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
                }));
                self.render(&input);
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
        let stale = self.gesture.with_value(|gesture| {
            gesture.as_ref().is_some_and(|gesture| {
                self.workspace.pending_epoch.get_untracked() != gesture.epoch
                    || self.workspace.editor_read_revision.get_untracked() != gesture.read
                    || self.actions.account_generation() != gesture.account
                    || !self.actions.is_current(gesture.project, &gesture.path)
                    || self.actions.source() != gesture.source
            })
        });
        if stale {
            self.gesture.set_value(None);
            return;
        }
        let result = self.gesture.with_value(|gesture| {
            let gesture = gesture.as_ref()?;
            let offset = self.offset(&input, event)?;
            event.prevent_default();
            Some(self.actions.drag_pointer_selection(
                gesture.project,
                &gesture.path,
                &gesture.source,
                &gesture.pointer,
                offset,
            ))
        });
        match result {
            Some(Ok(Some(_))) => self.render(&input),
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
}
