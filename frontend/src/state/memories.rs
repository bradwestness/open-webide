use leptos::prelude::*;
use openwebide_core::ProjectMemories;
#[derive(Clone, Copy)]
pub struct MemoriesState {
    pub data: RwSignal<Option<ProjectMemories>>,
    pub loading: RwSignal<bool>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub editing: RwSignal<bool>,
    pub edit_id: RwSignal<Option<(i64, i64)>>,
    pub title: RwSignal<String>,
    pub content: RwSignal<String>,
}
impl MemoriesState {
    pub fn new() -> Self {
        Self {
            data: RwSignal::new(None),
            loading: RwSignal::new(false),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
            editing: RwSignal::new(false),
            edit_id: RwSignal::new(None),
            title: RwSignal::new(String::new()),
            content: RwSignal::new(String::new()),
        }
    }
}
impl Default for MemoriesState {
    fn default() -> Self {
        Self::new()
    }
}
