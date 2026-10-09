use leptos::prelude::*;
use openwebide_core::scheduled::ScheduledTask;
#[derive(Clone, Copy)]
pub struct MonitorsState {
    pub entries: RwSignal<Vec<ScheduledTask>>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}
impl Default for MonitorsState {
    fn default() -> Self {
        Self::new()
    }
}
impl MonitorsState {
    pub fn new() -> Self {
        Self {
            entries: RwSignal::new(Vec::new()),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }
}
