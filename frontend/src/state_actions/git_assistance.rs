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
    pub generate: Callback<AssistanceKind>,
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
        let draft = RwSignal::new(String::new());
        let kind = RwSignal::new(None);
        let busy = RwSignal::new(false);
        let error = RwSignal::new(None);
        let revision = StoredValue::new(0u64);
        let queue = StoredValue::new_local(super::assistance::AssistanceQueue::from_context());
        Effect::new(move |_| {
            auth.generation.track();
            projects.active_project.track();
            revision.update_value(|revision| *revision += 1);
            draft.set(String::new());
            kind.set(None);
            busy.set(false);
            error.set(None);
        });
        let generate = Callback::new(move |requested: AssistanceKind| {
            if busy.get_untracked() || chat.streaming.get_untracked() {
                return;
            }
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            let session_id = chat.active_session.get_untracked();
            let connection_id = chat
                .sessions
                .with_untracked(|sessions| {
                    sessions
                        .iter()
                        .find(|session| {
                            Some(session.id) == session_id && session.project_id == Some(project)
                        })
                        .and_then(|session| session.connection_id)
                })
                .or(settings.default_connection.get_untracked())
                .or_else(|| {
                    settings.model_setup.with_untracked(|setup| {
                        setup.defaults.primary.as_ref().map(|model| model.server_id)
                    })
                });
            let Some(connection_id) = connection_id else {
                error.set(Some("Choose a model in Settings first.".into()));
                return;
            };
            let epoch = auth.generation.get_untracked();
            let host_revision = repository.revision();
            revision.update_value(|revision| *revision += 1);
            let ticket = revision.get_value();
            let original = draft.get_untracked();
            busy.set(true);
            error.set(None);
            spawn_local(async move {
                let current = move || {
                    auth.generation.try_get_untracked() == Some(epoch)
                        && projects.active_project.try_get_untracked() == Some(Some(project))
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
                    let tracked_diff = repo.diff(None).await?;
                    let mut changes = tracked_diff.clone();
                    let mut new_files = Vec::new();
                    let workspace = Workspace::for_project(api, projects, project).ok_or("Reconnect the project folder first.")?;
                    for (path, status) in status.files.iter().filter(|(_, status)| **status == GitFileStatus::Untracked).take(8) {
                        let _ = status;
                        if let Ok(content) = workspace.read(path).await {
                            new_files.push((path.clone(), fingerprint(&content)));
                            changes.push_str(&format!("\nNew file {path}:\n{}\n", content.chars().take(1500).collect::<String>()));
                        }
                    }
                    if changes.trim().is_empty() { return Err("There are no changes to describe.".into()); }
                    let guidance = workspace.read("AGENTS.md").await.unwrap_or_default();
                    let input = format!("Repository formatting conventions:\n{}\nCurrent branch: {}\nActual changes:\n{}", guidance.chars().take(6000).collect::<String>(), status.branch, changes.chars().take(16000).collect::<String>());
                    if !current() || chat.streaming.get_untracked() { return Err("Draft request superseded.".into()); }
                    let request = AssistanceRequest { kind: requested, connection_id, project_id: Some(project), session_id: None, input };
                    let generated = api.with_value(Clone::clone).assistance(&request).await?.ok_or("The model could not produce a draft. Try again.")?;
                    // A diff changed during generation must not produce a misleading draft.
                    let fresh_status = repo.status().await?;
                    if repo.diff(None).await? != tracked_diff || fresh_status.files != status.files || fresh_status.branch != status.branch { return Err("Changes moved while drafting. Try again.".into()); }
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
                        draft.set(text);
                        kind.set(Some(requested));
                    }
                    Ok(_) => error.set(Some(
                        "Kept your edited draft. Generate again when ready.".into(),
                    )),
                    Err(message) => error.set(Some(message)),
                }
            });
        });
        Self {
            draft,
            kind,
            busy,
            error,
            generate,
        }
    }
}

fn fingerprint(content: &str) -> u64 {
    use std::hash::Hasher;
    let mut hash = std::hash::DefaultHasher::new();
    hash.write(content.as_bytes());
    hash.finish()
}
