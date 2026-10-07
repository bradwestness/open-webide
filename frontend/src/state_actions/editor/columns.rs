//! Rectangular selection ownership; source and policy stay behind the facade.
use super::{EditorActions, EditorPresentationScope};
use leptos::prelude::*;
use openwebide_core::editor::{ColumnSelection, Selection, SelectionError};

#[derive(Clone, Debug)]
pub struct EditorColumnSelection {
    scope: EditorPresentationScope,
    source: u64,
    gesture: ColumnSelection,
}
impl EditorActions {
    pub fn toggle_current_cursor(
        self,
        project: i64,
        path: &str,
        offset: usize,
    ) -> Result<Option<Vec<Selection>>, SelectionError> {
        self.workspace
            .content
            .with_untracked(|source| self.toggle_cursor(project, path, source, offset))
    }
    pub fn begin_current_column_selection(
        self,
        project: i64,
        path: &str,
        anchor: usize,
        head: usize,
    ) -> Result<Option<(EditorColumnSelection, Vec<Selection>)>, SelectionError> {
        if !self.is_current(project, path) {
            return Ok(None);
        }
        let Some(scope) = untrack(|| self.presentation_scope()) else {
            return Ok(None);
        };
        let source_revision = self.workspace.editor_source_revision.get_untracked();
        let indentation = self.rules_untracked().indentation;
        let mut gesture = None;
        let selections = self.workspace.content.with_untracked(|source| {
            self.operate_selections(project, path, source, |document| {
                gesture = Some(document.begin_column_selection(anchor, head, indentation)?);
                Ok(true)
            })
        })?;
        Ok(selections.zip(gesture).map(|(selections, gesture)| {
            (
                EditorColumnSelection {
                    scope,
                    source: source_revision,
                    gesture,
                },
                selections,
            )
        }))
    }
    pub fn drag_current_column_selection(
        self,
        selection: &EditorColumnSelection,
        head: usize,
    ) -> Result<Option<Vec<Selection>>, SelectionError> {
        if untrack(|| self.presentation_scope()).as_ref() != Some(&selection.scope)
            || self.workspace.editor_source_revision.get_untracked() != selection.source
            || self.rules_untracked().indentation != selection.gesture.indentation()
            || !self.workspace.editor_documents.with_untracked(|documents| {
                documents
                    .get(&(selection.scope.project, selection.scope.path.clone()))
                    .is_some_and(|document| selection.gesture.matches(document))
            })
        {
            return Ok(None);
        }
        self.workspace.content.with_untracked(|source| {
            self.operate_selections(
                selection.scope.project,
                &selection.scope.path,
                source,
                |document| document.drag_column_selection(&selection.gesture, head),
            )
        })
    }
}
