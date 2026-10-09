//! Draft Git descriptions from shared repository and workspace facades.
use crate::{
    backend::Api,
    project_git::ProjectGit,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState, settings::SettingsState},
    workspace::Workspace,
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::{AssistanceKind, AssistanceRequest, GitFileStatus};

#[derive(Clone, Copy)]
pub struct GitAssistance {
    pub draft: RwSignal<String>,
    pub kind: RwSignal<Option<AssistanceKind>>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub note: RwSignal<Option<String>>,
    pub generate: Callback<AssistanceKind>,
    pub generate_staged: Callback<()>,
    selection: Signal<Option<openwebide_core::ModelSelection>>,
}
impl GitAssistance {
    pub fn new(
        api: Api,
        repository: ProjectGit,
        projects: ProjectsState,
        auth: AuthState,
        chat: ChatState,
        settings: SettingsState,
    ) -> Self {
        let git = expect_context::<crate::state::git::GitState>();
        let connection = Signal::derive(move || {
            let project = projects.active_project.get();
            let session = chat.active_session.get();
            chat.sessions
                .with(|sessions| {
                    sessions
                        .iter()
                        .find(|entry| Some(entry.id) == session && entry.project_id == project)
                        .and_then(|entry| entry.connection_id)
                })
                .or(settings.default_connection.get())
                .or_else(|| {
                    settings
                        .model_setup
                        .with(|setup| setup.defaults.primary.as_ref().map(|model| model.server_id))
                })
        });
        let selection = Signal::derive(move || {
            let server_id = connection.get()?;
            let model = chat
                .active_session
                .get()
                .and_then(|id| {
                    chat.session_model
                        .with(|models| models.get(&id).cloned().flatten())
                })
                .or(chat.selected_model.get())
                .or_else(|| {
                    settings.model_setup.with(|setup| {
                        setup
                            .defaults
                            .primary
                            .as_ref()
                            .filter(|model| model.server_id == server_id)
                            .map(|model| model.model.clone())
                    })
                })
                .or_else(|| {
                    settings.connections.with(|connections| {
                        connections
                            .iter()
                            .find(|connection| connection.id == server_id)
                            .and_then(|connection| connection.model.clone())
                    })
                })?;
            Some(openwebide_core::ModelSelection { server_id, model })
        });
        let ui = expect_context::<crate::state::ui::UiState>();
        let note = RwSignal::new(None);
        let draft = RwSignal::new(String::new());
        let kind = RwSignal::new(None);
        let busy = RwSignal::new(false);
        let error = RwSignal::new(None);
        let revision = StoredValue::new(0u64);
        let queue = StoredValue::new_local(super::assistance::AssistanceQueue::from_context());
        Effect::new(move |_| {
            auth.generation.track();
            projects.active_project.track();
            chat.active_session.track();
            selection.track();
            settings.model_setup.track();
            revision.update_value(|revision| *revision += 1);
            draft.set(String::new());
            kind.set(None);
            busy.set(false);
            error.set(None);
            note.set(None);
        });
        let generate_request = Callback::new(move |(requested, staged): (AssistanceKind, bool)| {
            if busy.get_untracked()
                || chat.streaming.get_untracked()
                || git.commit_busy.get_untracked()
                || git.branch_busy.get_untracked()
                || git.sync_busy.get_untracked().is_some()
            {
                return;
            }
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            let selected_model = selection.get_untracked();
            let session = chat.active_session.get_untracked();
            let connection_id = selected_model.as_ref().map(|model| model.server_id);
            let Some(connection_id) = connection_id else {
                ui.notify("Choose a model in Settings first.");
                error.set(Some("Choose a model in Settings first.".into()));
                return;
            };
            let epoch = auth.generation.get_untracked();
            let host_revision = repository.revision();
            revision.update_value(|revision| *revision += 1);
            let ticket = revision.get_value();
            let original = draft.get_untracked();
            let original_message = git.commit_message.get_untracked();
            busy.set(true);
            error.set(None);
            spawn_local(async move {
                let current = || {
                    auth.generation.try_get_untracked() == Some(epoch)
                        && projects.active_project.try_get_untracked() == Some(Some(project))
                        && chat.active_session.try_get_untracked() == Some(session)
                        && selection.try_get_untracked() == Some(selected_model.clone())
                        && repository.revision() == host_revision
                        && revision.try_get_value() == Some(ticket)
                        && chat.streaming.try_get_untracked() == Some(false)
                };
                let result: Result<String, String> = async {
                    if !current() { return Err("Draft request superseded.".into()); }
                    let Some(queue) = queue.try_with_value(Clone::clone) else { return Err("Draft request superseded.".into()); };
                    let _permit = queue.permit().await;
                    if !current() || chat.streaming.get_untracked() { return Err("Draft request superseded.".into()); }
                    let repo = repository.repository(Some(project)).await?;
                    let status = repo.status().await?;
                    let tracked_diff = if staged {repo.index_diff().await?} else {repo.diff(None).await?};
                    let mut changes = tracked_diff.clone();
                    let mut new_files = Vec::new();
                    let workspace = Workspace::for_project(api, projects, project).ok_or("Reconnect the project folder first.")?;
                    for (path, status) in status.files.iter().filter(|(_, status)| !staged && **status == GitFileStatus::Untracked).take(8) {
                        let _ = status;
                        if let Ok(content) = workspace.read(path).await {
                            new_files.push((path.clone(), fingerprint(&content)));
                            changes.push_str(&format!("\nNew file {path}:\n{}\n", content.chars().take(1500).collect::<String>()));
                        }
                    }
                    if changes.trim().is_empty() { return Err(if staged {"No staged changes to describe. Stage files and try again."}else{"There are no changes to describe."}.into()); }
                    let guidance = workspace.read("AGENTS.md").await.unwrap_or_default();
                    let input = format!("Repository formatting conventions:\n{}\nCurrent branch: {}\nActual changes:\n{}", guidance.chars().take(6000).collect::<String>(), status.branch, if staged {changes.clone()}else{changes.chars().take(16000).collect::<String>()});
                    if !current() || chat.streaming.get_untracked() { return Err("Draft request superseded.".into()); }
                    let model = selected_model.as_ref().map(|model|model.model.clone());
                    let request = AssistanceRequest { kind: requested, connection_id, model, staged_draft: staged, project_id: Some(project), session_id: session, input };
                    request.validate()?;
                    let generated = if staged {
                        let result=api.with_value(Clone::clone).staged_assistance(&request).await?;
                        if !current() {return Err("Draft request superseded.".into());}
                        note.set(Some(format!("{} · {}", request.model.as_deref().unwrap_or("Primary model"), result.limits_note())));
                        result.text
                    } else {api.with_value(Clone::clone).assistance(&request).await?.ok_or("The model could not produce a draft. Try again.")?};
                    // A diff changed during generation must not produce a misleading draft.
                    let fresh_status = repo.status().await?;
                    let fresh_diff=if staged {repo.index_diff().await?} else {repo.diff(None).await?};
                    if fresh_diff != tracked_diff || (!staged && fresh_status.files != status.files) || fresh_status.branch != status.branch { return Err("Changes moved while drafting. Try again.".into()); }
                    for (path, content) in new_files {
                        if fingerprint(&workspace.read(&path).await.map_err(|error| error.to_string())?) != content {
                            return Err("Files changed while drafting. Try again.".into());
                        }
                    }
                    if workspace.read("AGENTS.md").await.unwrap_or_default() != guidance { return Err("Repository conventions changed while drafting. Try again.".into()); }
                    Ok(generated)
                }.await;
                if !current() {
                    if revision.try_get_value() == Some(ticket) {
                        busy.set(false);
                    }
                    return;
                }
                busy.set(false);
                match result {
                    Ok(text) if draft.get_untracked() == original => {
                        if staged && git.commit_message.get_untracked() == original_message {
                            git.commit_message.set(text.clone());
                        }
                        draft.set(text);
                        kind.set(Some(requested));
                    }
                    Ok(_) => error.set(Some(
                        "Kept your edited draft. Generate again when ready.".into(),
                    )),
                    Err(message) => {
                        ui.notify(message.clone());
                        error.set(Some(message));
                    }
                }
            });
        });
        let generate = Callback::new(move |requested| generate_request.run((requested, false)));
        let generate_staged =
            Callback::new(move |()| generate_request.run((AssistanceKind::Commit, true)));
        Self {
            draft,
            kind,
            busy,
            error,
            note,
            generate,
            generate_staged,
            selection,
        }
    }
    /// Draft once after staging settles, only while Changes is visible and the message is empty.
    pub fn auto_commit(self, visible: Signal<bool>) {
        let git = expect_context::<crate::state::git::GitState>();
        let projects = expect_context::<ProjectsState>();
        let auth = expect_context::<AuthState>();
        let settings = expect_context::<SettingsState>();
        let chat = expect_context::<ChatState>();
        let pending = StoredValue::new(None);
        let host = expect_context::<ProjectGit>();
        Effect::new(move |_| {
            settings.bridge_url.track();
            projects.local_handles.track();
            let selection = self.selection.get();
            let staged = git
                .path_changes
                .with(|changes| changes.as_ref().map(|changes| changes.staged.clone()))
                .unwrap_or_default();
            if !visible.get()
                || selection.is_none()
                || staged.is_empty()
                || !git.commit_message.with(String::is_empty)
                || self.busy.get()
                || chat.streaming.get()
            {
                return;
            }
            let identity = (
                projects.active_project.get(),
                auth.generation.get(),
                git.changes_revision.get(),
                staged,
                selection,
                host.revision(),
                chat.active_session.get(),
            );
            if pending.get_value().as_ref() == Some(&identity) {
                return;
            }
            pending.set_value(Some(identity.clone()));
            spawn_local(async move {
                crate::util::sleep_ms(1000).await;
                if pending.try_get_value().flatten().as_ref() == Some(&identity)
                    && visible.try_get_untracked() == Some(true)
                    && projects.active_project.try_get_untracked() == Some(identity.0)
                    && auth.generation.try_get_untracked() == Some(identity.1)
                    && git.changes_revision.try_get_untracked() == Some(identity.2)
                    && git.path_changes.try_with_untracked(|changes| {
                        changes.as_ref().map(|changes| changes.staged.clone())
                    }) == Some(Some(identity.3.clone()))
                    && host.revision() == identity.5
                    && self.selection.try_get_untracked() == Some(identity.4)
                    && chat.active_session.try_get_untracked() == Some(identity.6)
                    && chat.streaming.try_get_untracked() == Some(false)
                    && git.commit_message.try_with_untracked(String::is_empty) == Some(true)
                    && self.busy.try_get_untracked() == Some(false)
                {
                    self.generate_staged.run(());
                }
            });
        });
    }
}

fn fingerprint(content: &str) -> u64 {
    use std::hash::Hasher;
    let mut hash = std::hash::DefaultHasher::new();
    hash.write(content.as_bytes());
    hash.finish()
}
