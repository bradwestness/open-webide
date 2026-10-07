//! Scoped native text contexts; document transactions and policy stay shared.
use super::{EditorActions, NATIVE_CONTEXT_BYTES};
use leptos::prelude::*;
use openwebide_core::editor::{EditError, FoldProjection, ProjectionError, Selection};

/// Immutable surrounding text and its complete source selection. The full view
/// shares existing allocations and proves provenance; only the input text is sliced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorNativeContext {
    key: (i64, String),
    epoch: u64,
    read: u64,
    account: u64,
    source: u64,
    projection_revision: u64,
    revision: u64,
    original: FoldProjection,
    projection: FoldProjection,
    selections: Vec<Selection>,
}

/// Native ownership can continue when the committed source still matches the
/// browser's value. Other replicas or a full context request reconciliation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorNativeCommit {
    pub context: EditorNativeContext,
    pub retain_native_value: bool,
}

const CONTEXT_HEADROOM_BYTES: usize = 4 * 1024;

pub(super) fn same_projection(first: &FoldProjection, second: &FoldProjection) -> bool {
    std::ptr::eq(first.text().as_ptr(), second.text().as_ptr())
        && first.text().len() == second.text().len()
}

impl EditorNativeContext {
    pub fn projection(&self) -> &FoldProjection {
        &self.projection
    }

    /// Local UTF-16 offsets for the browser; source selections remain unclipped.
    pub fn native_selection(&self) -> Result<Selection, ProjectionError> {
        let visible = self.projection.input_selection(self.selections[0])?;
        let native = |offset| {
            self.projection
                .byte_to_textarea(offset)
                .map_err(|_| ProjectionError::InvalidOffset)
        };
        Ok(Selection {
            anchor: native(visible.anchor)?,
            head: native(visible.head)?,
        })
    }

    /// Programmatic selection echoes must not truncate a large source selection.
    pub fn source_selection(&self, native: Selection) -> Result<Selection, ProjectionError> {
        if native == self.native_selection()? {
            return Ok(self.selections[0]);
        }
        self.projection.source_native_selection(native)
    }

    fn current(&self, actions: EditorActions) -> bool {
        actions.key().as_ref() == Some(&self.key)
            && actions.workspace.pending_epoch.get_untracked() == self.epoch
            && actions.workspace.editor_read_revision.get_untracked() == self.read
            && actions.account_generation() == self.account
            && actions.workspace.editor_source_revision.get_untracked() == self.source
            && actions.workspace.editor_projection_revision.get_untracked()
                == self.projection_revision
            && actions.workspace.content.with_untracked(|source| {
                actions
                    .workspace
                    .editor_documents
                    .with_untracked(|documents| {
                        documents.get(&self.key).is_some_and(|document| {
                            document.text() == source
                                && document.revision() == self.revision
                                && document.selections() == self.selections
                                && same_projection(&document.projection(), &self.original)
                        })
                    })
            })
    }
}

impl EditorActions {
    pub fn native_context(self) -> Option<EditorNativeContext> {
        let key = self.key()?;
        self.workspace.content.with_untracked(|source| {
            self.workspace.editor_documents.with_untracked(|documents| {
                let document = documents
                    .get(&key)
                    .filter(|document| document.text() == source)?;
                let original = document.projection();
                let projection = document
                    .input_context(NATIVE_CONTEXT_BYTES - CONTEXT_HEADROOM_BYTES)
                    .ok()?;
                Some(EditorNativeContext {
                    key,
                    epoch: self.workspace.pending_epoch.get_untracked(),
                    read: self.workspace.editor_read_revision.get_untracked(),
                    account: self.account_generation(),
                    source: self.workspace.editor_source_revision.get_untracked(),
                    projection_revision: self.workspace.editor_projection_revision.get_untracked(),
                    revision: document.revision(),
                    original,
                    projection,
                    selections: document.selections().to_vec(),
                })
            })
        })
    }

    /// Replay a bounded browser value using the same transaction/history path as
    /// the full projection. A stale context cannot edit another document/account.
    pub fn native_context_input(
        self,
        context: &EditorNativeContext,
        value: &str,
        selection: Selection,
        input_type: &str,
        timestamp: f64,
    ) -> Result<EditorNativeCommit, EditError> {
        if !context.current(self) {
            return Err(EditError::StaleContext);
        }
        self.replay_projected_input(
            &context.projection,
            &context.original,
            value,
            selection,
            input_type,
            timestamp,
        )?;
        let mut context = self.native_context().ok_or(EditError::StaleContext)?;
        let retained = (|| {
            if value.len() > NATIVE_CONTEXT_BYTES && !self.is_composing() {
                return None;
            }
            let native_head = openwebide_core::editor::byte_to_utf16(value, selection.head).ok()?;
            let visible_head = context
                .original
                .visible_selection(context.selections[0])
                .ok()?
                .head;
            let global_head = context.original.byte_to_textarea(visible_head).ok()?;
            let start = global_head.checked_sub(native_head)?;
            let end = start.checked_add(value.encode_utf16().count())?;
            let projection = context
                .original
                .window(
                    context.original.textarea_to_byte(start)
                        ..context.original.textarea_to_byte(end),
                )
                .ok()?;
            (projection.textarea_text() == value).then_some(projection)
        })();
        let retain_native_value = retained.is_some();
        if let Some(projection) = retained {
            context.projection = projection;
        }
        Ok(EditorNativeCommit {
            context,
            retain_native_value,
        })
    }
}
