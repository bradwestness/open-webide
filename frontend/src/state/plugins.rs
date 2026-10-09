use leptos::prelude::*;
use openwebide_core::plugins::{
    PluginInstallation, ProjectPlugin,
    marketplace::{MarketplaceFailure, MarketplaceSettings},
};

#[derive(Clone, Copy)]
pub struct PluginsState {
    pub installations: RwSignal<Vec<PluginInstallation>>,
    pub project_plugins: RwSignal<Vec<ProjectPlugin>>,
    pub marketplaces: RwSignal<MarketplaceSettings>,
    pub failures: RwSignal<Vec<MarketplaceFailure>>,
    pub marketplace_repository: RwSignal<String>,
    pub marketplace_reference: RwSignal<String>,
    pub marketplace_path: RwSignal<String>,
    pub search: RwSignal<String>,
    pub repository: RwSignal<String>,
    pub commit: RwSignal<String>,
    pub path: RwSignal<String>,
    pub busy: RwSignal<bool>,
    pub loaded: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}
impl Default for PluginsState {
    fn default() -> Self {
        Self {
            installations: RwSignal::new(Vec::new()),
            project_plugins: RwSignal::new(Vec::new()),
            marketplaces: RwSignal::new(MarketplaceSettings::default()),
            failures: RwSignal::new(Vec::new()),
            marketplace_repository: RwSignal::new(String::new()),
            marketplace_reference: RwSignal::new(String::new()),
            marketplace_path: RwSignal::new("marketplace.json".into()),
            search: RwSignal::new(String::new()),
            repository: RwSignal::new(String::new()),
            commit: RwSignal::new(String::new()),
            path: RwSignal::new(".".into()),
            busy: RwSignal::new(false),
            loaded: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }
}
