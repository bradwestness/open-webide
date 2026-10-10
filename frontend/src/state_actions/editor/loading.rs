//! Complete-only document loading above the shared workspace adapters.
use super::{EditorActions, EditorText};
use leptos::prelude::*;
use openwebide_core::editor::{DocumentPreparation, EditorAdmission};

impl EditorActions {
    /// Prepare an incoming file while retaining the current buffer. The caller's
    /// guard covers its workspace root/transport; this facade also protects editor
    /// ownership and drafts across every browser yield and final publication.
    pub(crate) async fn publish_read(
        self,
        content: String,
        baseline: &EditorText,
        current: impl Fn() -> bool,
    ) -> bool {
        let scope = self.presentation_scope();
        let revision = self.workspace.editor_source_revision.get_untracked();
        let owned = || {
            current()
                && !self.workspace.content.is_disposed()
                && untrack(|| self.presentation_scope()) == scope
                && self.workspace.editor_source_revision.get_untracked() == revision
                && !self.workspace.dirty.get_untracked()
                && self
                    .workspace
                    .content
                    .with_untracked(|text| text == baseline)
        };
        if scope.is_none() || !owned() {
            return false;
        }
        let text = EditorText::from(content);
        let source = text.shared();
        let mut admission = EditorAdmission::new(&source);
        while !admission.advance(8) {
            crate::util::yield_task().await;
            if !owned() {
                return false;
            }
        }
        let mut document = None;
        if admission.finish() == Some(None) {
            let mut preparation = DocumentPreparation::new(&source);
            while !preparation.advance(64) {
                crate::util::yield_task().await;
                if !owned() {
                    return false;
                }
            }
            if !owned() {
                return false;
            }
            document = preparation.finish();
        }
        if !owned() {
            return false;
        }
        let scope = scope.unwrap();
        batch(|| {
            self.workspace.editor_documents.update(|documents| {
                let key = (scope.project, scope.path);
                if let Some(mut document) = document {
                    if documents
                        .get(&key)
                        .is_none_or(|existing| !existing.matches_text(&source))
                    {
                        document.enforce_editor_limits();
                        documents.insert(key, document);
                    }
                } else {
                    // Over-limit sources use the existing bounded read-only view.
                    documents.remove(&key);
                }
            });
            if self
                .workspace
                .content
                .with_untracked(|current| current.as_str() != source.as_str())
            {
                self.workspace.content.set(text);
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        future::Future,
        task::{Context, Poll, Waker},
    };

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn incoming_documents_yield_and_preserve_drafts_and_ownership() {
        for phase in ["admission", "index"] {
            let incoming = if phase == "admission" {
                "文😀\r\n".repeat(12_000)
            } else {
                "文😀\r\n".repeat(2000)
            };
            for stale in [
                "complete", "draft", "read", "project", "account", "epoch", "root", "dispose",
            ] {
                let owner = Owner::new();
                let (workspace, auth, actions) = owner.with(|| {
                    let auth = crate::state::auth::AuthState::new();
                    provide_context(auth);
                    let workspace = crate::state::workspace::WorkspaceState::new();
                    workspace.active_project.set(Some(1));
                    workspace.open_file.set(Some("incoming.rs".into()));
                    workspace.content.set("baseline".into());
                    (workspace, auth, EditorActions::new(workspace))
                });
                let baseline = workspace.content.get_untracked();
                let root_current = Cell::new(true);
                let mut pending =
                    Box::pin(
                        actions.publish_read(incoming.clone(), &baseline, || root_current.get()),
                    );
                assert!(matches!(
                    pending
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop())),
                    Poll::Pending
                ));
                assert_eq!(workspace.content.get_untracked(), baseline);
                assert!(
                    workspace
                        .editor_documents
                        .with_untracked(std::collections::HashMap::is_empty)
                );
                match stale {
                    "draft" => {
                        workspace.content.set("my draft".into());
                        workspace.dirty.set(true);
                    }
                    "read" => {
                        workspace.begin_editor_read();
                    }
                    "project" => workspace.active_project.set(Some(2)),
                    "account" => auth.generation.update(|generation| *generation += 1),
                    "epoch" => workspace.pending_epoch.update(|epoch| *epoch += 1),
                    "root" => root_current.set(false),
                    "dispose" => owner.cleanup(),
                    _ => {}
                }
                assert_eq!(pending.await, stale == "complete");
                if stale == "complete" {
                    workspace.editor_documents.with_untracked(|docs| {
                        let document = &docs[&(1, "incoming.rs".into())];
                        assert_eq!(document.text(), incoming);
                        assert!(!document.is_dirty());
                        assert!(std::sync::Arc::ptr_eq(
                            &document.shared_text(),
                            &workspace.content.get_untracked().shared()
                        ));
                        assert_eq!(
                            document.byte_to_textarea(incoming.len()).unwrap(),
                            incoming.encode_utf16().count()
                                - incoming.bytes().filter(|byte| *byte == b'\r').count()
                        );
                    });
                } else if stale != "dispose" {
                    assert_eq!(
                        workspace.content.get_untracked().as_str(),
                        if stale == "draft" {
                            "my draft"
                        } else {
                            "baseline"
                        }
                    );
                    assert!(
                        workspace
                            .editor_documents
                            .with_untracked(std::collections::HashMap::is_empty)
                    );
                }
                owner.cleanup();
            }
        }
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn unchanged_reads_retain_selection_and_undo_history() {
        use openwebide_core::editor::{Document, Edit, Selection};
        let owner = Owner::new();
        let (workspace, actions) = owner.with(|| {
            let workspace = crate::state::workspace::WorkspaceState::new();
            workspace.active_project.set(Some(1));
            workspace.open_file.set(Some("same.txt".into()));
            workspace.content.set("old!".into());
            let mut document = Document::for_editor("old").unwrap();
            document
                .apply(
                    vec![Edit::replace(3..3, "!")],
                    vec![Selection::caret(2)],
                    None,
                )
                .unwrap();
            document.mark_saved();
            workspace.editor_documents.update(|docs| {
                docs.insert((1, "same.txt".into()), document);
            });
            (workspace, EditorActions::new(workspace))
        });
        let baseline = workspace.content.get_untracked();
        assert!(
            actions
                .publish_read("old!".into(), &baseline, || true)
                .await
        );
        workspace.editor_documents.update(|docs| {
            let document = docs.get_mut(&(1, "same.txt".into())).unwrap();
            assert_eq!(document.selections(), &[Selection::caret(2)]);
            assert!(!document.is_dirty());
            assert!(document.undo());
            assert_eq!(document.text(), "old");
        });
        owner.cleanup();
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn oversized_reads_keep_the_read_only_source_without_document_indexes() {
        let owner = Owner::new();
        let (workspace, actions) = owner.with(|| {
            let workspace = crate::state::workspace::WorkspaceState::new();
            workspace.active_project.set(Some(1));
            workspace.open_file.set(Some("large.txt".into()));
            (workspace, EditorActions::new(workspace))
        });
        let incoming = "x".repeat(openwebide_core::editor::MAX_EDITOR_LINE_BYTES + 1);
        assert!(
            actions
                .publish_read(incoming.clone(), &EditorText::default(), || true)
                .await
        );
        assert_eq!(workspace.content.get_untracked().as_str(), incoming);
        assert!(
            workspace
                .editor_documents
                .with_untracked(std::collections::HashMap::is_empty)
        );
        assert_eq!(
            actions.limit(),
            Some(openwebide_core::editor::EditorLimit::LineBytes)
        );
        owner.cleanup();
    }
}
