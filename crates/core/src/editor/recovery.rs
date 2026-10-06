//! Versioned recovery data: hosts persist it, shared Rust validates and restores it.
use super::{
    Document, Edit, FoldRange, MAX_DOCUMENT_BYTES, MAX_SELECTIONS, Selection, normalize_folds,
    normalize_selections,
};
use serde::{Deserialize, Serialize};

pub const MAX_RECOVERY_FILES: usize = 64;
pub const MAX_RECOVERY_BYTES: usize = 128 * 1024 * 1024;
const RECOVERY_BODY_LIMIT: usize = 192 * 1024 * 1024;
const MAX_RECOVERY_FOLDS: usize = 100_000;

pub const fn recovery_body_limit() -> usize {
    RECOVERY_BODY_LIMIT
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentRecovery {
    #[serde(with = "encoded_text")]
    pub text: String,
    #[serde(with = "encoded_text")]
    pub saved: String,
    pub selections: Vec<Selection>,
    pub collapsed: Vec<FoldRange>,
}

impl Document {
    /// IME previews are transient; persistence captures the last committed state.
    pub fn recovery(&self) -> DocumentRecovery {
        let document = self
            .composition
            .as_ref()
            .map_or(self, |composition| composition.committed_document());
        DocumentRecovery {
            text: document.text.clone(),
            saved: document.saved.clone(),
            selections: document.selections.clone(),
            collapsed: document
                .folds
                .ranges()
                .iter()
                .filter(|range| document.folds.collapsed_at(range.start_line).is_some())
                .copied()
                .collect(),
        }
    }
}

impl DocumentRecovery {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.len() > MAX_DOCUMENT_BYTES || self.saved.len() > MAX_DOCUMENT_BYTES {
            return Err("Recovered document exceeds the editor's size limit".into());
        }
        if self.selections.is_empty() || self.selections.len() > MAX_SELECTIONS {
            return Err("Recovered selections exceed the editor's limits".into());
        }
        normalize_selections(&self.text, self.selections.clone())
            .map_err(|error| error.to_string())?;
        if self.collapsed.len() > MAX_RECOVERY_FOLDS {
            return Err("Too many recovered folds".into());
        }
        let lines = self.text.bytes().filter(|byte| *byte == b'\n').count() + 1;
        if normalize_folds(self.collapsed.clone(), lines) != self.collapsed {
            return Err("Recovered folds are outside the document or overlap".into());
        }
        Ok(())
    }

    /// The recovered draft is one undoable change against its original disk text.
    pub fn restore(&self) -> Result<Document, String> {
        self.validate()?;
        let mut document = Document::new(self.saved.clone());
        if self.text != self.saved {
            document
                .apply(
                    vec![Edit::replace(0..self.saved.len(), self.text.clone())],
                    self.selections.clone(),
                    None,
                )
                .map_err(|error| error.to_string())?;
        } else {
            document
                .set_selections(self.selections.clone())
                .map_err(|error| error.to_string())?;
        }
        let lines = self.text.bytes().filter(|byte| *byte == b'\n').count() + 1;
        document.folds.set_ranges(self.collapsed.clone(), lines);
        document.folds.collapse_all();
        Ok(document)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryScroll {
    pub top: f64,
    pub left: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRecoveryFile {
    pub path: String,
    pub document: Option<DocumentRecovery>,
    pub scroll: RecoveryScroll,
    pub read_only: bool,
}

impl EditorRecoveryFile {
    /// Conservative metadata costs keep encoded payloads within the wire budget.
    pub fn recovery_bytes(&self) -> usize {
        self.document.as_ref().map_or(self.path.len(), |document| {
            self.path
                .len()
                .saturating_add(document.text.len())
                .saturating_add(document.saved.len())
                .saturating_add(document.collapsed.len().saturating_mul(96))
                .saturating_add(document.selections.len().saturating_mul(64))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRecoveryRoot {
    pub mode: crate::WorkspaceMode,
    pub path: Option<String>,
}
impl EditorRecoveryRoot {
    /// Host capability boundary: local roots are origin-bound directory handles.
    pub fn for_project(project: &crate::Project) -> Self {
        Self {
            mode: project.mode,
            path: match project.mode {
                crate::WorkspaceMode::Remote => project.path.clone(),
                crate::WorkspaceMode::Local => None,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRecovery {
    pub format: u32,
    pub root: Option<EditorRecoveryRoot>,
    pub selected: Option<String>,
    pub files: Vec<EditorRecoveryFile>,
}
impl Default for EditorRecovery {
    fn default() -> Self {
        Self {
            format: 1,
            root: None,
            selected: None,
            files: Vec::new(),
        }
    }
}
impl EditorRecovery {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1 {
            return Err("Unsupported editor recovery format".into());
        }
        if !self.files.is_empty() && self.root.is_none() {
            return Err("Recovered files have no project root".into());
        }
        if self.root.as_ref().is_some_and(|root| {
            root.path
                .as_ref()
                .is_some_and(|path| path.len() > 4096 || path.contains('\0'))
        }) {
            return Err("Recovered project root is invalid".into());
        }
        if self.files.len() > MAX_RECOVERY_FILES {
            return Err("Too many open files for editor recovery".into());
        }
        let mut paths = std::collections::HashSet::new();
        let mut bytes = 0_usize;
        for file in &self.files {
            if file.path.len() > 4096
                || crate::vfs::workspace_path(&file.path).map_err(|error| error.to_string())?
                    != file.path
                || file.path.is_empty()
                || !paths.insert(&file.path)
            {
                return Err("Recovered file paths must be unique project-relative paths".into());
            }
            if !file.scroll.top.is_finite()
                || !file.scroll.left.is_finite()
                || file.scroll.top < 0.0
                || file.scroll.left < 0.0
            {
                return Err("Recovered scroll position is invalid".into());
            }
            if let Some(document) = &file.document {
                document.validate()?;
            }
            bytes = bytes.saturating_add(file.recovery_bytes());
            if bytes > MAX_RECOVERY_BYTES {
                return Err(
                    "Editor recovery exceeds 128 MiB; keep these files open while saving them"
                        .into(),
                );
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !paths.contains(selected))
        {
            return Err("Selected recovered file is not an open tab".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRecoveryRecord {
    pub revision: i64,
    pub state: EditorRecovery,
}
// Encoding bounds escaped Unicode/control-heavy files without a second JSON layer.
mod encoded_text {
    use super::MAX_DOCUMENT_BYTES;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(value: &str, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(value.as_bytes()))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() > MAX_DOCUMENT_BYTES.div_ceil(3) * 4 {
            return Err(D::Error::custom(
                "Recovered text exceeds document size limit",
            ));
        }
        let bytes = STANDARD.decode(value).map_err(D::Error::custom)?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(D::Error::custom(
                "Recovered text exceeds document size limit",
            ));
        }
        String::from_utf8(bytes).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::super::NativeInputKind;
    use super::*;

    #[test]
    fn recovery_preserves_unicode_crlf_baseline_selections_folds_and_undo() {
        let mut document = Document::new("fn f() {\r\n  α\r\n}\r\n");
        document.set_selections(vec![Selection::caret(14)]).unwrap();
        document.replace_selections("😀", None).unwrap();
        document.folds.set_ranges(
            vec![FoldRange {
                start_line: 0,
                end_line: 2,
            }],
            4,
        );
        document.folds.toggle(0);
        let snapshot = document.recovery();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains('α'));
        let decoded: DocumentRecovery = serde_json::from_str(&json).unwrap();
        let mut restored = decoded.restore().unwrap();
        assert_eq!(restored.text(), document.text());
        assert_eq!(restored.selections(), document.selections());
        assert!(restored.is_dirty());
        assert!(restored.folds.collapsed_at(0).is_some());
        assert!(restored.undo());
        assert_eq!(restored.text(), snapshot.saved);
        assert!(!restored.is_dirty());
        assert!(restored.redo());
        assert_eq!(restored.text(), snapshot.text);
    }

    #[test]
    fn recovery_omits_composition_previews_and_keeps_completed_writes() {
        let mut document = Document::new("base");
        document.replace_selections("x", None).unwrap();
        let before = document.recovery();
        document.begin_composition(None);
        document
            .native_input(
                "文xbase",
                Selection::caret(3),
                NativeInputKind::Insert,
                None,
            )
            .unwrap();
        assert_eq!(document.recovery(), before);
        document.mark_saved_version("written");
        assert_eq!(document.recovery().saved, "written");
        assert_eq!(document.recovery().text, "xbase");
    }

    #[test]
    fn recovery_rejects_malformed_text_selection_folds_paths_versions_and_scroll() {
        let mut snapshot = Document::new("α\nline\n").recovery();
        snapshot.selections = vec![Selection::caret(1)];
        assert!(snapshot.validate().is_err());
        snapshot.selections = vec![Selection::caret(0)];
        snapshot.collapsed = vec![FoldRange {
            start_line: 0,
            end_line: 9,
        }];
        assert!(snapshot.validate().is_err());
        let invalid_utf8 =
            r#"{"text":"/w==","saved":"","selections":[{"anchor":0,"head":0}],"collapsed":[]}"#;
        assert!(serde_json::from_str::<DocumentRecovery>(invalid_utf8).is_err());
        let invalid_base64 = invalid_utf8.replace("/w==", "garbage!");
        assert!(serde_json::from_str::<DocumentRecovery>(&invalid_base64).is_err());
        let file = EditorRecoveryFile {
            path: "a.rs".into(),
            document: None,
            scroll: RecoveryScroll::default(),
            read_only: false,
        };
        let mut state = EditorRecovery {
            format: 1,
            root: Some(EditorRecoveryRoot {
                mode: crate::WorkspaceMode::Local,
                path: None,
            }),
            selected: Some(file.path.clone()),
            files: vec![file],
        };
        state.validate().unwrap();
        state.files[0].path = "../outside".into();
        assert!(state.validate().is_err());
        state.files[0].path = "a.rs".into();
        state.files.push(state.files[0].clone());
        assert!(state.validate().is_err());
        state.files.pop();
        state.selected = Some("missing.rs".into());
        assert!(state.validate().is_err());
        state.selected = None;
        state.files[0].scroll.top = f64::INFINITY;
        assert!(state.validate().is_err());
        state.files[0].scroll.top = -1.0;
        assert!(state.validate().is_err());
        state.files[0].scroll.top = 0.0;
        state.format = 2;
        assert!(state.validate().is_err());
        assert_eq!(
            serde_json::from_str::<EditorRecovery>(
                &serde_json::to_string(&EditorRecovery::default()).unwrap()
            )
            .unwrap(),
            EditorRecovery::default()
        );
    }
}
