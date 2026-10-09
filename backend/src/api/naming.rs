//! Shared naming orchestration for user and agent writes in every workspace mode.
use super::*;
use crate::state::AppDb;
use openwebide_core::{AssistanceKind, AssistanceRequest, MemoryCommand, scheduled::TaskCommand};
use openwebide_storage::Store;

async fn name(
    store: &Store<AppDb>,
    user: UserId,
    project: Option<i64>,
    session: Option<i64>,
    kind: AssistanceKind,
    content: &str,
) -> String {
    let connection = if let Some(session) = session {
        store
            .get_session(session, user)
            .await
            .ok()
            .and_then(|session| session.connection_id)
    } else {
        None
    };
    let connection = match connection {
        Some(connection) => Some(connection),
        None => store
            .model_setup(user)
            .await
            .ok()
            .and_then(|setup| setup.defaults.primary.map(|model| model.server_id)),
    };
    if let Some(connection_id) = connection {
        let request = AssistanceRequest {
            connection_id,
            session_id: session,
            project_id: project,
            kind,
            input: content.into(),
        };
        if let Ok(Some(title)) = super::assistance::execute(store, user, &request).await {
            return title;
        }
    }
    openwebide_core::assistance::fallback_name(content)
}

pub(crate) async fn memory(
    store: &Store<AppDb>,
    user: UserId,
    project: i64,
    session: Option<i64>,
    mut command: MemoryCommand,
) -> Result<MemoryCommand, ApiError> {
    store.get_project(project, user).await?;
    command.validate().map_err(ApiError::bad_request)?;
    // Agent content updates retain names the user assigned explicitly.
    if session.is_some()
        && let MemoryCommand::Update {
            id,
            auto_title,
            title,
            ..
        } = &mut command
        && *auto_title
    {
        let memories = store.project_memories(user, project).await?;
        if let Some(entry) = memories.entries.iter().find(|entry| entry.id == *id)
            && !entry.auto_title
        {
            *auto_title = false;
            *title = entry.title.clone();
        }
    }
    if let MemoryCommand::Create {
        auto_title: true,
        title,
        content,
    }
    | MemoryCommand::Update {
        auto_title: true,
        title,
        content,
        ..
    } = &mut command
    {
        *title = name(
            store,
            user,
            Some(project),
            session,
            AssistanceKind::MemoryName,
            content,
        )
        .await;
    }
    Ok(command)
}

pub(crate) async fn task(
    store: &Store<AppDb>,
    user: UserId,
    project: Option<i64>,
    mut command: TaskCommand,
    agent: bool,
) -> Result<TaskCommand, ApiError> {
    if let Some(project) = project {
        store.get_project(project, user).await?;
    }
    if agent
        && let TaskCommand::Update { id, draft, .. } = &mut command
        && draft.auto_title
        && let Some(existing) = store
            .scheduled_tasks(user, project, now())
            .await?
            .into_iter()
            .find(|entry| entry.id == *id)
        && !existing.draft.auto_title
    {
        draft.auto_title = false;
        draft.title = existing.draft.title;
    }
    if let TaskCommand::Create { draft } | TaskCommand::Update { draft, .. } = &mut command {
        let session = if draft.session_target == openwebide_core::scheduled::SessionTarget::Existing
        {
            let session = store.get_session(draft.session_id, user).await?;
            if session.project_id != project {
                return Err(ApiError::bad_request(
                    "Task session belongs to a different project",
                ));
            }
            Some(session.id)
        } else {
            None
        };
        draft.validate(now()).map_err(ApiError::bad_request)?;
        if draft.auto_title {
            draft.title = name(
                store,
                user,
                project,
                session,
                AssistanceKind::TaskName,
                &draft.prompt,
            )
            .await;
        }
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{
        NewProject, UserRole, WorkspaceMode,
        scheduled::{Schedule, TaskDraft},
    };
    use openwebide_storage::rusqlite_db::RusqliteDb;
    #[test]
    fn automatic_names_refresh_and_manual_names_survive_in_every_scope() {
        futures::executor::block_on(async {
            for mode in [
                Some(WorkspaceMode::Local),
                Some(WorkspaceMode::Remote),
                None,
            ] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
                let user = store
                    .insert_user("owner", "hash", UserRole::Admin, 1)
                    .await
                    .unwrap()
                    .id;
                let project = if let Some(mode) = mode {
                    Some(
                        store
                            .create_project(
                                &NewProject {
                                    name: "test".into(),
                                    mode,
                                    path: Some("test".into()),
                                },
                                user,
                                1,
                            )
                            .await
                            .unwrap()
                            .id,
                    )
                } else {
                    None
                };
                let session = store
                    .create_session("test", None, None, project, user, 1)
                    .await
                    .unwrap()
                    .id;
                if let Some(project) = project {
                    let automatic = memory(
                        &store,
                        user,
                        project,
                        Some(session),
                        MemoryCommand::Create {
                            auto_title: true,
                            title: String::new(),
                            content: "Run cargo test before pushing".into(),
                        },
                    )
                    .await
                    .unwrap();
                    let saved = store
                        .memory_command(user, project, &automatic, false, 1)
                        .await
                        .unwrap();
                    let entry = &saved.entries[0];
                    assert!(entry.auto_title);
                    assert_eq!(entry.title, "Run cargo test before pushing");
                    let updated = memory(
                        &store,
                        user,
                        project,
                        Some(session),
                        MemoryCommand::Update {
                            id: entry.id,
                            revision: entry.revision,
                            auto_title: true,
                            title: entry.title.clone(),
                            content: "Run frontend browser checks".into(),
                        },
                    )
                    .await
                    .unwrap();
                    let saved = store
                        .memory_command(user, project, &updated, false, 2)
                        .await
                        .unwrap();
                    let entry = &saved.entries[0];
                    assert_eq!(entry.title, "Run frontend browser checks");
                    let manual = memory(
                        &store,
                        user,
                        project,
                        Some(session),
                        MemoryCommand::Update {
                            id: entry.id,
                            revision: entry.revision,
                            auto_title: false,
                            title: "My build instructions".into(),
                            content: "Changed content".into(),
                        },
                    )
                    .await
                    .unwrap();
                    let saved = store
                        .memory_command(user, project, &manual, false, 3)
                        .await
                        .unwrap();
                    assert!(!saved.entries[0].auto_title);
                    assert_eq!(saved.entries[0].title, "My build instructions");
                }
                let draft = TaskDraft {
                    model: None,
                    session_target: openwebide_core::scheduled::SessionTarget::Existing,
                    auto_title: true,
                    title: String::new(),
                    prompt: "Check package upgrades".into(),
                    session_id: session,
                    schedule: Schedule::Once { at: now() + 600 },
                    enabled: true,
                };
                let named = task(&store, user, project, TaskCommand::Create { draft }, false)
                    .await
                    .unwrap();
                let TaskCommand::Create { draft } = named else {
                    panic!("Expected task draft")
                };
                assert!(draft.auto_title);
                assert_eq!(draft.title, "Check package upgrades");
                let mut manual = draft;
                manual.auto_title = false;
                manual.title = "My dependency check".into();
                manual.prompt = "Entirely different activity".into();
                let named = task(
                    &store,
                    user,
                    project,
                    TaskCommand::Create { draft: manual },
                    false,
                )
                .await
                .unwrap();
                let TaskCommand::Create { draft } = named else {
                    panic!("Expected task draft")
                };
                assert_eq!(draft.title, "My dependency check");
                let binding = openwebide_core::scheduled::HostBinding {
                    host_id: "host".into(),
                    path: "test".into(),
                };
                let saved = store
                    .scheduled_command(
                        user,
                        project,
                        &TaskCommand::Create {
                            draft: draft.clone(),
                        },
                        if mode == Some(WorkspaceMode::Local) {
                            Some(&binding)
                        } else {
                            None
                        },
                        false,
                        now(),
                    )
                    .await
                    .unwrap();
                let entry = &saved[0];
                let mut automatic_update = draft;
                automatic_update.title.clear();
                automatic_update.auto_title = true;
                automatic_update.prompt = "New agent prompt".into();
                let update = task(
                    &store,
                    user,
                    project,
                    TaskCommand::Update {
                        id: entry.id,
                        revision: entry.revision,
                        draft: automatic_update,
                    },
                    true,
                )
                .await
                .unwrap();
                let TaskCommand::Update { draft, .. } = update else {
                    panic!("Expected updated task")
                };
                assert_eq!(draft.title, "My dependency check");
                assert!(!draft.auto_title);
            }
        });
    }
}
