use leptos::prelude::*;
use openwebide_core::scheduled::ScheduledTask;
#[derive(Clone, Copy)]
pub struct TasksState {
    pub auto_title: RwSignal<bool>,
    pub entries: RwSignal<Vec<ScheduledTask>>,
    pub busy: RwSignal<bool>,
    pub loading: RwSignal<bool>,
    pub loaded: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub editing: RwSignal<bool>,
    pub edit_id: RwSignal<Option<(i64, i64)>>,
    pub title: RwSignal<String>,
    pub model: RwSignal<Option<openwebide_core::ModelSelection>>,
    pub prompt: RwSignal<String>,
    pub session: RwSignal<i64>,
    pub session_target: RwSignal<openwebide_core::scheduled::SessionTarget>,
    pub kind: RwSignal<String>,
    pub date: RwSignal<String>,
    pub time: RwSignal<String>,
    pub days: RwSignal<Vec<u8>>,
    pub cron: RwSignal<String>,
    pub timezone: RwSignal<String>,
    pub enabled: RwSignal<bool>,
}
impl TasksState {
    pub fn new() -> Self {
        Self {
            auto_title: RwSignal::new(true),
            entries: RwSignal::new(Vec::new()),
            busy: RwSignal::new(false),
            loading: RwSignal::new(false),
            loaded: RwSignal::new(false),
            error: RwSignal::new(None),
            editing: RwSignal::new(false),
            edit_id: RwSignal::new(None),
            title: RwSignal::new(String::new()),
            model: RwSignal::new(None),
            prompt: RwSignal::new(String::new()),
            session: RwSignal::new(0),
            session_target: RwSignal::new(openwebide_core::scheduled::SessionTarget::Latest),
            kind: RwSignal::new("weekly".into()),
            date: RwSignal::new(String::new()),
            time: RwSignal::new("09:00".into()),
            days: RwSignal::new(vec![1, 2, 3, 4, 5]),
            cron: RwSignal::new("0 9 * * 1-5".into()),
            timezone: RwSignal::new(
                crate::browser_preferences::capture()
                    .and_then(|value| value.timezone)
                    .unwrap_or_else(|| "UTC".into()),
            ),
            enabled: RwSignal::new(true),
        }
    }
}

impl Default for TasksState {
    fn default() -> Self {
        Self::new()
    }
}
