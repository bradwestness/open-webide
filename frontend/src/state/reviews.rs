use leptos::prelude::*;
use openwebide_core::{ReviewRequest, RunChange};
#[derive(Clone, Copy)]
pub struct ReviewsState {
    pub records: RwSignal<Vec<RunChange>>,
    pub busy: RwSignal<Option<(i64, ReviewRequest)>>,
    pub generation: StoredValue<u64>,
}
impl ReviewsState {
    pub fn new() -> Self {
        Self {
            records: RwSignal::new(vec![]),
            busy: RwSignal::new(None),
            generation: StoredValue::new(0),
        }
    }
}

impl Default for ReviewsState {
    fn default() -> Self {
        Self::new()
    }
}
