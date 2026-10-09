//! Shared catalog, installation and activation workflows with thin host transports.
use crate::{
    backend::Api,
    local_agent::BrowserBridgeClient,
    project_host::{ProjectExecution, ProjectHost},
    state::{
        auth::AuthState, chat::ChatState, plugins::PluginsState, projects::ProjectsState,
        settings::SettingsState,
    },
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::plugins::{marketplace::*, *};

enum PluginTransport {
    Remote(Api, i64),
    Local(BrowserBridgeClient),
}
impl PluginTransport {
    async fn prepare(&self, source: &PluginSource) -> Result<PreparedPlugin, String> {
        match self {
            Self::Remote(api, id) => {
                api.with_value(Clone::clone)
                    .prepare_plugin(*id, source)
                    .await
            }
            Self::Local(client) => client.prepare_plugin(source).await,
        }
    }
    async fn package(&self, expected: &PreparedPlugin) -> Result<PluginPackage, String> {
        match self {
            Self::Remote(api, id) => {
                api.with_value(Clone::clone)
                    .plugin_package(*id, expected)
                    .await
            }
            Self::Local(client) => client.plugin_package(expected).await,
        }
    }
}
#[derive(Clone, Debug)]
pub struct CatalogSelection {
    pub marketplace: MarketplaceSource,
    pub publisher: String,
    pub name: String,
    pub version: String,
}
#[derive(Clone)]
enum Operation {
    Refresh,
    Catalogs,
    Sources(Vec<MarketplaceSource>),
    Install(Option<CatalogSelection>),
    Enable(Box<PluginInstallation>),
    Disable(Box<ProjectPlugin>),
    Remove(Box<PluginInstallation>),
}
#[derive(Clone, Copy)]
pub struct ProjectPluginActions {
    pub refresh: Callback<()>,
    pub refresh_catalogs: Callback<()>,
    pub save_sources: Callback<Vec<MarketplaceSource>>,
    pub install: Callback<()>,
    pub install_release: Callback<CatalogSelection>,
    pub enable: Callback<PluginInstallation>,
    pub disable: Callback<ProjectPlugin>,
    pub remove: Callback<PluginInstallation>,
}
impl ProjectPluginActions {
    pub fn new(
        api: Api,
        state: PluginsState,
        host: ProjectHost,
        auth: AuthState,
        projects: ProjectsState,
        chat: ChatState,
        settings: SettingsState,
    ) -> Self {
        let generation = RwSignal::new(0_u64);
        let skill_actions = use_context::<crate::project_skills::ProjectSkillActions>();
        let scope = move || {
            (
                auth.generation.get_untracked(),
                projects.active_project.get_untracked(),
                chat.active_session.get_untracked(),
                settings.bridge_url.get_untracked(),
                host.revision(),
                projects
                    .active_project
                    .get_untracked()
                    .and_then(|id| projects.project(id))
                    .map(|p| (p.mode, p.path)),
                projects.local_handles.with_untracked(|h| {
                    projects
                        .active_project
                        .get_untracked()
                        .and_then(|id| h.get(&id).cloned())
                }),
            )
        };
        let perform = Callback::new(move |operation: Operation| {
            if state.busy.get_untracked() {
                return;
            }
            generation.update(|g| *g += 1);
            let ticket = generation.get_untracked();
            let identity = scope();
            let current = move || {
                generation.try_get_untracked() == Some(ticket)
                    && auth.generation.try_get_untracked().is_some()
                    && scope() == identity
            };
            let project = projects.active_project.get_untracked();
            state.busy.set(true);
            state.error.set(None);
            spawn_local(async move {
                let refresh_skills = matches!(
                    operation,
                    Operation::Enable(_) | Operation::Disable(_) | Operation::Remove(_)
                );
                let result =
                    run_operation(operation, api, state, host, project, current.clone()).await;
                if !current() {
                    return;
                }
                state.busy.set(false);
                if let Err(error) = result {
                    state.error.set(Some(error));
                } else if refresh_skills && let Some(actions) = skill_actions {
                    actions.refresh.run(());
                }
            });
        });
        let refresh = Callback::new(move |()| perform.run(Operation::Refresh));
        Effect::new(move |_| {
            auth.generation.track();
            projects.active_project.track();
            chat.active_session.track();
            settings.bridge_url.track();
            projects.projects.track();
            projects.local_handles.track();
            generation.update(|g| *g += 1);
            state.installations.set(Vec::new());
            state.project_plugins.set(Vec::new());
            state.marketplaces.set(MarketplaceSettings::default());
            state.failures.set(Vec::new());
            state.loaded.set(false);
            state.busy.set(false);
            state.error.set(None);
            state.repository.set(String::new());
            state.commit.set(String::new());
            state.path.set(".".into());
            state.marketplace_repository.set(String::new());
            state.marketplace_reference.set(String::new());
            state.marketplace_path.set("marketplace.json".into());
            state.search.set(String::new());
            if auth.user.get_untracked().is_some() {
                refresh.run(());
            }
        });
        Self {
            refresh,
            refresh_catalogs: Callback::new(move |()| perform.run(Operation::Catalogs)),
            save_sources: Callback::new(move |sources| perform.run(Operation::Sources(sources))),
            install: Callback::new(move |()| perform.run(Operation::Install(None))),
            install_release: Callback::new(move |release| {
                perform.run(Operation::Install(Some(release)));
            }),
            enable: Callback::new(move |entry| perform.run(Operation::Enable(Box::new(entry)))),
            disable: Callback::new(move |entry| perform.run(Operation::Disable(Box::new(entry)))),
            remove: Callback::new(move |entry| perform.run(Operation::Remove(Box::new(entry)))),
        }
    }
}

async fn run_operation(
    operation: Operation,
    api: Api,
    state: PluginsState,
    host: ProjectHost,
    project: Option<i64>,
    current: impl Fn() -> bool + Clone + 'static,
) -> Result<(), String> {
    let backend = api.with_value(Clone::clone);

    match operation {
        Operation::Refresh => {
            let entries = backend.plugin_installations().await?;
            if !current() {
                return Ok(());
            }
            state.installations.set(entries);
            let markets = backend.plugin_marketplaces().await?;
            if !current() {
                return Ok(());
            }
            state.marketplaces.set(markets);
            if let Some(id) = project {
                let entries = backend.project_plugins(id).await?;
                if !current() {
                    return Ok(());
                }
                state.project_plugins.set(entries);
            }
            state.loaded.set(true);
        }
        Operation::Catalogs => {
            let refreshed = backend.refresh_plugin_marketplaces().await?;
            if !current() {
                return Ok(());
            }
            state.marketplaces.set(refreshed.settings);
            state.failures.set(refreshed.failures);
        }
        Operation::Sources(sources) => {
            let request = SaveMarketplaces {
                revision: state.marketplaces.get_untracked().revision,
                sources,
            };
            let saved = backend.save_plugin_marketplaces(&request).await?;
            if !current() {
                return Ok(());
            }
            state.marketplaces.set(saved);
            state.failures.set(Vec::new());
        }
        Operation::Disable(entry) => {
            let id = project.ok_or("Open a project first.")?;
            let entries = backend
                .project_plugin_command(
                    id,
                    &ProjectPluginCommand::Disable {
                        id: entry.id,
                        revision: entry.revision,
                    },
                )
                .await?;
            if !current() {
                return Ok(());
            }
            state.project_plugins.set(entries);
        }
        Operation::Remove(entry) => {
            let entries = backend
                .remove_plugin(&RemovePlugin {
                    source: entry.prepared.source.clone(),
                    revision: entry.revision,
                })
                .await?;
            if !current() {
                return Ok(());
            }
            state.installations.set(entries);
            if let Some(id) = project {
                let entries = backend.project_plugins(id).await?;
                if current() {
                    state.project_plugins.set(entries);
                }
            }
        }
        Operation::Install(selection) => {
            let id = project.ok_or("Open a project before installing a plugin.")?;
            let (source, expected) = if let Some(selection) = selection {
                let markets = state.marketplaces.get_untracked();
                let catalog = markets
                    .catalogs
                    .iter()
                    .find(|c| c.source == selection.marketplace)
                    .ok_or("Refresh this marketplace first.")?;
                let source = catalog
                    .catalog
                    .resolve(
                        &catalog.source,
                        &selection.publisher,
                        &selection.name,
                        &selection.version,
                    )
                    .map_err(|e| e.to_string())?;
                (source, Some(selection))
            } else {
                (
                    PluginSource {
                        repository: state.repository.get_untracked().trim().into(),
                        commit: state.commit.get_untracked().trim().into(),
                        path: state.path.get_untracked().trim().into(),
                    },
                    None,
                )
            };
            source.validate().map_err(|e| e.to_string())?;
            let revision = state.installations.with_untracked(|entries| {
                entries
                    .iter()
                    .find(|e| {
                        e.prepared.source.repository == source.repository
                            && e.prepared.source.path == source.path
                    })
                    .map(|e| e.revision)
            });
            let transport = match host
                .resolve_guarded(Some(id), true, current.clone())
                .await?
            {
                ProjectExecution::Remote { api, .. } => PluginTransport::Remote(api, id),
                ProjectExecution::Local(client) => PluginTransport::Local(client),
            };
            if !current() {
                return Ok(());
            }
            let prepared = transport.prepare(&source).await?;
            prepared.validate().map_err(|e| e.to_string())?;
            if prepared.source != source {
                return Err("Plugin host returned a different source.".into());
            }
            if expected.is_some_and(|e| {
                e.publisher != prepared.manifest.publisher
                    || e.name != prepared.manifest.name
                    || e.version != prepared.manifest.version
            }) {
                return Err(
                    "Package identity or version does not match its marketplace release.".into(),
                );
            }
            if !current() {
                return Ok(());
            }
            let entries = backend
                .record_plugin(&RecordPlugin { prepared, revision })
                .await?;
            if current() {
                state.installations.set(entries);
            }
        }
        Operation::Enable(entry) => {
            let id = project.ok_or("Open a project before enabling a plugin.")?;
            let revision = state.project_plugins.with_untracked(|entries| {
                entries
                    .iter()
                    .find(|e| {
                        e.prepared.source.repository == entry.prepared.source.repository
                            && e.prepared.source.path == entry.prepared.source.path
                    })
                    .map(|e| e.revision)
            });
            let transport = match host
                .resolve_guarded(Some(id), true, current.clone())
                .await?
            {
                ProjectExecution::Remote { api, .. } => PluginTransport::Remote(api, id),
                ProjectExecution::Local(client) => PluginTransport::Local(client),
            };
            if !current() {
                return Ok(());
            }
            let package = transport.package(&entry.prepared).await?;
            package.validate().map_err(|e| e.to_string())?;
            if package.prepared.source != entry.prepared.source
                || package.prepared.manifest != entry.prepared.manifest
                || package.prepared.digest != entry.prepared.digest
            {
                return Err("Plugin host returned different package content.".into());
            }
            if !current() {
                return Ok(());
            }
            let entries = backend
                .record_plugin(&RecordPlugin {
                    prepared: package.prepared.clone(),
                    revision: Some(entry.revision),
                })
                .await?;
            if !current() {
                return Ok(());
            }
            let installed = entries
                .iter()
                .find(|e| e.prepared.source == package.prepared.source)
                .ok_or("Installation was not recorded.")?;
            let command = ProjectPluginCommand::Enable {
                package: Box::new(package),
                installation_revision: installed.revision,
                revision,
            };
            state.installations.set(entries);
            let bindings = backend.project_plugin_command(id, &command).await?;
            if current() {
                state.project_plugins.set(bindings);
            }
        }
    }
    Ok(())
}
