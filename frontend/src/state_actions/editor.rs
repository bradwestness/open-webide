//! Shared editor facade. DOM adapters provide text/selection/events; editing policy
//! lives in the Rust document engine without filesystem-mode branches.
use leptos::prelude::*;
use openwebide_core::editor::{Document, Edit, EditError, Indentation, Selection};

use crate::state::workspace::WorkspaceState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorCommand {
    Tab,
    Outdent,
    Newline,
    Undo,
    Redo,
    ConvertIndentation,
}

type TypingState = Option<((i64, String), String, f64)>;

#[derive(Clone, Copy)]
pub struct EditorActions {
    workspace: WorkspaceState,
    preferences: Option<crate::state::settings::SettingsState>,
    group: RwSignal<u64>,
    typing: RwSignal<TypingState>,
    composing: RwSignal<bool>,
}

impl EditorActions {
    pub fn new(workspace: WorkspaceState) -> Self {
        Self {
            workspace,
            preferences: use_context::<crate::state::settings::SettingsState>(),
            group: workspace.editor_group,
            typing: RwSignal::new(None),
            composing: RwSignal::new(false),
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
        self.group.update(|group| *group = group.wrapping_add(1));
        self.typing.set(None);
        self.composing.set(true);
    }

    pub fn end_composition(self) {
        self.composing.set(false);
        self.typing.set(None);
    }

    fn key(self) -> Option<(i64, String)> {
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
        let text = self.workspace.content.get_untracked();
        let document = documents
            .entry(key)
            .or_insert_with(|| Document::new(text.clone()));
        if document.text() != text {
            *document = Document::new(text);
        }
        if !self.workspace.dirty.get_untracked() {
            document.mark_saved();
        }
        document
    }

    pub fn record_selection(self, selection: Selection) -> Result<(), EditError> {
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
                    previous == &key
                        && kind == input_type
                        && timestamp >= *time
                        && timestamp - time <= 750.0
                })
            });
        if !same && !self.composing.get_untracked() {
            self.group.update(|group| *group = group.wrapping_add(1));
        }
        let group =
            (coalesces || self.composing.get_untracked()).then(|| self.group.get_untracked());
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                let edit = replacement(document.text(), &text);
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
                document.apply(edit.into_iter().collect(), vec![after], group)?;
                Ok(Some(candidate))
            })
            .unwrap_or(Ok(None));
        if let Ok(Some(text)) = &result {
            self.workspace.content.set(text.clone());
            self.publish_dirty(key.clone());
            self.typing
                .set(coalesces.then(|| (key, input_type.into(), timestamp)));
        }
        result.map(|_| ())
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
        self.typing.set(None);
        let result = self
            .workspace
            .editor_documents
            .try_update(|documents| {
                let document = self.document(documents, key.clone());
                document.set_selections(vec![selection])?;
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
                        document.newline_with_ending(self.rules_untracked().line_ending)?;
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

    fn publish_dirty(self, key: (i64, String)) {
        let dirty = self
            .workspace
            .editor_documents
            .with_untracked(|documents| documents.get(&key).is_some_and(Document::is_dirty));
        self.workspace.dirty.set(dirty);
    }
}

/// Minimal native-input replacement, preserving UTF-8 boundaries and distant text.
fn replacement(old: &str, new: &str) -> Option<Edit> {
    let normalized = old.replace("\r\n", "\n").replace('\r', "\n");
    if normalized == new {
        return None;
    }
    let start = normalized
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = normalized[start..]
        .chars()
        .rev()
        .zip(new[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let end = normalized.len() - suffix;
    let start_doc =
        openwebide_core::editor::textarea_to_byte(old, normalized[..start].encode_utf16().count());
    let end_doc =
        openwebide_core::editor::textarea_to_byte(old, normalized[..end].encode_utf16().count());
    let mut inserted = new[start..new.len() - suffix].to_string();
    if old
        .split_once('\n')
        .is_some_and(|(prefix, _)| prefix.ends_with('\r'))
    {
        inserted = inserted.replace('\n', "\r\n");
    }
    Some(Edit::replace(start_doc..end_doc, inserted))
}
