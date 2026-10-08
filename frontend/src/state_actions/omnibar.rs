//! Universal search orchestration; Workspace supplies thin filesystem adapters.
use super::{commands::CommandActions, navigation::NavigationActions};
use crate::{
    backend::Api,
    omnibar::{OmnibarAction, OmnibarEntry},
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState, ui::UiState},
    workspace::Workspace,
};
use futures::future::{AbortHandle, abortable};
use leptos::{prelude::*, task::spawn_local};

type OmnibarScope = (u64, (u64, Option<i64>, Option<i64>));

#[derive(Clone, Copy)]
pub struct OmnibarActions {
    pub query: RwSignal<String>,
    pub entries: Memo<Vec<OmnibarEntry>>,
    pub loading: RwSignal<bool>,
    pub notice: RwSignal<Option<String>>,
    pub activate: Callback<OmnibarAction>,
    pub scope: Memo<OmnibarScope>,
}
pub struct OmnibarContext {
    pub api: Api,
    pub projects: ProjectsState,
    pub chat: ChatState,
    pub ui: UiState,
    pub commands: CommandActions,
    pub navigation: NavigationActions,
    pub open_file: Callback<String>,
}
impl OmnibarActions {
    pub fn new(context: OmnibarContext) -> Self {
        let OmnibarContext {
            api,
            projects,
            chat,
            ui,
            commands,
            navigation,
            open_file,
        } = context;
        let auth = expect_context::<AuthState>();
        let root = super::workspace::project_epoch(projects, auth);
        let scope = Memo::new(move |_| (root.get(), commands.scope.get()));
        let opened_scope = StoredValue::new(None);
        let query = RwSignal::new(String::new());
        let files = RwSignal::new(Vec::<String>::new());
        let sessions = RwSignal::new(Vec::<openwebide_core::ChatSession>::new());
        let loading = RwSignal::new(false);
        let notice = RwSignal::new(None::<String>);
        let request = StoredValue::new(None::<AbortHandle>);
        on_cleanup(move || {
            if let Some(Some(request)) = request.try_get_value() {
                request.abort();
            }
        });
        Effect::new(move |_| {
            let open = ui.palette_open.get();
            let token = scope.get();
            if let Some(previous) = request.try_update_value(Option::take).flatten() {
                previous.abort();
            }
            files.set(Vec::new());
            notice.set(None);
            loading.set(false);
            if !open {
                query.set(String::new());
                opened_scope.set_value(None);
                return;
            }
            if opened_scope
                .get_value()
                .is_some_and(|previous| previous != token)
            {
                opened_scope.set_value(None);
                ui.palette_open.set(false);
                return;
            }
            opened_scope.set_value(Some(token));
            sessions.set(chat.sessions.get_untracked());
            loading.set(true);
            let current = move || {
                ui.palette_open.try_get_untracked() == Some(true)
                    && scope.try_get_untracked() == Some(token)
            };
            let workspace = projects
                .active_project
                .get_untracked()
                .and_then(|id| Workspace::for_project(api, projects, id));
            let has_project = projects.active_project.get_untracked().is_some();
            let (future, handle) = abortable(async move {
                let file_request = async {
                    if let Some(workspace) = workspace {
                        workspace
                            .discover_files(current)
                            .await
                            .map_err(|error| error.to_string())
                    } else if has_project {
                        Err(
                            "Grant folder access in Files to search this project's filenames"
                                .into(),
                        )
                    } else {
                        Ok(Default::default())
                    }
                };
                let backend = api.with_value(Clone::clone);
                let (discovery, all_sessions) =
                    futures::join!(file_request, backend.list_sessions());
                if !current() {
                    return;
                }
                let mut messages = Vec::new();
                match discovery {
                    Ok(discovery) => {
                        files.set(discovery.paths);
                        if discovery.truncated {
                            messages.push("Filename index reached its limit; use Files search for additional results".to_string());
                        }
                    }
                    Err(error) => messages.push(format!("Filename search: {error}")),
                }
                match all_sessions {
                    Ok(found) => {
                        sessions.set(found.clone());
                    }
                    Err(error) => messages.push(format!("Session search: {error}")),
                }
                notice.set((!messages.is_empty()).then(|| messages.join(" · ")));
                loading.set(false);
            });
            request.set_value(Some(handle));
            spawn_local(async move {
                let _ = future.await;
            });
        });
        let entries = Memo::new(move |_| {
            crate::omnibar::search(
                &query.get(),
                &files.get(),
                &projects.projects.get(),
                &sessions.get(),
                commands.context.get(),
            )
        });
        let activate = Callback::new(move |action: OmnibarAction| {
            if !ui.palette_open.get_untracked()
                || opened_scope.get_value() != Some(scope.get_untracked())
            {
                return;
            }
            if let OmnibarAction::Command(command) = action {
                commands.run.run(command);
                return;
            }
            ui.palette_open.set(false);
            match action {
                OmnibarAction::File(path) => open_file.run(path),
                OmnibarAction::Project(id) => navigation.open_project.run(id),
                OmnibarAction::Session(id) => {
                    if let Some(session) = sessions.with_untracked(|sessions| {
                        sessions.iter().find(|session| session.id == id).cloned()
                    }) {
                        chat.sessions.update(|cached| {
                            if !cached.iter().any(|cached| cached.id == id) {
                                cached.push(session);
                            }
                        });
                        navigation.open_session.run(id);
                    } else {
                        ui.notify("This session is no longer available");
                    }
                }
                OmnibarAction::Command(_) => {}
            }
        });
        Self {
            query,
            entries,
            loading,
            notice,
            activate,
            scope,
        }
    }
}
