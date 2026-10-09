use leptos::prelude::*;
use openwebide_core::{ProjectSkills, SkillDraft};
#[derive(Clone, Copy)]
pub struct SkillsState {
    pub data: RwSignal<Option<ProjectSkills>>,
    pub loading: RwSignal<bool>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub editing: RwSignal<bool>,
    pub edit_id: RwSignal<Option<(i64, i64)>>,
    pub draft: RwSignal<SkillDraft>,
}
pub fn empty_draft() -> SkillDraft {
    SkillDraft {
        name: String::new(),
        description: String::new(),
        instructions: String::new(),
        enabled: true,
        resources: Vec::new(),
        metadata: Default::default(),
    }
}
impl SkillsState {
    pub fn new() -> Self {
        Self {
            data: RwSignal::new(None),
            loading: RwSignal::new(false),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
            editing: RwSignal::new(false),
            edit_id: RwSignal::new(None),
            draft: RwSignal::new(empty_draft()),
        }
    }
}
impl Default for SkillsState {
    fn default() -> Self {
        Self::new()
    }
}
