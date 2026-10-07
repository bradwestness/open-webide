//! Shared publication/coalescing policy above the thin worker transport.
use super::EditorActions;
use crate::state::workspace::{EditorFallbackPaint, EditorSyntaxScope, PreparedEditorSyntax};
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
    pub(super) fn syntax_scope_current(self, scope: &EditorSyntaxScope) -> bool {
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

    /// Pending worker results do not require a second full-file lexical pass.
    fn worker_syntax_pending(self) -> bool {
        self.workspace.editor_worker_active.get_untracked()
            && self.key().is_some()
            && self
                .workspace
                .editor_preparation
                .with_untracked(|prepared| {
                    prepared
                        .as_ref()
                        .is_none_or(|prepared| !self.syntax_scope_current(&prepared.scope))
                })
    }

    fn fallback_paint(
        self,
    ) -> Option<(bool, std::sync::Arc<openwebide_core::highlight::TokenRows>)> {
        self.workspace
            .editor_fallback_paint
            .with_untracked(|paint| {
                paint
                    .as_ref()
                    .filter(|paint| self.syntax_scope_current(&paint.scope))
                    .map(|paint| (paint.prepared_source, paint.tokens.clone()))
            })
    }
    pub fn syntax_is_pending(self) -> bool {
        if self.worker_syntax_pending() {
            return true;
        }
        self.workspace.editor_fallback_active.get_untracked()
            && self.fallback_paint().is_none()
            && (!self.workspace.editor_worker_active.get_untracked()
                || self.syntax_highlights().is_none())
    }

    /// Keep full-row probes behind cooperative fallback paint. A pending external
    /// worker may still supply borrowed neutral rows, preserving its visible-source
    /// contract; terminal/on-thread fallback must not measure and discard that table.
    pub fn full_row_paint_ready(self) -> bool {
        !self.workspace.editor_fallback_active.get_untracked()
            || self.worker_syntax_pending()
            || !self.syntax_is_pending()
    }

    /// Empty pending tokens borrow row bodies from the immutable projection.
    /// Terminal analysis fallback retains the existing contextual lexer.
    pub fn syntax_paint(self) -> (bool, std::sync::Arc<openwebide_core::highlight::TokenRows>) {
        if !self.workspace.editor_worker_active.get_untracked()
            && self.workspace.editor_fallback_active.get_untracked()
        {
            return self
                .fallback_paint()
                .unwrap_or_else(|| (false, std::sync::Arc::new(Vec::new())));
        }
        if let Some(tokens) = self.syntax_highlights() {
            (true, tokens)
        } else if let Some(paint) = self.fallback_paint() {
            paint
        } else if self.syntax_is_pending() {
            (false, std::sync::Arc::new(Vec::new()))
        } else {
            let language = self
                .key()
                .map_or(openwebide_core::highlight::Language::Plain, |key| {
                    openwebide_core::highlight::language_from_path(&key.1)
                });
            (
                false,
                std::sync::Arc::new(self.workspace.content.with_untracked(|source| {
                    openwebide_core::highlight::share_token_rows(
                        openwebide_core::highlight::highlight_lines(
                            &source.replace("\r\n", "\n"),
                            language,
                        ),
                    )
                })),
            )
        }
    }

    pub fn install_syntax_worker(self) {
        self.install_fallback_paint();
        if !crate::editor_worker::enabled() {
            return;
        }
        let Ok(client) = crate::editor_worker::WorkerClient::new() else {
            return;
        };
        self.install_syntax_transport(Rc::new(client));
    }

    /// Shared cooperative fallback policy above browser task/frame primitives.
    fn install_fallback_paint(self) {
        self.workspace.editor_fallback_active.set(true);
        let ticket = StoredValue::new(0_u64);
        on_cleanup(move || {
            if !self.workspace.editor_fallback_active.is_disposed() {
                self.workspace.editor_fallback_active.set(false);
                self.workspace.editor_fallback_paint.set(None);
            }
        });
        Effect::new(move || {
            self.workspace.content.track();
            self.workspace.open_file.track();
            self.workspace.active_project.track();
            self.workspace.pending_epoch.track();
            self.workspace.editor_read_revision.track();
            self.workspace.editor_worker_active.track();
            self.workspace.editor_preparation_revision.track();
            if let Some(auth) = self.auth {
                auth.generation.track();
            }
            self.rules();
            ticket.update_value(|value| *value = value.wrapping_add(1));
            let next = ticket.get_value();
            if self.worker_syntax_pending() || self.fallback_paint().is_some() {
                return;
            }
            let scope = self
                .workspace
                .editor_preparation
                .with_untracked(|prepared| {
                    prepared
                        .as_ref()
                        .filter(|prepared| self.syntax_scope_current(&prepared.scope))
                        .map(|prepared| prepared.scope.clone())
                })
                .or_else(|| self.syntax_scope());
            let Some(scope) = scope else {
                return;
            };
            if let Some(tokens) = self.syntax_highlights() {
                if !self.workspace.editor_worker_active.get_untracked() {
                    self.workspace
                        .editor_fallback_paint
                        .set(Some(EditorFallbackPaint {
                            scope,
                            prepared_source: true,
                            tokens,
                            lexical: None,
                        }));
                }
                return;
            }
            let mut lexical = openwebide_core::highlight::LexicalPreparation::for_textarea(
                scope.source.clone(),
                openwebide_core::highlight::language_from_path(&scope.key.1),
            );
            if let Some(previous) = self
                .workspace
                .editor_fallback_paint
                .with_untracked(|paint| {
                    paint
                        .as_ref()
                        .filter(|paint| {
                            paint.scope.key == scope.key
                                && paint.scope.account_generation == scope.account_generation
                                && paint.scope.read_revision == scope.read_revision
                                && paint.scope.tab_width == scope.tab_width
                        })
                        .and_then(|paint| paint.lexical.clone())
                })
            {
                lexical = lexical.reuse(previous);
            }
            // Resolve small files in one bounded batch without a transient pending
            // frame. Larger jobs retain their context and yield before continuing.
            lexical.advance(
                openwebide_core::highlight::LEXICAL_BATCH_ROWS,
                openwebide_core::highlight::LEXICAL_BATCH_BYTES,
            );
            if lexical.is_complete() {
                let lexical =
                    std::sync::Arc::new(lexical.finish_snapshot().expect("completed lexical job"));
                self.workspace
                    .editor_fallback_paint
                    .set(Some(EditorFallbackPaint {
                        scope,
                        prepared_source: false,
                        tokens: lexical.tokens().clone(),
                        lexical: Some(lexical),
                    }));
                return;
            }
            leptos::task::spawn_local(async move {
                // Let rapid source/read/account changes coalesce before more token allocation.
                crate::util::yield_task().await;
                let current = || {
                    !ticket.is_disposed()
                        && ticket.get_value() == next
                        && self.syntax_scope_current(&scope)
                        && !self.worker_syntax_pending()
                };
                if !current() {
                    return;
                }
                let mut batches = 1_usize;
                while !lexical.is_complete() {
                    if !current() {
                        return;
                    }
                    lexical.advance(
                        openwebide_core::highlight::LEXICAL_BATCH_ROWS,
                        openwebide_core::highlight::LEXICAL_BATCH_BYTES,
                    );
                    batches += 1;
                    if !lexical.is_complete() {
                        if batches
                            .is_multiple_of(openwebide_core::highlight::LEXICAL_BATCHES_PER_FRAME)
                        {
                            crate::util::yield_frame().await;
                        } else {
                            crate::util::yield_task().await;
                        }
                    }
                }
                if !current() {
                    return;
                }
                let lexical =
                    std::sync::Arc::new(lexical.finish_snapshot().expect("completed lexical job"));
                self.workspace
                    .editor_fallback_paint
                    .set(Some(EditorFallbackPaint {
                        scope,
                        prepared_source: false,
                        tokens: lexical.tokens().clone(),
                        lexical: Some(lexical),
                    }));
            });
        });
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
                        // Keep the shared budgeted parser fallback and cooperative
                        // lexical paint after failure; never trust a malformed reply.
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
