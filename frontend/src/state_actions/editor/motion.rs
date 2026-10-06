//! Shared deferred-motion policy; frame and DOM measurement adapters call here.
use super::EditorActions;
use crate::state::editor_motion::PendingEditorMotion;
use leptos::prelude::*;
use openwebide_core::editor::{
    MotionQueue, MotionRequest, Selection, SelectionError, SelectionMotion, VisualLayout,
};

impl EditorActions {
    fn motion_current(self, pending: &PendingEditorMotion) -> bool {
        self.key().as_ref() == Some(&pending.key)
            && self.workspace.pending_epoch.get_untracked() == pending.epoch
            && self.workspace.editor_read_revision.get_untracked() == pending.read_revision
            && self.auth.map_or(0, |auth| auth.generation.get_untracked())
                == pending.account_generation
            && untrack(|| self.preferences()) == pending.preferences
            && self.rules_untracked().indentation == pending.indentation
            && self.workspace.editor_documents.with_untracked(|documents| {
                documents.get(&pending.key).is_some_and(|document| {
                    pending.queue.matches(document)
                        && document.text() == self.workspace.content.get_untracked()
                })
            })
    }

    pub fn cancel_queued_motion(self, ticket: Option<u64>) {
        self.workspace.editor_motion.try_update(|pending| {
            if ticket.is_none()
                || pending
                    .as_ref()
                    .is_some_and(|pending| Some(pending.ticket) == ticket)
            {
                *pending = None;
            }
        });
    }

    pub fn queued_motion_ticket(self) -> Option<u64> {
        self.workspace
            .editor_motion
            .with_untracked(|pending| pending.as_ref().map(|pending| pending.ticket))
    }

    /// Returns a ticket and whether the browser should start a new frame callback.
    pub fn queue_motion(
        self,
        project: i64,
        path: &str,
        source: &str,
        motion: SelectionMotion,
        extend: bool,
    ) -> Result<Option<(u64, bool)>, SelectionError> {
        if !self.is_current(project, path) || self.source() != source {
            return Ok(None);
        }
        let existing = self
            .workspace
            .editor_motion
            .try_update(Option::take)
            .flatten();
        let preferences = untrack(|| self.preferences());
        let indentation = self.rules_untracked().indentation;
        let (mut pending, scheduled) = if let Some(pending) =
            existing.filter(|pending| self.motion_current(pending))
        {
            (pending, false)
        } else {
            let key = (project, path.to_owned());
            let queue = self
                .workspace
                .editor_documents
                .try_update(|documents| MotionQueue::new(self.document(documents, key.clone())))
                .unwrap_or(Err(openwebide_core::editor::EditError::StaleContext.into()))?;
            self.workspace
                .editor_motion_ticket
                .update(|ticket| *ticket = ticket.wrapping_add(1));
            (
                PendingEditorMotion {
                    ticket: self.workspace.editor_motion_ticket.get_untracked(),
                    key,
                    epoch: self.workspace.pending_epoch.get_untracked(),
                    read_revision: self.workspace.editor_read_revision.get_untracked(),
                    account_generation: self.auth.map_or(0, |auth| auth.generation.get_untracked()),
                    preferences,
                    indentation,
                    queue,
                },
                true,
            )
        };
        let result = pending.queue.push(MotionRequest {
            motion,
            extend,
            wrapped: preferences.word_wrap,
        });
        let ticket = pending.ticket;
        self.workspace.editor_motion.set(Some(pending));
        result?;
        Ok(Some((ticket, scheduled)))
    }

    pub fn next_queued_motion(self, ticket: u64) -> Option<MotionRequest> {
        let result = self.workspace.editor_motion.with_untracked(|pending| {
            pending
                .as_ref()
                .filter(|pending| pending.ticket == ticket && self.motion_current(pending))
                .and_then(|pending| pending.queue.next_request())
        });
        if result.is_none() {
            self.cancel_queued_motion(Some(ticket));
        }
        result
    }

    pub fn wait_for_motion_layout(self, ticket: u64) -> Result<(), SelectionError> {
        self.workspace
            .editor_motion
            .try_update(|pending| {
                let Some(pending) = pending.as_mut().filter(|pending| pending.ticket == ticket)
                else {
                    return Ok(());
                };
                pending.queue.wait_for_layout()
            })
            .unwrap_or(Ok(()))
    }

    pub fn apply_queued_motion(
        self,
        ticket: u64,
        layout: Option<&VisualLayout>,
    ) -> Result<Option<Vec<Selection>>, SelectionError> {
        let Some(mut pending) = self
            .workspace
            .editor_motion
            .try_update(Option::take)
            .flatten()
        else {
            return Ok(None);
        };
        if pending.ticket != ticket {
            self.workspace.editor_motion.set(Some(pending));
            return Ok(None);
        }
        if !self.motion_current(&pending) {
            return Ok(None);
        }
        let indentation = self.rules_untracked().indentation;
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let Some(document) = documents.get_mut(&pending.key) else {
                    return Ok(None);
                };
                pending.queue.apply_next(document, indentation, layout)
            })
            .unwrap_or(Ok(None));
        if result.is_ok() && pending.queue.next_request().is_some() {
            self.workspace.editor_motion.set(Some(pending));
        }
        if matches!(result, Ok(Some(_))) {
            self.typing.set(None);
            self.workspace
                .editor_fold_revision
                .update(|value| *value = value.wrapping_add(1));
        }
        result
    }
}
