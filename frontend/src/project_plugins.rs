//! Shared install workflow; only transport selection differs between project modes.
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
use openwebide_core::plugins::{PluginSource, PreparedPlugin, RecordPlugin};

enum PluginTransport {
    Remote(Api, i64),
    Local(BrowserBridgeClient),
}
impl PluginTransport {
    async fn prepare(&self, source: &PluginSource) -> Result<PreparedPlugin, String> {
        match self {
            Self::Remote(api, project) => {
                api.with_value(Clone::clone)
                    .prepare_plugin(*project, source)
                    .await
            }
            Self::Local(client) => client.prepare_plugin(source).await,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ProjectPluginActions {
    pub refresh: Callback<()>,
    pub install: Callback<()>,
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
                    .map(|project| (project.mode, project.path)),
                projects.local_handles.with_untracked(|handles| {
                    projects
                        .active_project
                        .get_untracked()
                        .and_then(|id| handles.get(&id).cloned())
                }),
            )
        };
        let refresh = Callback::new(move |()| {
            if state.busy.get_untracked() {
                return;
            }
            generation.update(|value| *value += 1);
            let ticket = generation.get_untracked();
            let identity = scope();
            state.busy.set(true);
            state.error.set(None);
            spawn_local(async move {
                let backend = api.with_value(Clone::clone);
                let result = backend.plugin_installations().await;
                if generation.try_get_untracked() != Some(ticket)
                    || auth.generation.try_get_untracked().is_none()
                    || scope() != identity
                {
                    return;
                }
                state.busy.set(false);
                match result {
                    Ok(entries) => state.installations.set(entries),
                    Err(error) => state.error.set(Some(error)),
                }
            });
        });
        let install = Callback::new(move |()| {
            if state.busy.get_untracked() {
                return;
            }
            let Some(project) = projects.active_project.get_untracked() else {
                state
                    .error
                    .set(Some("Open a project before installing a plugin.".into()));
                return;
            };
            let source = PluginSource {
                repository: state.repository.get_untracked().trim().into(),
                commit: state.commit.get_untracked().trim().into(),
                path: state.path.get_untracked().trim().into(),
            };
            if let Err(error) = source.validate() {
                state.error.set(Some(error.to_string()));
                return;
            }
            generation.update(|value| *value += 1);
            let ticket = generation.get_untracked();
            let identity = scope();
            let current = move || {
                generation.try_get_untracked() == Some(ticket)
                    && auth.generation.try_get_untracked().is_some()
                    && scope() == identity
            };
            let revision = state.installations.with_untracked(|entries| {
                entries
                    .iter()
                    .find(|entry| {
                        entry.prepared.source.repository == source.repository
                            && entry.prepared.source.path == source.path
                    })
                    .map(|entry| entry.revision)
            });
            state.busy.set(true);
            state.error.set(None);
            spawn_local(async move {
                let result = async {
                    // Adapter selection is the only project-mode boundary.
                    let transport = match host
                        .resolve_guarded(Some(project), true, current.clone())
                        .await?
                    {
                        ProjectExecution::Remote { api, .. } => {
                            PluginTransport::Remote(api, project)
                        }
                        ProjectExecution::Local(client) => PluginTransport::Local(client),
                    };
                    if !current() {
                        return Err("Project access changed".into());
                    }
                    let prepared = transport.prepare(&source).await?;
                    prepared.validate().map_err(|error| error.to_string())?;
                    if prepared.source != source {
                        return Err("The plugin host returned a different package source.".into());
                    }
                    if !current() {
                        return Err("Project access changed".into());
                    }
                    let backend = api.with_value(Clone::clone);
                    backend
                        .record_plugin(&RecordPlugin { prepared, revision })
                        .await
                }
                .await;
                if !current() {
                    return;
                }
                state.busy.set(false);
                match result {
                    Ok(entries) => state.installations.set(entries),
                    Err(error) => state.error.set(Some(error)),
                }
            });
        });
        Effect::new(move |_| {
            auth.generation.track();
            projects.active_project.track();
            chat.active_session.track();
            settings.bridge_url.track();
            projects.projects.track();
            projects.local_handles.track();
            generation.update(|value| *value += 1);
            state.installations.set(Vec::new());
            state.error.set(None);
            state.busy.set(false);
            state.repository.set(String::new());
            state.commit.set(String::new());
            state.path.set(".".into());
            if auth.user.get_untracked().is_some() {
                refresh.run(());
            }
        });
        Self { refresh, install }
    }
}
