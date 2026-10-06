//! Shared editor facade. DOM adapters provide text/selection/events; editing policy
//! lives in the Rust document engine without filesystem-mode branches.
use leptos::prelude::*;
use openwebide_core::editor::{Document, EditError, Indentation, Selection};

use crate::state::workspace::WorkspaceState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorCommand {
    Tab,
    Outdent,
    Newline,
    Undo,
    Redo,
    ConvertIndentation,
    TypeCharacter(char),
    DeletePair,
    Line(openwebide_core::editor::LineCommand),
    DuplicateSelection,
    LineComment,
    BlockComment,
    Reindent,
}

mod motion;
mod preparation;
mod rows;
pub use rows::{EditorFragmentCache, EditorFragmentWindow, EditorRowSourceSlice};

/// Facts established by a guarded native insertion transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTextCommit {
    pub selection: Selection,
    pub retain_native_value: bool,
}

type TypingState = Option<((i64, String), String, f64)>;

#[derive(Clone, Copy)]
pub struct EditorActions {
    workspace: WorkspaceState,
    auth: Option<crate::state::auth::AuthState>,
    preferences: Option<crate::state::settings::SettingsState>,
    capacity: Memo<Option<openwebide_core::editor::EditorLimit>>,
    group: RwSignal<u64>,
    typing: RwSignal<TypingState>,
}

impl EditorActions {
    pub fn new(workspace: WorkspaceState) -> Self {
        Self {
            workspace,
            auth: use_context::<crate::state::auth::AuthState>(),
            preferences: use_context::<crate::state::settings::SettingsState>(),
            capacity: Memo::new(move |_| {
                workspace
                    .content
                    .with(|source| openwebide_core::editor::editor_limit(source))
            }),
            group: workspace.editor_group,
            typing: RwSignal::new(None),
        }
    }

    pub fn rules(self) -> openwebide_core::editor::EditorRules {
        let key = self
            .workspace
            .active_project
            .get()
            .zip(self.workspace.open_file.get());
        let defaults = self
            .preferences
            .map(|settings| settings.editor_preferences.get())
            .unwrap_or_default();
        let mut rules = self
            .workspace
            .editor_rules
            .with(|rules| key.as_ref().and_then(|key| rules.get(key).cloned()))
            .unwrap_or_else(|| {
                openwebide_core::editor::resolve_rules(
                    "",
                    &self.workspace.content.get(),
                    defaults,
                    &[],
                )
                .0
            });
        if let Some(indentation) = self
            .workspace
            .editor_indentation
            .with(|values| key.as_ref().and_then(|key| values.get(key).copied()))
        {
            rules.indentation = indentation;
        }
        rules
    }

    pub fn preferences(self) -> openwebide_core::editor::EditorPreferences {
        self.preferences
            .map(|settings| settings.editor_preferences.get())
            .unwrap_or_default()
    }

    pub fn set_indentation(self, mut indentation: Indentation) {
        indentation.width = indentation.width();
        indentation.tab_width = indentation.tab_width();
        if let Some(key) = self.key() {
            self.workspace.editor_indentation.update(|values| {
                values.insert(key, indentation);
            });
        }
    }

    pub fn rules_untracked(self) -> openwebide_core::editor::EditorRules {
        untrack(|| self.rules())
    }

    pub fn prepare_save(
        self,
        rules: &openwebide_core::editor::EditorRules,
    ) -> Result<Option<String>, EditError> {
        let Some(key) = self.key() else {
            return Ok(None);
        };
        self.typing.set(None);
        self.group.update(|group| *group = group.wrapping_add(1));
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                document.prepare_save(rules)?;
                Ok(Some(document.text().to_string()))
            })
            .unwrap_or(Ok(None));
        if let Ok(Some(text)) = &result {
            self.workspace.content.set(text.clone());
            self.publish_dirty(key);
        }
        result
    }

    pub fn begin_composition(self) {
        self.cancel_queued_motion(None);
        if self.is_composing() {
            return;
        }
        self.cancel_composition();
        let Some(key) = self.key() else {
            return;
        };
        self.group.update(|group| *group = group.wrapping_add(1));
        self.typing.set(None);
        let group = self.group.get_untracked();
        self.workspace.editor_documents.update(|documents| {
            self.document(documents, key.clone())
                .begin_composition(Some(group));
        });
        self.workspace
            .editor_composition
            .set(Some(crate::state::workspace::EditorComposition {
                key,
                epoch: self.workspace.pending_epoch.get_untracked(),
            }));
    }

    pub fn is_composing(self) -> bool {
        self.workspace.editor_composition.with_untracked(|owner| {
            owner.as_ref().is_some_and(|owner| {
                self.key().as_ref() == Some(&owner.key)
                    && self.workspace.pending_epoch.get_untracked() == owner.epoch
            })
        })
    }

    pub fn cancel_composition(self) {
        let Some(owner) = self.workspace.editor_composition.get_untracked() else {
            return;
        };
        self.workspace.editor_composition.set(None);
        self.typing.set(None);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = documents.get_mut(&owner.key)?;
                let preview = document.text().to_string();
                document
                    .cancel_composition()
                    .then(|| (preview, document.text().to_string(), document.is_dirty()))
            })
            .flatten();
        if let Some((preview, source, dirty)) = result
            && self.workspace.pending_epoch.get_untracked() == owner.epoch
        {
            self.workspace.snapshots.update(|snapshots| {
                if let Some(snapshot) = snapshots.get_mut(&owner.key.0)
                    && snapshot.open_file.as_deref() == Some(&owner.key.1)
                    && snapshot.content == preview
                {
                    snapshot.content.clone_from(&source);
                    snapshot.dirty = dirty;
                }
            });
            if self.key().as_ref() == Some(&owner.key) && self.source() == preview {
                self.workspace.content.set(source);
                self.workspace.dirty.set(dirty);
            }
        }
    }

    pub fn end_composition(self) -> Result<Option<(String, Selection)>, EditError> {
        let Some(owner) = self.workspace.editor_composition.get_untracked() else {
            return Ok(None);
        };
        if self.key().as_ref() != Some(&owner.key)
            || self.workspace.pending_epoch.get_untracked() != owner.epoch
        {
            self.cancel_composition();
            return Ok(None);
        }
        self.workspace.editor_composition.set(None);
        self.typing.set(None);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = documents.get_mut(&owner.key)?;
                if document.text() != self.source() {
                    document.cancel_composition();
                    return None;
                }
                let outcome = document.end_composition();
                Some((
                    outcome,
                    document.text().to_string(),
                    document.selections()[0],
                    document.is_dirty(),
                ))
            })
            .flatten();
        if let Some((outcome, source, selection, dirty)) = result {
            self.workspace.content.set(source.clone());
            self.workspace.dirty.set(dirty);
            return outcome.map(|_| Some((source, selection)));
        }
        Ok(None)
    }

    pub fn projection_revision(self) -> u64 {
        self.workspace.editor_projection_revision.get()
    }
    pub fn view_revision(self) -> u64 {
        self.workspace.editor_view_revision.get()
    }
    pub fn layout_epoch(self) -> u64 {
        self.workspace.editor_layout_epoch.get()
    }
    pub fn measured_rows(self) -> Option<crate::state::workspace::EditorRowMeasurements> {
        let revision = self.view_revision();
        self.workspace.editor_rows.with(|rows| {
            rows.as_ref()
                .filter(|rows| rows.revision == revision)
                .cloned()
        })
    }
    pub fn invalidate_measured_rows(self) {
        self.workspace.editor_rows.set(None);
        self.workspace
            .editor_layout_epoch
            .update(|epoch| *epoch = epoch.wrapping_add(1));
    }
    pub fn begin_row_preparation(self, revision: u64, total: usize) -> Option<u64> {
        if self.view_revision() != revision || total > openwebide_core::editor::MAX_EDITOR_LINES {
            return None;
        }
        self.workspace
            .editor_row_ticket
            .update(|ticket| *ticket = ticket.wrapping_add(1));
        let ticket = self.workspace.editor_row_ticket.get_untracked();
        self.workspace.editor_row_preparation.set(Some(
            crate::state::workspace::EditorRowPreparation {
                ticket,
                revision,
                completed: 0,
                total,
            },
        ));
        Some(ticket)
    }
    pub fn row_preparation_current(self, ticket: u64) -> bool {
        self.workspace
            .editor_row_preparation
            .with_untracked(|preparation| {
                preparation.is_some_and(|preparation| {
                    preparation.ticket == ticket && preparation.revision == self.view_revision()
                })
            })
    }
    pub fn report_row_preparation(self, ticket: u64, completed: usize) {
        if !self.row_preparation_current(ticket) {
            return;
        }
        self.workspace.editor_row_preparation.update(|preparation| {
            if let Some(preparation) = preparation.as_mut()
                && completed >= preparation.completed
                && completed <= preparation.total
            {
                preparation.completed = completed;
            }
        });
    }
    pub fn end_row_preparation(self, ticket: u64) {
        self.workspace
            .editor_row_preparation
            .try_update(|preparation| {
                if preparation
                    .as_ref()
                    .is_some_and(|preparation| preparation.ticket == ticket)
                {
                    *preparation = None;
                }
            });
    }
    pub fn publish_measured_rows(
        self,
        revision: u64,
        metrics: String,
        rows: openwebide_core::editor::MeasuredRows,
    ) -> bool {
        if self.view_revision() != revision
            || self
                .projection()
                .is_none_or(|projection| projection.lines().len() != rows.len())
        {
            return false;
        }
        self.workspace
            .editor_rows
            .set(Some(crate::state::workspace::EditorRowMeasurements {
                revision,
                metrics,
                rows,
            }));
        true
    }

    pub fn limit(self) -> Option<openwebide_core::editor::EditorLimit> {
        self.capacity.get()
    }

    fn key(self) -> Option<(i64, String)> {
        if self.capacity.get_untracked().is_some() {
            return None;
        }
        Some((
            self.workspace.active_project.get_untracked()?,
            self.workspace.open_file.get_untracked()?,
        ))
    }

    pub fn is_current(self, project: i64, path: &str) -> bool {
        self.key().is_some_and(|(current_project, current_path)| {
            current_project == project && current_path == path
        })
    }

    fn document(
        self,
        documents: &mut std::collections::HashMap<(i64, String), Document>,
        key: (i64, String),
    ) -> &mut Document {
        let document = documents.entry(key).or_insert_with(|| {
            self.workspace
                .content
                .with_untracked(|text| Document::new(text.clone()))
        });
        self.workspace.content.with_untracked(|text| {
            if document.text() != text {
                *document = Document::new(text.clone());
            }
        });
        document.enforce_editor_limits();
        if !self.workspace.dirty.get_untracked() {
            document.mark_saved();
        }
        document
    }

    pub fn record_selection(self, selection: Selection) -> Result<(), EditError> {
        self.cancel_native_text();
        self.cancel_queued_motion(None);
        let Some(key) = self.key() else {
            return Ok(());
        };
        self.workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key);
                if document.selections() != [selection] {
                    self.typing.set(None);
                }
                document.set_selections(vec![selection])
            })
            .unwrap_or(Ok(()))
    }

    /// Native select/scroll notifications also fire after programmatic restores.
    /// Keep secondary selections when the primary did not move; IME previews own
    /// their selections until the composition commits or cancels.
    pub fn record_native_selection(self, selection: Selection) -> Result<(), EditError> {
        let Some(key) = self.key() else {
            return Ok(());
        };
        if self
            .workspace
            .editor_text_insertion
            .with_untracked(|insertion| {
                insertion
                    .as_ref()
                    .is_some_and(|insertion| insertion.key == key)
            })
        {
            return Ok(());
        }
        self.workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key);
                if document.is_composing() || document.selections().first() == Some(&selection) {
                    return Ok(());
                }
                self.cancel_queued_motion(None);
                self.typing.set(None);
                document.set_selections(vec![selection])
            })
            .unwrap_or(Ok(()))
    }

    /// The browser adapter supplies measurements only for the currently mounted document.
    pub fn record_scroll(self, project: i64, path: &str, top: f64, left: f64) {
        if !self.is_current(project, path) || !top.is_finite() || !left.is_finite() {
            return;
        }
        self.workspace.editor_scroll.update(|positions| {
            positions.insert(
                (project, path.to_string()),
                crate::state::workspace::EditorScroll {
                    top: top.max(0.0),
                    left: left.max(0.0),
                },
            );
        });
    }
    pub fn finish_row_preparation(
        self,
        ticket: u64,
        paint: crate::state::workspace::EditorRowPaint,
        result: Result<Option<openwebide_core::editor::MeasuredRows>, ()>,
    ) -> Option<&'static str> {
        if !self.row_preparation_current(ticket) || !self.row_paint_current(&paint) {
            self.end_row_preparation(ticket);
            return None;
        }
        let message = match result {
            Ok(Some(rows)) => {
                if self.publish_measured_rows(
                    self.view_revision(),
                    paint.metrics.clone(),
                    rows.clone(),
                ) {
                    self.workspace.editor_row_cache.set(Some(
                        crate::state::workspace::EditorRowCache { paint, rows },
                    ));
                }
                None
            }
            Ok(None) => None,
            Err(()) => Some(
                "The highlighted view is unavailable for this layout. You can keep editing the file.",
            ),
        };
        self.end_row_preparation(ticket);
        message
    }

    pub fn scroll(self) -> crate::state::workspace::EditorScroll {
        self.key()
            .and_then(|key| {
                self.workspace
                    .editor_scroll
                    .with_untracked(|positions| positions.get(&key).copied())
            })
            .unwrap_or_default()
    }

    /// Map native UTF-16 positions through the active document's shared index.
    pub fn native_selection(self, source: &str, selection: Selection) -> Selection {
        let indexed = self.key().and_then(|key| {
            self.workspace.editor_documents.with_untracked(|documents| {
                documents
                    .get(&key)
                    .filter(|document| document.text() == source)
                    .map(|document| document.native_selection(selection))
            })
        });
        indexed.unwrap_or_else(|| Selection {
            anchor: openwebide_core::editor::textarea_to_byte(source, selection.anchor),
            head: openwebide_core::editor::textarea_to_byte(source, selection.head),
        })
    }

    pub fn selection(self, text: &str) -> Option<Selection> {
        let key = self.key()?;
        self.workspace.editor_documents.with_untracked(|documents| {
            documents
                .get(&key)
                .filter(|document| document.text() == text)
                .and_then(|document| document.selections().first().copied())
        })
    }

    pub fn selections(self, text: &str) -> Vec<Selection> {
        let Some(key) = self.key() else {
            return Vec::new();
        };
        self.workspace.editor_documents.with_untracked(|documents| {
            documents
                .get(&key)
                .filter(|document| document.text() == text)
                .map(|document| document.selections().to_vec())
                .unwrap_or_default()
        })
    }

    pub fn visual_caret(
        self,
        source: &str,
        index: usize,
        identity: &str,
    ) -> Option<openwebide_core::editor::VisualCaret> {
        let key = self.key()?;
        self.workspace.editor_documents.with_untracked(|documents| {
            documents
                .get(&key)
                .filter(|document| document.text() == source)
                .and_then(|document| document.visual_caret(index, identity))
        })
    }

    /// Source/scope checked selection commands share the document policy in both modes.
    pub fn selection_command(
        self,
        project: i64,
        path: &str,
        source: &str,
        command: openwebide_core::editor::SelectionCommand,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        let rules = self.rules_untracked();
        let language = openwebide_core::highlight::language_from_path(path);
        if !self.is_current(project, path) || self.source() != source {
            return Ok(None);
        }
        let syntax = (command == openwebide_core::editor::SelectionCommand::Expand)
            .then(|| self.syntax_structure(|| true))
            .flatten();
        self.operate_selections(project, path, source, move |document| {
            if let Some(context) = &syntax {
                document.selection_command_with_context(command, rules.indentation, context)
            } else {
                document.selection_command(command, language, rules.indentation)
            }
        })
    }

    pub fn move_selections(
        self,
        project: i64,
        path: &str,
        source: &str,
        motion: openwebide_core::editor::SelectionMotion,
        extend: bool,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        self.move_selections_in(project, path, source, motion, extend, None)
    }

    pub fn move_selections_with_layout(
        self,
        project: i64,
        path: &str,
        source: &str,
        motion: openwebide_core::editor::SelectionMotion,
        extend: bool,
        layout: &openwebide_core::editor::VisualLayout,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        self.move_selections_in(project, path, source, motion, extend, Some(layout))
    }

    fn move_selections_in(
        self,
        project: i64,
        path: &str,
        source: &str,
        motion: openwebide_core::editor::SelectionMotion,
        extend: bool,
        layout: Option<&openwebide_core::editor::VisualLayout>,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        let indentation = self.rules_untracked().indentation;
        self.operate_selections(project, path, source, move |document| {
            if let Some(layout) = layout {
                document.move_selections_with_layout(motion, extend, indentation, layout)
            } else {
                document.move_selections(motion, extend, indentation)
            }
        })
    }

    pub fn toggle_cursor(
        self,
        project: i64,
        path: &str,
        source: &str,
        offset: usize,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        self.operate_selections(project, path, source, move |document| {
            Ok(document.toggle_cursor(offset)?)
        })
    }

    pub fn select_columns(
        self,
        project: i64,
        path: &str,
        source: &str,
        anchor: usize,
        head: usize,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        let indentation = self.rules_untracked().indentation;
        self.operate_selections(project, path, source, move |document| {
            document.select_columns(anchor, head, indentation)
        })
    }

    fn operate_selections(
        self,
        project: i64,
        path: &str,
        source: &str,
        operation: impl FnOnce(&mut Document) -> Result<bool, openwebide_core::editor::SelectionError>,
    ) -> Result<Option<Vec<Selection>>, openwebide_core::editor::SelectionError> {
        if !self.is_current(project, path) || self.source() != source {
            return Ok(None);
        }
        self.cancel_queued_motion(None);
        let Some(key) = self.key() else {
            return Ok(None);
        };
        let mut folds_changed = false;
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key);
                let folds = document.fold_state().clone();
                operation(document)?;
                folds_changed = document.fold_state() != &folds;
                Ok(Some(document.selections().to_vec()))
            })
            .unwrap_or(Ok(None));
        if matches!(result, Ok(Some(_))) {
            self.typing.set(None);
            if folds_changed {
                self.workspace
                    .editor_fold_revision
                    .update(|value| *value = value.wrapping_add(1));
            }
        }
        result
    }

    /// Parser-backed folding provider for the active buffer. DOM/worker adapters
    /// supply a deadline or cancellation primitive; all parsing policy is shared.
    pub fn syntax_folds(
        self,
        should_continue: impl FnMut() -> bool,
    ) -> Option<(
        openwebide_core::editor::SyntaxStatus,
        Vec<openwebide_core::editor::FoldRange>,
    )> {
        if self.capacity.get_untracked().is_some() {
            let key = self
                .workspace
                .active_project
                .get_untracked()
                .zip(self.workspace.open_file.get_untracked())?;
            self.workspace
                .editor_syntax
                .update(|cache| cache.remove(&key));
            return Some((openwebide_core::editor::SyntaxStatus::TooLarge, Vec::new()));
        }
        self.analyze_syntax(should_continue, |document, status| {
            (
                status,
                document.map_or_else(Vec::new, |document| document.folds().to_vec()),
            )
        })
    }

    pub fn syntax_structure(
        self,
        should_continue: impl FnMut() -> bool,
    ) -> Option<std::sync::Arc<openwebide_core::editor::Structure>> {
        self.analyze_syntax(should_continue, |document, _| {
            document.and_then(|document| document.structure().cloned())
        })
        .flatten()
    }

    pub fn preparation_revision(self) {
        self.workspace.editor_preparation_revision.track();
    }

    pub fn syntax_highlights(
        self,
    ) -> Option<std::sync::Arc<Vec<Vec<openwebide_core::highlight::Token>>>> {
        self.analyze_syntax(
            || true,
            |document, _| document.and_then(|document| document.highlights().cloned()),
        )
        .flatten()
    }

    fn analyze_syntax<T>(
        self,
        mut should_continue: impl FnMut() -> bool,
        result_for: impl FnOnce(
            Option<&openwebide_core::editor::SyntaxAnalysis>,
            openwebide_core::editor::SyntaxStatus,
        ) -> T,
    ) -> Option<T> {
        let key = self.key()?;
        let epoch = self.workspace.pending_epoch.get_untracked();
        let read_revision = self.workspace.editor_read_revision.get_untracked();
        let account_generation = self.auth.map_or(0, |auth| auth.generation.get_untracked());
        let language = openwebide_core::highlight::language_from_path(&key.1);
        let text = self.workspace.content.get_untracked();
        let tab_width = self.rules_untracked().indentation.tab_width();
        let result = if self.workspace.editor_worker_active.get_untracked() {
            if !should_continue() {
                Some(result_for(
                    None,
                    openwebide_core::editor::SyntaxStatus::Cancelled,
                ))
            } else {
                let scope = self.syntax_scope()?;
                self.workspace
                    .editor_preparation
                    .with_untracked(|prepared| {
                        prepared
                            .as_ref()
                            .filter(|prepared| prepared.scope == scope)
                            .map(|prepared| {
                                result_for(prepared.analysis.as_deref(), prepared.status)
                            })
                    })
            }
        } else {
            let deadline = js_sys::Date::now() + 12.0;
            self.workspace
                .editor_syntax
                .try_update(|documents| {
                    let (status, prepared) =
                        documents.prepare(key.clone(), language, &text, tab_width, || {
                            js_sys::Date::now() <= deadline && should_continue()
                        });
                    Some(result_for(prepared.as_deref(), status))
                })
                .flatten()
        };
        (self.key() == Some(key)
            && self.workspace.pending_epoch.get_untracked() == epoch
            && self.workspace.editor_read_revision.get_untracked() == read_revision
            && self.auth.map_or(0, |auth| auth.generation.get_untracked()) == account_generation
            && self.rules_untracked().indentation.tab_width() == tab_width
            && self
                .workspace
                .content
                .with_untracked(|current| current == &text))
        .then_some(result)
        .flatten()
    }

    pub fn refresh_fold_ranges(
        self,
        should_continue: impl FnMut() -> bool,
    ) -> Option<openwebide_core::editor::SyntaxStatus> {
        let key = self.key()?;
        let epoch = self.workspace.pending_epoch.get_untracked();
        let text = self.workspace.content.get_untracked();
        let result = self.syntax_folds(should_continue)?;
        if self.key() != Some(key.clone())
            || self.workspace.pending_epoch.get_untracked() != epoch
            || self
                .workspace
                .content
                .with_untracked(|current| current != &text)
        {
            return None;
        }
        let ranges = result.1;
        let changed = self
            .workspace
            .editor_documents
            .try_update(|documents| self.document(documents, key).set_fold_ranges(ranges))
            .unwrap_or(false);
        if changed {
            self.workspace
                .editor_fold_revision
                .update(|value| *value = value.wrapping_add(1));
        }
        Some(result.0)
    }

    pub fn cursor_status(self) -> (usize, usize, usize) {
        self.workspace.content.with_untracked(|source| {
            let selection = self.selection(source).unwrap_or_default();
            let position = self
                .key()
                .and_then(|key| {
                    self.workspace.editor_documents.with_untracked(|documents| {
                        documents
                            .get(&key)
                            .filter(|document| document.text() == source)
                            .map(|document| document.line_column(selection.head))
                    })
                })
                .unwrap_or_else(|| openwebide_core::editor::line_column(source, selection.head));
            (
                position.0,
                position.1,
                source[selection.range()].chars().count(),
            )
        })
    }

    pub fn search(
        self,
        source: &str,
        query: &str,
        options: openwebide_core::editor::SearchOptions,
        scope: Option<std::ops::Range<usize>>,
    ) -> Result<Vec<openwebide_core::editor::SearchMatch>, openwebide_core::editor::SearchError>
    {
        openwebide_core::editor::SearchPattern::new(query, options)?.find(source, scope)
    }

    pub fn replace_search(
        self,
        source: &str,
        query: &str,
        options: openwebide_core::editor::SearchOptions,
        scope: Option<std::ops::Range<usize>>,
        replacement: &str,
        index: Option<usize>,
    ) -> Result<Option<Selection>, openwebide_core::editor::SearchError> {
        use openwebide_core::editor::{SearchError, SearchPattern};
        if self.workspace.is_resolving() || self.workspace.pending_diff.get_untracked().is_some() {
            return Err(SearchError::ReadOnly);
        }
        if self.source() != source {
            return Err(SearchError::ChangedDocument);
        }
        let Some(key) = self.key() else {
            return Ok(None);
        };
        let pattern = SearchPattern::new(query, options)?;
        self.typing.set(None);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                if document.replace_search(&pattern, scope, replacement, index)? == 0 {
                    return Ok(None);
                }
                Ok(Some((
                    document.text().to_string(),
                    document.selections()[0],
                )))
            })
            .unwrap_or(Ok(None));
        if let Ok(Some((text, _))) = &result {
            self.workspace.content.set(text.clone());
            self.publish_dirty(key);
        }
        result.map(|result| result.map(|(_, selection)| selection))
    }

    pub fn navigation_target(self, query: &str) -> Option<usize> {
        openwebide_core::editor::navigation_target(&self.source(), query)
    }

    pub fn matching_bracket(self, offset: usize) -> Option<(usize, usize)> {
        let key = self.key()?;
        let source = self.source();
        if !openwebide_core::editor::has_adjacent_bracket(&source, offset) {
            return None;
        }
        let syntax = self.syntax_structure(|| true);
        if self.key() != Some(key.clone()) || self.source() != source {
            return None;
        }
        if let Some(context) = syntax {
            openwebide_core::editor::matching_bracket_with_context(&source, &context, offset)
        } else {
            openwebide_core::editor::matching_bracket(
                &source,
                openwebide_core::highlight::language_from_path(&key.1),
                offset,
            )
        }
    }

    pub fn source(self) -> String {
        self.workspace.content.get_untracked()
    }

    pub fn navigate(self, offset: usize) -> Result<Selection, EditError> {
        let source = self.workspace.content.get_untracked();
        let selection = Selection::caret(offset);
        self.record_selection(selection)?;
        let (line, _) = openwebide_core::editor::line_column(&source, offset);
        self.fold_command(openwebide_core::editor::FoldCommand::Reveal(line - 1));
        Ok(selection)
    }

    pub fn prepare_edit(self, selection: Selection) -> Result<(), EditError> {
        self.record_native_selection(selection)?;
        let Some(key) = self.key() else {
            return Ok(());
        };
        let changed = self
            .workspace
            .editor_documents
            .try_update(|documents| self.document(documents, key).reveal_selection())
            .unwrap_or(false);
        if changed {
            self.workspace
                .editor_fold_revision
                .update(|value| *value = value.wrapping_add(1));
        }
        Ok(())
    }

    pub fn fold_command(
        self,
        command: openwebide_core::editor::FoldCommand,
    ) -> Option<(openwebide_core::editor::FoldProjection, Selection)> {
        let key = self.key()?;
        self.cancel_queued_motion(None);
        self.typing.set(None);
        let result = self.workspace.editor_documents.try_update(|documents| {
            let document = self.document(documents, key);
            let changed = document.fold_command(command);
            (changed, (document.projection(), document.selections()[0]))
        });
        if result.as_ref().is_some_and(|(changed, _)| *changed) {
            self.workspace
                .editor_fold_revision
                .update(|value| *value = value.wrapping_add(1));
        }
        result.map(|(_, view)| view)
    }

    pub fn projection(self) -> Option<openwebide_core::editor::FoldProjection> {
        let key = self.key()?;
        self.workspace.content.with_untracked(|text| {
            self.workspace.editor_documents.with_untracked(|documents| {
                documents
                    .get(&key)
                    .filter(|document| document.text() == text)
                    .map(Document::projection)
            })
        })
    }

    pub fn fold_state(self) -> Option<openwebide_core::editor::FoldState> {
        let key = self.key()?;
        self.workspace.content.with_untracked(|text| {
            self.workspace.editor_documents.with_untracked(|documents| {
                documents
                    .get(&key)
                    .filter(|document| document.text() == text)
                    .map(|document| document.fold_state().clone())
            })
        })
    }

    pub fn projected_input(
        self,
        value: String,
        selection: Selection,
        input_type: &str,
        timestamp: f64,
    ) -> Result<(), EditError> {
        let source = self.workspace.content.get_untracked();
        let Some(projection) = self
            .projection()
            .filter(openwebide_core::editor::FoldProjection::is_folded)
        else {
            return self.native_input(value, selection, input_type, timestamp);
        };
        let (value, selection) = projection
            .replay_input(
                &source,
                &value,
                selection,
                input_type,
                self.selection(&source).unwrap_or_default(),
            )
            .map_err(|_| EditError::InvalidRange)?;
        self.native_input(value, selection, input_type, timestamp)
    }

    fn native_history_group(
        self,
        key: &(i64, String),
        input_type: &str,
        timestamp: f64,
    ) -> (bool, Option<u64>) {
        let coalesces = matches!(
            input_type,
            "insertText"
                | "deleteContentBackward"
                | "deleteContentForward"
                | "insertCompositionText"
        );
        let same = coalesces
            && self.typing.with_untracked(|typing| {
                typing.as_ref().is_some_and(|(previous, kind, time)| {
                    previous == key
                        && kind == input_type
                        && timestamp >= *time
                        && timestamp - time <= 750.0
                })
            });
        let composing = self.is_composing();
        if !same && !composing {
            self.group.update(|group| *group = group.wrapping_add(1));
        }
        let group = (coalesces || composing).then(|| self.group.get_untracked());
        (coalesces, group)
    }

    pub fn cancel_native_text(self) {
        self.workspace.editor_text_insertion.set(None);
    }

    /// Capture the declared insertion while the browser still owns its original caret.
    pub fn begin_native_text(self, text: String) {
        self.cancel_native_text();
        if self.is_composing() {
            return;
        }
        let Some(key) = self.key() else {
            return;
        };
        let insertion = self.workspace.editor_documents.with_untracked(|documents| {
            let document = documents.get(&key)?;
            let projection = document.projection();
            let visible = projection
                .visible_selection(document.selections()[0])
                .ok()?;
            let at = projection.byte_to_textarea(visible.range().start).ok()?;
            let added = text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .encode_utf16()
                .count();
            Some(crate::state::workspace::EditorTextInsertion {
                key: key.clone(),
                source_revision: self.workspace.editor_source_revision.get_untracked(),
                projection,
                document_revision: document.revision(),
                account_generation: self.auth.map_or(0, |auth| auth.generation.get_untracked()),
                selections: document.selections().to_vec(),
                native_caret: at.checked_add(added)?,
                text,
            })
        });
        self.workspace.editor_text_insertion.set(insertion);
    }

    /// Consume a matching native commit without copying/diffing the complete DOM value.
    /// Source/account changes reject the commit; browser transformations use replay.
    pub fn finish_native_text(
        self,
        data: Option<&str>,
        native: Selection,
        timestamp: f64,
    ) -> Result<Option<NativeTextCommit>, EditError> {
        let Some(insertion) = self.workspace.editor_text_insertion.get_untracked() else {
            return Ok(None);
        };
        self.cancel_native_text();
        let current = self.key().as_ref() == Some(&insertion.key)
            && self.workspace.editor_source_revision.get_untracked() == insertion.source_revision
            && self.auth.map_or(0, |auth| auth.generation.get_untracked())
                == insertion.account_generation
            && !self.is_composing()
            && self.workspace.editor_documents.with_untracked(|documents| {
                documents.get(&insertion.key).is_some_and(|document| {
                    document.revision() == insertion.document_revision
                        && document.projection() == insertion.projection
                        && document.selections() == insertion.selections
                })
            });
        if !current {
            return Err(EditError::StaleContext);
        }
        if data != Some(insertion.text.as_str())
            || native != Selection::caret(insertion.native_caret)
        {
            return Ok(None);
        }
        let retain_native_value =
            insertion.selections.len() == 1 && !insertion.projection.is_folded();
        self.insert_native_text(&insertion.text, timestamp)
            .map(|selection| {
                selection.map(|selection| NativeTextCommit {
                    selection,
                    retain_native_value,
                })
            })
    }

    /// Cancellable text events supply their insertion, not a complete DOM value.
    pub fn insert_native_text(
        self,
        text: &str,
        timestamp: f64,
    ) -> Result<Option<Selection>, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        let Some(key) = self.key() else {
            return Ok(None);
        };
        let (_, group) = self.native_history_group(&key, "insertText", timestamp);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                document.insert_native_text(text, group)?;
                Ok((document.text().to_string(), document.selections()[0]))
            })
            .unwrap_or(Err(EditError::InvalidSelection));
        match result {
            Ok((source, selection)) => {
                self.workspace.content.set(source);
                self.publish_dirty(key.clone());
                self.typing.set(Some((key, "insertText".into(), timestamp)));
                Ok(Some(selection))
            }
            Err(error) => {
                self.typing.set(None);
                Err(error)
            }
        }
    }

    pub fn native_input(
        self,
        text: String,
        selection: Selection,
        input_type: &str,
        timestamp: f64,
    ) -> Result<(), EditError> {
        let Some(key) = self.key() else {
            return Ok(());
        };
        let composing = self.is_composing();
        if input_type == "insertFromComposition"
            && !composing
            && text == self.source().replace("\r\n", "\n").replace('\r', "\n")
        {
            return Ok(());
        }
        if matches!(
            input_type,
            "insertCompositionText" | "insertFromComposition"
        ) && !composing
        {
            return Err(EditError::UnsupportedNativeInput);
        }
        if composing
            && !self.workspace.editor_documents.with_untracked(|documents| {
                documents.get(&key).is_some_and(|document| {
                    document.is_composing() && document.text() == self.source()
                })
            })
        {
            self.cancel_composition();
            return Err(EditError::UnsupportedNativeInput);
        }
        let (coalesces, group) = self.native_history_group(&key, input_type, timestamp);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                let outcome = (|| {
                    let edit = document.native_replacement(&text);
                    let mut candidate = document.text().to_string();
                    if let Some(edit) = &edit {
                        candidate.replace_range(edit.range.clone(), &edit.text);
                    }
                    let after = Selection {
                        anchor: openwebide_core::editor::textarea_to_byte(
                            &candidate,
                            openwebide_core::editor::byte_to_utf16(&text, selection.anchor)?,
                        ),
                        head: openwebide_core::editor::textarea_to_byte(
                            &candidate,
                            openwebide_core::editor::byte_to_utf16(&text, selection.head)?,
                        ),
                    };
                    document.native_input(
                        &candidate,
                        after,
                        openwebide_core::editor::NativeInputKind::from_input_type(input_type),
                        group,
                    )?;
                    Ok(())
                })();
                if outcome.is_err() {
                    document.cancel_composition();
                }
                Some((outcome, document.text().to_string()))
            })
            .flatten();
        if let Some((outcome, text)) = result {
            self.workspace.content.set(text);
            self.publish_dirty(key.clone());
            self.typing
                .set((outcome.is_ok() && coalesces).then(|| (key, input_type.into(), timestamp)));
            if outcome.is_err() && composing {
                self.workspace.editor_composition.set(None);
            }
            return outcome;
        }
        Ok(())
    }

    pub fn command(
        self,
        command: EditorCommand,
        selection: Selection,
        indentation: Indentation,
    ) -> Result<Option<(String, Selection)>, EditError> {
        let Some(key) = self.key() else {
            return Ok(None);
        };
        let syntax = if matches!(
            command,
            EditorCommand::TypeCharacter(_)
                | EditorCommand::DeletePair
                | EditorCommand::Newline
                | EditorCommand::Reindent
                | EditorCommand::BlockComment
                | EditorCommand::LineComment
        ) {
            self.syntax_structure(|| true)
        } else {
            None
        };
        if self.key() != Some(key.clone()) {
            return Ok(None);
        }
        self.typing.set(None);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                if document.selections().first() != Some(&selection) {
                    document.set_selections(vec![selection])?;
                }
                match command {
                    EditorCommand::ConvertIndentation => {
                        document.convert_indentation(indentation, indentation.tab_width())?;
                    }
                    EditorCommand::Tab => {
                        document.tab(indentation)?;
                    }
                    EditorCommand::Outdent => {
                        document.indent_lines(indentation, true)?;
                    }
                    EditorCommand::Newline => {
                        if let Some(syntax) = &syntax {
                            document.newline_with_context(
                                indentation,
                                self.rules_untracked().line_ending,
                                syntax,
                            )?;
                        } else {
                            document.newline_with_structure(
                                indentation,
                                self.rules_untracked().line_ending,
                                openwebide_core::highlight::language_from_path(&key.1),
                            )?;
                        }
                    }
                    EditorCommand::TypeCharacter(ch) => {
                        if let Some(syntax) = &syntax {
                            document.type_character_with_context(ch, syntax)?;
                        } else {
                            document.type_character(
                                ch,
                                openwebide_core::highlight::language_from_path(&key.1),
                            )?;
                        }
                    }
                    EditorCommand::DeletePair => {
                        let changed = if let Some(syntax) = &syntax {
                            document.delete_pairs_with_context(syntax)?
                        } else {
                            document.delete_empty_pairs(
                                openwebide_core::highlight::language_from_path(&key.1),
                            )?
                        };
                        if !changed {
                            return Ok(None);
                        }
                    }
                    EditorCommand::Line(command) => {
                        document.line_command(command, self.rules_untracked().line_ending)?;
                    }
                    EditorCommand::DuplicateSelection => {
                        document.duplicate_selections()?;
                    }
                    EditorCommand::LineComment => {
                        if let Some(syntax) = &syntax {
                            document.toggle_line_comments_with_context(syntax)?;
                        } else {
                            document.toggle_line_comments(
                                openwebide_core::highlight::language_from_path(&key.1),
                            )?;
                        }
                    }
                    EditorCommand::BlockComment => {
                        if let Some(syntax) = &syntax {
                            document.toggle_block_comments_with_context(syntax)?;
                        } else {
                            document.toggle_block_comments(
                                openwebide_core::highlight::language_from_path(&key.1),
                            )?;
                        }
                    }
                    EditorCommand::Reindent => {
                        if let Some(syntax) = &syntax {
                            document.reindent_with_context(indentation, syntax)?;
                        } else {
                            document.reindent(
                                indentation,
                                openwebide_core::highlight::language_from_path(&key.1),
                            )?;
                        }
                    }
                    EditorCommand::Undo => {
                        document.undo();
                    }
                    EditorCommand::Redo => {
                        document.redo();
                    }
                }
                Ok(Some((
                    document.text().to_string(),
                    document.selections()[0],
                )))
            })
            .unwrap_or(Ok(None));
        if let Ok(Some((text, _))) = &result {
            self.workspace.content.set(text.clone());
            self.publish_dirty(key);
        }
        result
    }

    pub fn clipboard_content(
        self,
        selection: Selection,
    ) -> Result<Option<openwebide_core::editor::ClipboardContent>, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        self.record_native_selection(selection)?;
        let Some(key) = self.key() else {
            return Ok(None);
        };
        self.workspace.editor_documents.with_untracked(|documents| {
            documents
                .get(&key)
                .filter(|document| document.text() == self.source())
                .map(Document::clipboard_content)
                .transpose()
        })
    }

    pub fn paste(self, text: &str, selection: Selection) -> Result<Option<Selection>, EditError> {
        self.paste_clipboard(text, None, selection)
    }

    pub fn paste_clipboard(
        self,
        text: &str,
        metadata: Option<&str>,
        selection: Selection,
    ) -> Result<Option<Selection>, EditError> {
        self.edit_clipboard(Some((text, metadata)), selection)
    }

    pub fn cut(self, selection: Selection) -> Result<Option<Selection>, EditError> {
        self.edit_clipboard(None, selection)
    }

    fn edit_clipboard(
        self,
        pasted: Option<(&str, Option<&str>)>,
        selection: Selection,
    ) -> Result<Option<Selection>, EditError> {
        self.record_native_selection(selection)?;
        let Some(key) = self.key() else {
            return Ok(None);
        };
        self.typing.set(None);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                if let Some((text, metadata)) = pasted {
                    document.paste_clipboard(text, metadata)?;
                } else {
                    document.replace_selections("", None)?;
                }
                Ok(Some((
                    document.text().to_string(),
                    document.selections()[0],
                )))
            })
            .unwrap_or(Ok(None));
        if let Ok(Some((text, _))) = &result {
            self.workspace.content.set(text.clone());
            self.publish_dirty(key);
        }
        result.map(|value| value.map(|(_, selection)| selection))
    }

    pub fn paste_with_indentation(
        self,
        text: &str,
        selection: Selection,
    ) -> Result<Option<(String, Selection)>, EditError> {
        self.paste_clipboard_with_indentation(text, None, selection)
    }

    pub fn paste_clipboard_with_indentation(
        self,
        text: &str,
        metadata: Option<&str>,
        selection: Selection,
    ) -> Result<Option<(String, Selection)>, EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        let Some(key) = self.key() else {
            return Ok(None);
        };
        self.typing.set(None);
        let rules = self.rules_untracked();
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                if document.selections().first() != Some(&selection) {
                    document.set_selections(vec![selection])?;
                }
                document.paste_clipboard_with_indentation(
                    text,
                    metadata,
                    rules.indentation,
                    rules.line_ending,
                )?;
                Ok(Some((
                    document.text().to_string(),
                    document.selections()[0],
                )))
            })
            .unwrap_or(Ok(None));
        if let Ok(Some((text, _))) = &result {
            self.workspace.content.set(text.clone());
            self.publish_dirty(key);
        }
        result
    }

    fn publish_dirty(self, key: (i64, String)) {
        let dirty = self
            .workspace
            .editor_documents
            .with_untracked(|documents| documents.get(&key).is_some_and(Document::is_dirty));
        self.workspace.dirty.set(dirty);
    }
}
