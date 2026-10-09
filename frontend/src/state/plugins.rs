use leptos::prelude::*;
use openwebide_core::plugins::PluginInstallation;

#[derive(Clone, Copy)]
pub struct PluginsState {
    pub installations: RwSignal<Vec<PluginInstallation>>,
    pub repository: RwSignal<String>,
    pub commit: RwSignal<String>,
    pub path: RwSignal<String>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}
impl Default for PluginsState {
    fn default() -> Self {
        Self {
            installations: RwSignal::new(Vec::new()),
            repository: RwSignal::new(String::new()),
            commit: RwSignal::new(String::new()),
            path: RwSignal::new(".".into()),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }
}
