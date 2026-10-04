//! Conversation branches preserve their source session and shared project files.
use super::*;
use openwebide_core::{Compaction, ForkedSession};

impl<D: Db> Store<D> {
    pub async fn fork_session(
        &self,
        user: UserId,
        source: i64,
        prompt_id: i64,
        created_at: i64,
    ) -> Result<ForkedSession, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            let source_session = store.get_session(source, user).await?;
            store.ensure_not_rewinding(source).await?;
            let messages = store.list_messages(source).await?;
            let prompt = messages.iter().find(|message| message.id == prompt_id && message.role == Role::User).ok_or_else(|| StorageError::NotFound("user prompt".into()))?;
            let steps = store.list_tool_steps(source).await?;
            if steps.iter().any(|step| step.anchor_message_id < prompt_id && step.ok.is_none()) {
                return Err(StorageError::Conflict("Wait for the earlier tools to finish before branching at this prompt.".into()));
            }
            let session = store.create_session_unlocked(&format!("{} (branch)", source_session.name), source_session.connection_id, source_session.system_prompt_id, source_session.project_id, user, created_at).await?;
            let mode = store.get_user_setting(user, &openwebide_core::ApprovalMode::setting_key(source)).await?.unwrap_or_else(|| serde_json::to_string(&openwebide_core::ApprovalMode::default()).expect("approval mode serializes"));
            store.set_user_setting(user, &openwebide_core::ApprovalMode::setting_key(session.id), &mode).await?;
            let mut ids = BTreeMap::new();
            for original in messages.iter().filter(|message| message.id < prompt_id) {
                let mut content = original.content.clone();
                if original.role == Role::System && let Some(mut compaction) = Compaction::parse(&content) {
                    let Some(through) = ids.get(&compaction.through_message_id).copied() else { continue; };
                    compaction.through_message_id = through;
                    for retained in &mut compaction.retained {
                        retained.id = ids.get(&retained.id).copied().ok_or_else(|| StorageError::Conflict("Compaction references missing branch history".into()))?;
                        retained.session_id = session.id;
                    }
                    content = compaction.stored_content().map_err(|error| StorageError::Db(error.to_string()))?;
                }
                let copied = store.insert_interim_message_unlocked(session.id, original.role, &content, original.created_at, original.usage.as_ref(), original.tool_calls.as_deref()).await?;
                ids.insert(original.id, copied.id);
            }
            for step in steps.iter().filter(|step| step.anchor_message_id < prompt_id) {
                let anchor = ids.get(&step.anchor_message_id).ok_or_else(|| StorageError::Conflict("Tool references missing branch history".into()))?;
                // Copy history/checkpoints without applying edits or duplicating
                // project review decisions. Tool wire IDs remain session scoped.
                store.db.execute("INSERT INTO tool_steps (session_id, anchor_message_id, tool_call_id, name, summary, ok, result_summary, diff, created_at, completion_applied, checkpoint, timing, execution_order) SELECT ?, ?, tool_call_id, name, summary, ok, result_summary, diff, created_at, completion_applied, checkpoint, timing, execution_order FROM tool_steps WHERE session_id = ? AND tool_call_id = ?", &[DbValue::Int(session.id), DbValue::Int(*anchor), DbValue::Int(source), DbValue::Text(step.tool_call_id.clone())]).await?;
            }
            for (original, copied) in &ids {
                store.db.execute("INSERT INTO todo_updates (session_id, anchor_message_id, plan, created_at) SELECT ?, ?, plan, created_at FROM todo_updates WHERE session_id = ? AND anchor_message_id = ? ORDER BY id", &[DbValue::Int(session.id), DbValue::Int(*copied), DbValue::Int(source), DbValue::Int(*original)]).await?;
            }
            for (original, copied) in &ids {
                store.db.execute("INSERT INTO task_runs (session_id, task_id, anchor_message_id, snapshot) SELECT ?, task_id, ?, snapshot FROM task_runs WHERE session_id = ? AND anchor_message_id = ?", &[DbValue::Int(session.id), DbValue::Int(*copied), DbValue::Int(source), DbValue::Int(*original)]).await?;
            }
            let history = store.list_conversation(session.id).await?;
            Ok(ForkedSession { session, prompt: prompt.content.clone(), history })
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;
    use openwebide_core::{PromptContent, PromptImage};

    #[test]
    fn fork_copies_the_prefix_remaps_compaction_and_tool_anchors_and_preserves_source_in_both_modes()
     {
        block_on(async {
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
                let other = store
                    .insert_user("other", "hash", UserRole::User, 1)
                    .await
                    .unwrap()
                    .id;
                let project = if let Some(mode) = mode {
                    Some(
                        store
                            .create_project(
                                &NewProject {
                                    name: "project".into(),
                                    mode,
                                    path: Some("repos/test".into()),
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
                let source = store
                    .create_session("source", None, None, project, user, 1)
                    .await
                    .unwrap();
                store
                    .set_user_setting(
                        user,
                        &openwebide_core::ApprovalMode::setting_key(source.id),
                        "\"yolo\"",
                    )
                    .await
                    .unwrap();
                let first = store
                    .insert_message(source.id, Role::User, "earlier prompt", 2)
                    .await
                    .unwrap();
                let call = ToolCall {
                    id: "provider-call".into(),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path":"a.txt"}).to_string(),
                };
                let interim = store
                    .insert_interim_message(
                        source.id,
                        Role::Assistant,
                        "reading",
                        2,
                        None,
                        Some(&[call]),
                    )
                    .await
                    .unwrap();
                let step_id = format!("a{}t1c0", first.id);
                store
                    .upsert_tool_step(
                        source.id,
                        interim.id,
                        &step_id,
                        "read_file",
                        "a.txt",
                        2,
                        None,
                    )
                    .await
                    .unwrap();
                let checkpoint = openwebide_core::rewind::ProjectCheckpoint {
                    before: BTreeMap::new(),
                    after: Some(BTreeMap::new()),
                    skipped: BTreeMap::new(),
                };
                store
                    .save_project_checkpoint(source.id, &step_id, &checkpoint)
                    .await
                    .unwrap();
                store
                    .complete_tool_step(user, source.id, &step_id, true, "contents", None)
                    .await
                    .unwrap();
                let reply = store
                    .insert_message_with_usage(
                        source.id,
                        Role::Assistant,
                        "<think>reasoning</think>answer",
                        3,
                        Some(&TurnTelemetry {
                            prompt_tokens: 42,
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap();
                let compaction = Compaction {
                    summary: "earlier work".into(),
                    retained: vec![first.clone()],
                    through_message_id: reply.id,
                };
                store
                    .insert_message(
                        source.id,
                        Role::System,
                        &compaction.stored_content().unwrap(),
                        4,
                    )
                    .await
                    .unwrap();
                let content = PromptContent {
                    text: "selected prompt".into(),
                    images: vec![
                        PromptImage::from_bytes("test.png".into(), b"\x89PNG\r\n\x1a\n").unwrap(),
                    ],
                    ..Default::default()
                }
                .encode()
                .unwrap();
                let target = store
                    .insert_message(source.id, Role::User, &content, 5)
                    .await
                    .unwrap();
                store
                    .insert_message(source.id, Role::Assistant, "later reply", 6)
                    .await
                    .unwrap();
                let queue = store
                    .enqueue_prompt(user, source.id, "original followup", 7)
                    .await
                    .unwrap();
                let original = store.list_conversation(source.id).await.unwrap();
                let original_sessions = store.list_sessions(user).await.unwrap();
                assert!(
                    store
                        .fork_session(other, source.id, target.id, 8)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .fork_session(user, source.id, reply.id, 8)
                        .await
                        .is_err()
                );
                // Force failure after the new session/messages have been copied.
                store.db.execute(&format!("CREATE TRIGGER fail_branch BEFORE INSERT ON tool_steps WHEN NEW.session_id != {} BEGIN SELECT RAISE(ABORT, 'failure'); END", source.id), &[]).await.unwrap();
                assert!(
                    store
                        .fork_session(user, source.id, target.id, 8)
                        .await
                        .is_err()
                );
                assert_eq!(store.list_sessions(user).await.unwrap(), original_sessions);
                assert_eq!(store.list_conversation(source.id).await.unwrap(), original);
                store
                    .db
                    .execute("DROP TRIGGER fail_branch", &[])
                    .await
                    .unwrap();
                let branch = store
                    .fork_session(user, source.id, target.id, 8)
                    .await
                    .unwrap();
                assert_eq!(branch.prompt, content);
                assert_eq!(branch.session.project_id, project);
                assert_eq!(branch.session.connection_id, source.connection_id);
                assert_eq!(
                    store
                        .get_user_setting(
                            user,
                            &openwebide_core::ApprovalMode::setting_key(branch.session.id)
                        )
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("\"yolo\"")
                );
                let copied = store.list_messages(branch.session.id).await.unwrap();
                assert_eq!(copied.len(), 4);
                assert!(copied.iter().all(
                    |message| message.session_id == branch.session.id && message.id > target.id
                ));
                assert_eq!(copied[0].content, first.content);
                assert_eq!(copied[2].usage, reply.usage);
                assert_eq!(copied[2].content, reply.content);
                let compaction = Compaction::parse(&copied[3].content).unwrap();
                assert_eq!(compaction.through_message_id, copied[2].id);
                assert_eq!(compaction.retained[0].id, copied[0].id);
                assert_eq!(compaction.retained[0].session_id, branch.session.id);
                let copied_steps = store.list_tool_steps(branch.session.id).await.unwrap();
                assert_eq!(copied_steps.len(), 1);
                assert_eq!(copied_steps[0].anchor_message_id, copied[1].id);
                assert_eq!(copied_steps[0].checkpoint, Some(checkpoint));
                let tool_history = openwebide_core::tool_history(copied, &copied_steps);
                assert!(tool_history.iter().any(|message| message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some("provider-call")
                    && message.content == "contents"));
                assert_eq!(store.list_conversation(source.id).await.unwrap(), original);
                assert_eq!(
                    store
                        .list_queued_prompts(user, source.id)
                        .await
                        .unwrap()
                        .as_slice(),
                    std::slice::from_ref(&queue)
                );
                assert!(
                    store
                        .list_queued_prompts(user, branch.session.id)
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
        });
    }
}
