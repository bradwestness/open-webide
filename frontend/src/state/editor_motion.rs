//! Temporary cursor requests belong to the active document and account.
use openwebide_core::editor::{EditorPreferences, Indentation, MotionQueue};

#[derive(Clone, Debug)]
pub struct PendingEditorMotion {
    pub ticket: u64,
    pub key: (i64, String),
    pub epoch: u64,
    pub read_revision: u64,
    pub account_generation: u64,
    pub preferences: EditorPreferences,
    pub indentation: Indentation,
    pub queue: MotionQueue,
}
