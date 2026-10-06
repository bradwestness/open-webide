use std::collections::HashMap;

use leptos::prelude::*;
use openwebide_core::editor::EditorRecoveryRecord;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryPhase {
    Loading,
    Ready,
    LoadFailed(String),
    SaveFailed(String),
    Conflict(String),
}

#[derive(Clone, Debug)]
pub struct RecoveryProject {
    pub record: Option<EditorRecoveryRecord>,
    pub phase: RecoveryPhase,
    pub ticket: u64,
    pub writing: bool,
}

/// Database recovery state is separate from editable workspace buffers. Errors
/// retain both the last acknowledged revision and the user's current documents.
#[derive(Clone, Copy)]
pub struct EditorRecoveryState {
    pub projects: RwSignal<HashMap<i64, RecoveryProject>>,
}

impl EditorRecoveryState {
    pub fn new() -> Self {
        Self {
            projects: RwSignal::new(HashMap::new()),
        }
    }
}

impl Default for EditorRecoveryState {
    fn default() -> Self {
        Self::new()
    }
}
