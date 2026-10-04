//! Run review facade. Filesystem operations go through Workspace in both modes.
use crate::{
    backend::Api,
    state::{
        auth::AuthState,
        chat::ChatState,
        projects::ProjectsState,
        reviews::ReviewsState,
        ui::{ConfirmRequest, UiState},
        workspace::WorkspaceState,
    },
    workspace::Workspace,
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::{EditDecision, ReviewRequest};
#[derive(Clone, Copy)]
pub struct ReviewActions {
    pub review: Callback<ReviewRequest>,
    pub for_file: Callback<(String, EditDecision), bool>,
    pub refresh: Callback<()>,
    pub open: Callback<String>,
}
impl ReviewActions {
    pub fn new(
        api: Api,
        projects: ProjectsState,
        workspace: WorkspaceState,
        ui: UiState,
        open: Callback<String>,
        refresh_git: Callback<()>,
    ) -> Self {
        let auth = expect_context::<AuthState>();
        let chat = expect_context::<ChatState>();
        let state = use_context::<ReviewsState>().unwrap_or_else(|| {
            let state = ReviewsState::new();
            provide_context(state);
            state
        });
        let refresh = Callback::new(move |()| {
            let project = projects.active_project.get_untracked();
            let auth_generation = auth.generation.get_untracked();
            state.generation.update_value(|value| *value += 1);
            let generation = state.generation.get_value();
            if project.is_none() {
                state.records.set(vec![]);
                return;
            }
            spawn_local(async move {
                let project = project.unwrap();
                let result = api.with_value(Clone::clone).list_run_changes(project).await;
                if auth.generation.try_get_untracked() != Some(auth_generation)
                    || projects.active_project.try_get_untracked() != Some(Some(project))
                    || state.generation.try_get_value() != Some(generation)
                {
                    return;
                }
                match result {
                    Ok(records) => {
                        let has_reviews = !records.is_empty();
                        state.records.set(records);
                        if has_reviews {
                            super::workspace::refresh_pending(
                                api, projects, workspace, ui, auth, project,
                            )
                            .await;
                        }
                    }
                    Err(error) => ui.notify(error),
                }
            });
        });
        let last_project = StoredValue::new(None);
        let last_auth = StoredValue::new(auth.generation.get_untracked());
        Effect::new(move |_| {
            let project = projects.active_project.get();
            let generation = auth.generation.get();
            let _ = chat.finished_tools.get();
            let _ = chat.streaming.get();
            if last_project.get_value() != project || last_auth.get_value() != generation {
                state.records.set(vec![]);
                if last_auth.get_value() != generation {
                    state.busy.set(None);
                }
                last_project.set_value(project);
                last_auth.set_value(generation);
            }
            refresh.run(());
        });
        let perform = Callback::new(move |request: ReviewRequest| {
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            if chat.streaming.get_untracked()
                || chat.rewinding.get_untracked()
                || state.busy.get_untracked().is_some()
            {
                return;
            }
            if workspace.dirty.get_untracked() {
                ui.notify("Save or discard the editor's unsaved changes before reviewing.");
                return;
            }
            let Some(files) = Workspace::for_project(api, projects, project) else {
                ui.notify("Grant folder access before reviewing changes.");
                return;
            };
            let auth_generation = auth.generation.get_untracked();
            let active_session = chat.active_session.get_untracked();
            let epoch = workspace.pending_epoch.get_untracked();
            let current = move || {
                auth.generation.try_get_untracked() == Some(auth_generation)
                    && projects.active_project.try_get_untracked() == Some(Some(project))
                    && chat.active_session.try_get_untracked() == Some(active_session)
                    && workspace.pending_epoch.try_get_untracked() == Some(epoch)
            };
            state.busy.set(Some((project, request.clone())));
            let key = (project, request.path.clone());
            workspace.resolving_edits.update(|keys| {
                keys.insert(key.clone());
            });
            spawn_local(async move {
                let result = async {
                    let backend = api.with_value(Clone::clone);
                    // Preflight before taking the durable project lock.
                    let preview = backend.preview_run_review(project, &request).await?;
                    openwebide_core::rewind::verify_files(&files, &preview.restore, current)
                        .await?;
                    if !current() {
                        return Err(
                            "Return to this project and session to resume the review".into()
                        );
                    }
                    let plan = backend.prepare_run_review(project, &request).await?;
                    openwebide_core::rewind::restore_files(&files, &plan.restore, current).await?;
                    if !current() {
                        return Err(
                            "Return to this project and session to resume the review".into()
                        );
                    }
                    let reviewed = backend.complete_run_review(project, &request).await?;
                    if current() {
                        // Keep the editor read-only until its restored content is installed.
                        if workspace.open_file.get_untracked().as_deref() == Some(&request.path) {
                            match reviewed.current_bytes()? {
                                None => {
                                    workspace.open_file.set(None);
                                    workspace.content.set(String::new());
                                }
                                Some(bytes) => {
                                    workspace
                                        .content
                                        .set(String::from_utf8_lossy(&bytes).into_owned());
                                    workspace.dirty.set(false);
                                }
                            }
                        }
                        super::workspace::refresh_pending(
                            api, projects, workspace, ui, auth, project,
                        )
                        .await;
                        refresh_git.run(());
                        refresh.run(());
                    }
                    Ok::<_, String>(())
                }
                .await;
                if auth.generation.try_get_untracked() == Some(auth_generation) {
                    if workspace.pending_epoch.try_get_untracked() == Some(epoch) {
                        workspace.resolving_edits.update(|keys| {
                            keys.remove(&key);
                        });
                    }
                    if state.busy.get_untracked() == Some((project, request.clone())) {
                        state.busy.set(None);
                    }
                    if current()
                        && let Err(error) = result
                    {
                        ui.notify(format!(
                            "Review paused: {error}. Retry the same review to resume."
                        ));
                        refresh.run(());
                    }
                }
            });
        });
        let review = Callback::new(move |request: ReviewRequest| {
            let project = projects.active_project.get_untracked();
            let generation = auth.generation.get_untracked();
            let session = chat.active_session.get_untracked();
            if request.decision == EditDecision::Rejected {
                ui.set_confirm(ConfirmRequest { title: if request.hunk.is_some() { "Reject hunk?" } else { "Reject file changes?" }.into(), message: "Restore the selected changes to their previous contents. Later manual edits will be preserved.".into(), confirm_label: "Reject".into(), action: Callback::new(move |()| {
                    if projects.active_project.get_untracked() == project && auth.generation.get_untracked() == generation && chat.active_session.get_untracked() == session { perform.run(request.clone()); }
                }) });
            } else {
                perform.run(request);
            }
        });
        let for_file = Callback::new(move |(path, decision): (String, EditDecision)| {
            let record = state.records.with_untracked(|records| {
                records
                    .iter()
                    .filter(|record| {
                        record.file.path == path && record.available && record.pending() > 0
                    })
                    .max_by_key(|record| record.source_step)
                    .cloned()
            });
            let Some(record) = record else {
                return false;
            };
            review.run(ReviewRequest {
                session_id: record.session_id,
                message_id: record.message_id,
                path,
                revision: record.revision,
                decision,
                hunk: None,
            });
            true
        });
        Self {
            review,
            for_file,
            refresh,
            open,
        }
    }
}
