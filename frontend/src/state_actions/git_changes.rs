//! Shared index and stash inspection for the Changes pane.
use crate::{
    project_git::ProjectGit,
    state::{auth::AuthState, git::GitState, projects::ProjectsState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::git::{GitStash, GitStashAction, GitStashRequest};

#[derive(Clone, Copy)]
pub struct GitChangesActions {
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub stashes: RwSignal<Vec<GitStash>>,
    pub refresh: Callback<()>,
}
impl GitChangesActions {
    pub fn new() -> Self {
        let repository = expect_context::<ProjectGit>();
        let projects = expect_context::<ProjectsState>();
        let auth = expect_context::<AuthState>();
        let git = expect_context::<GitState>();
        let loading = RwSignal::new(false);
        let error = RwSignal::new(None);
        let stashes = RwSignal::new(Vec::new());
        let revision = RwSignal::new(0_u64);
        let refresh = Callback::new(move |()| {
            revision.update(|revision| *revision += 1);
            let ticket = revision.get_untracked();
            let project = projects.active_project.get_untracked();
            let generation = auth.generation.get_untracked();
            let host = repository.revision();
            error.set(None);
            if project.is_none() {
                git.path_changes.set(None);
                stashes.set(Vec::new());
                loading.set(false);
                return;
            }
            loading.set(true);
            spawn_local(async move {
                let result = super::git_history::read_query(async {
                    let repo = repository.repository(project).await?;
                    let paths = repo.path_changes().await;
                    if projects.active_project.try_get_untracked() != Some(project)
                        || auth.generation.try_get_untracked() != Some(generation)
                        || repository.revision() != host
                        || revision.try_get_untracked() != Some(ticket)
                    {
                        return Err("Changes query superseded".into());
                    }
                    let saved = repo
                        .stash(&GitStashRequest {
                            action: GitStashAction::List,
                            hash: None,
                            message: None,
                        })
                        .await;
                    Ok::<_, String>((paths, saved))
                })
                .await;
                if projects.active_project.try_get_untracked() != Some(project)
                    || auth.generation.try_get_untracked() != Some(generation)
                    || repository.revision() != host
                    || revision.try_get_untracked() != Some(ticket)
                {
                    return;
                }
                loading.set(false);
                match result {
                    Ok((paths, saved)) => {
                        match paths {
                            Ok(paths) => {
                                if git.path_changes.get_untracked().as_ref() != Some(&paths) {
                                    git.path_changes.set(Some(paths));
                                }
                            }
                            Err(message) => {
                                error.set(Some(message));
                            }
                        }
                        match saved {
                            Ok(saved) => {
                                if stashes.get_untracked() != saved.stashes {
                                    stashes.set(saved.stashes);
                                }
                            }
                            Err(message) => {
                                error.set(Some(message));
                            }
                        }
                    }
                    Err(message) => {
                        error.set(Some(message));
                    }
                }
            });
        });
        let settings = expect_context::<crate::state::settings::SettingsState>();
        let boundary = StoredValue::new(None);
        Effect::new(move |_| {
            let identity = (
                projects.active_project.get(),
                auth.generation.get(),
                repository.revision(),
            );
            if boundary.get_value() != Some(identity) {
                boundary.set_value(Some(identity));
                git.path_changes.set(None);
                stashes.set(Vec::new());
            }
            projects.active_project.track();
            auth.generation.track();
            settings.bridge_url.track();
            projects.local_handles.track();
            git.status.track();
            git.changes_revision.track();
            refresh.run(());
        });
        Self {
            loading,
            error,
            stashes,
            refresh,
        }
    }
}
impl Default for GitChangesActions {
    fn default() -> Self {
        Self::new()
    }
}
