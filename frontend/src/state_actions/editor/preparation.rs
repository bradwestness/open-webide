//! Shared publication/coalescing policy above the thin worker transport.
use super::EditorActions;
use crate::state::workspace::{EditorSyntaxScope, PreparedEditorSyntax};
use leptos::prelude::*;
use openwebide_core::editor::{SYNTAX_PROTOCOL_VERSION, SyntaxReply, SyntaxRequest};
use std::{cell::Cell, rc::Rc};

impl EditorActions {
    pub(super) fn syntax_scope(self) -> Option<EditorSyntaxScope> {
        Some(EditorSyntaxScope {
            key: self.key()?,
            source: self.workspace.content.get_untracked().into(),
            epoch: self.workspace.pending_epoch.get_untracked(),
            read_revision: self.workspace.editor_read_revision.get_untracked(),
            account_generation: self.auth.map_or(0, |auth| auth.generation.get_untracked()),
            tab_width: self.rules_untracked().indentation.tab_width(),
        })
    }
    fn syntax_scope_current(self, scope: &EditorSyntaxScope) -> bool {
        !self.workspace.content.is_disposed()
            && self.key().as_ref() == Some(&scope.key)
            && self.workspace.pending_epoch.get_untracked() == scope.epoch
            && self.workspace.editor_read_revision.get_untracked() == scope.read_revision
            && self.auth.map_or(0, |auth| auth.generation.get_untracked())
                == scope.account_generation
            && self.rules_untracked().indentation.tab_width() == scope.tab_width
            && self
                .workspace
                .content
                .with_untracked(|source| source.as_str() == scope.source.as_ref())
    }

    pub fn install_syntax_worker(self) {
        if !crate::editor_worker::enabled() {
            return;
        }
        let Ok(client) = crate::editor_worker::WorkerClient::new() else {
            return;
        };
        self.install_syntax_transport(Rc::new(client));
    }

    /// Transport boundary; browser contracts inject deferred replies into the same policy.
    pub fn install_syntax_transport(self, client: Rc<dyn crate::editor_worker::SyntaxTransport>) {
        self.workspace.editor_worker_active.set(true);
        let pending = StoredValue::new(None::<(u32, EditorSyntaxScope)>);
        let ticket = StoredValue::new(0_u32);
        let running = Rc::new(Cell::new(false));
        let cleanup = send_wrapper::SendWrapper::new(client.clone());
        on_cleanup(move || {
            cleanup.stop();
            if !self.workspace.editor_worker_active.is_disposed() {
                self.workspace.editor_worker_active.set(false);
                self.workspace.editor_preparation.set(None);
            }
        });
        Effect::new(move || {
            self.workspace.content.track();
            self.workspace.open_file.track();
            self.workspace.active_project.track();
            self.workspace.pending_epoch.track();
            self.workspace.editor_read_revision.track();
            self.workspace.editor_worker_active.track();
            if let Some(auth) = self.auth {
                auth.generation.track();
            }
            self.rules();
            if !self.workspace.editor_worker_active.get_untracked() {
                return;
            }
            ticket.update_value(|value| *value = value.wrapping_add(1));
            let next = ticket.get_value();
            pending.set_value(self.syntax_scope().map(|scope| (next, scope)));
            if running.replace(true) {
                return;
            }
            let client = client.clone();
            let running = running.clone();
            leptos::task::spawn_local(async move {
                while !pending.is_disposed() {
                    let Some((request_ticket, scope)) =
                        pending.try_update_value(Option::take).flatten()
                    else {
                        break;
                    };
                    if openwebide_core::editor::preparation_exceeds_limits(&scope.source) {
                        if ticket.get_value() == request_ticket && self.syntax_scope_current(&scope)
                        {
                            self.workspace
                                .editor_preparation
                                .set(Some(PreparedEditorSyntax {
                                    scope,
                                    status: openwebide_core::editor::SyntaxStatus::TooLarge,
                                    analysis: None,
                                }));
                            self.workspace
                                .editor_preparation_revision
                                .update(|value| *value = value.wrapping_add(1));
                        }
                        continue;
                    }
                    let request = SyntaxRequest {
                        version: SYNTAX_PROTOCOL_VERSION,
                        ticket: request_ticket,
                        document: format!("{}:{}", scope.key.0, scope.key.1),
                        language: openwebide_core::highlight::language_from_path(&scope.key.1),
                        source: scope.source.to_string(),
                        tab_width: scope.tab_width,
                    };
                    let message = serde_json::to_string(&request)
                        .expect("syntax request contains only serializable primitives");
                    let reply = client.request(request_ticket, message).await;
                    if pending.is_disposed() {
                        break;
                    }
                    if ticket.get_value() != request_ticket || !self.syntax_scope_current(&scope) {
                        continue;
                    }
                    let result = reply.ok().and_then(|message| {
                        SyntaxReply::receive(&message, request_ticket, &scope.source)
                    });
                    if let Some((status, analysis)) = result {
                        self.workspace
                            .editor_preparation
                            .set(Some(PreparedEditorSyntax {
                                scope,
                                status,
                                analysis,
                            }));
                    } else {
                        // Same preparation engine runs through the synchronous adapter after a
                        // worker failure. The editor remains usable and never trusts a bad reply.
                        self.workspace.editor_worker_active.set(false);
                        self.workspace.editor_preparation.set(None);
                        client.stop();
                    }
                    self.workspace
                        .editor_preparation_revision
                        .update(|value| *value = value.wrapping_add(1));
                }
                running.set(false);
            });
        });
    }
}
