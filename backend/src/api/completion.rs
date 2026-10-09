//! Completion evidence and optional generation shared by scheduled results and notifications.
use super::*;
use crate::state::AppDb;
use openwebide_core::{AssistanceKind, AssistanceRequest};
use openwebide_storage::Store;

/// Locate the completed response for one injected prompt, stopping at the next user turn.
pub(crate) fn after_prompt(messages: &[openwebide_core::ChatMessage], anchor: i64) -> Option<i64> {
    messages
        .iter()
        .skip_while(|message| message.id <= anchor)
        .take_while(|message| message.role != Role::User)
        .filter(|message| {
            message.role == Role::Assistant && message.tool_calls.as_ref().is_none_or(Vec::is_empty)
        })
        .last()
        .map(|message| message.id)
}
pub(crate) async fn summary(
    store: &Store<AppDb>,
    user: UserId,
    session_id: i64,
    message_id: Option<i64>,
) -> Option<String> {
    let session = store.get_session(session_id, user).await.ok()?;
    let messages = store.list_messages(session_id).await.ok()?;
    let message = messages.iter().rev().find(|message| {
        message.role == Role::Assistant
            && message.tool_calls.as_ref().is_none_or(Vec::is_empty)
            && message_id.is_none_or(|id| id == message.id)
    })?;
    let fallback = openwebide_core::assistance::completion_excerpt(&message.content);
    // A notification for an old run must not describe a newer turn.
    if messages.last().is_none_or(|last| last.id != message.id) {
        return Some(fallback);
    }
    let anchor = messages
        .iter()
        .rev()
        .find(|entry| entry.role == Role::User && entry.id < message.id)
        .map_or(message.id, |entry| entry.id);
    let mut input = format!(
        "Final response: {}",
        openwebide_core::strip_reasoning(&message.content)
            .chars()
            .take(6000)
            .collect::<String>()
    );
    for step in store
        .list_tool_steps(session_id)
        .await
        .ok()?
        .iter()
        .filter(|step| step.anchor_message_id >= anchor && step.anchor_message_id <= message.id)
        .rev()
        .take(12)
    {
        input.push_str(&format!(
            "\nTool {}: {:?}. {}",
            step.name,
            step.ok,
            step.result_summary
                .as_deref()
                .unwrap_or("unfinished")
                .chars()
                .take(1000)
                .collect::<String>()
        ));
    }
    if store
        .session_run_active(user, session_id, now())
        .await
        .ok()?
    {
        return Some(fallback);
    }
    let connection = match session.connection_id {
        Some(connection) => Some(connection),
        None => store
            .model_setup(user)
            .await
            .ok()
            .and_then(|setup| setup.defaults.primary.map(|model| model.server_id)),
    };
    if let Some(connection_id) = connection {
        let request = AssistanceRequest {
            kind: AssistanceKind::Completion,
            connection_id,
            session_id: Some(session_id),
            project_id: session.project_id,
            input,
        };
        if let Ok(Some(summary)) = super::assistance::execute(store, user, &request).await {
            // Recheck the source after optional work; fall back to the original result on a new turn.
            if store
                .list_messages(session_id)
                .await
                .ok()?
                .last()
                .map(|entry| entry.id)
                == Some(message.id)
            {
                return Some(summary);
            }
        }
    }
    Some(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{NewProject, UserRole};
    use openwebide_storage::rusqlite_db::RusqliteDb;
    #[test]
    fn notification_fallback_preserves_the_actual_run_and_account() {
        futures::executor::block_on(async {
            for mode in [
                Some(WorkspaceMode::Local),
                Some(WorkspaceMode::Remote),
                None,
            ] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
                let owner = store
                    .insert_user("owner", "hash", UserRole::Admin, 1)
                    .await
                    .unwrap()
                    .id;
                let other = store
                    .insert_user("other", "hash", UserRole::User, 1)
                    .await
                    .unwrap()
                    .id;
                let project = match mode {
                    Some(mode) => Some(
                        store
                            .create_project(
                                &NewProject {
                                    name: "project".into(),
                                    mode,
                                    path: Some("project".into()),
                                },
                                owner,
                                1,
                            )
                            .await
                            .unwrap()
                            .id,
                    ),
                    None => None,
                };
                let session = store
                    .create_session("session", None, None, project, owner, 1)
                    .await
                    .unwrap();
                let prompt = store
                    .insert_message(session.id, Role::User, "run tests", 1)
                    .await
                    .unwrap();
                let failed = store
                    .insert_message(
                        session.id,
                        Role::Assistant,
                        "<think>private reasoning</think>Tests failed; awaiting a fix.",
                        2,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    summary(&store, owner, session.id, Some(failed.id))
                        .await
                        .as_deref(),
                    Some("Tests failed; awaiting a fix.")
                );
                assert!(
                    summary(&store, other, session.id, Some(failed.id))
                        .await
                        .is_none()
                );
                store
                    .insert_message(session.id, Role::User, "new request", 3)
                    .await
                    .unwrap();
                store
                    .insert_message(session.id, Role::Assistant, "Different outcome", 4)
                    .await
                    .unwrap();
                assert_eq!(
                    after_prompt(&store.list_messages(session.id).await.unwrap(), prompt.id),
                    Some(failed.id)
                );
                assert_eq!(
                    summary(&store, owner, session.id, Some(failed.id))
                        .await
                        .as_deref(),
                    Some("Tests failed; awaiting a fix.")
                );
            }
        });
    }
}
