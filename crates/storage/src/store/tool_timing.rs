use super::*;
use openwebide_core::ToolTiming;

impl<D: Db + Send + Sync> Store<D> {
    pub fn save_tool_timing<'a>(
        &'a self,
        user: UserId,
        session: i64,
        id: &'a str,
        timing: &'a ToolTiming,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StorageError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.db.transaction(|tx| async move {
                let store = Store::new(tx);
                store.get_session(session, user).await?;
                store.ensure_not_rewinding(session).await?;
                let rows = store.db.execute("SELECT timing FROM tool_steps WHERE session_id = ? AND tool_call_id = ?", &[DbValue::Int(session), DbValue::Text(id.into())]).await?;
                let row = rows.rows.first().ok_or_else(|| StorageError::NotFound("Tool step not found".into()))?;
                let previous: Option<ToolTiming> = row.get_text_opt(0).map(serde_json::from_str).transpose().map_err(|error| StorageError::InvalidValue(error.to_string()))?;
                let timing = previous.map_or(Ok(*timing), |previous| previous.merge(*timing)).map_err(|error| StorageError::Conflict(error.into()))?;
                if previous == Some(timing) { return Ok(()); }
                let json = serde_json::to_string(&timing).map_err(|error| StorageError::InvalidValue(error.to_string()))?;
                store.db.execute("UPDATE tool_steps SET timing = ? WHERE session_id = ? AND tool_call_id = ?", &[DbValue::Text(json), DbValue::Int(session), DbValue::Text(id.into())]).await?;
                Ok(())
            }).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;

    #[test]
    fn timing_is_owned_durable_replay_safe_and_follows_fork_and_rewind_in_all_modes() {
        block_on(async {
            for mode in [
                Some(WorkspaceMode::Local),
                Some(WorkspaceMode::Remote),
                None,
            ] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
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
                let first = store
                    .insert_message(session, Role::User, "first", 1)
                    .await
                    .unwrap();
                store
                    .upsert_tool_step(session, first.id, "one", "host_info", "host", 1, None)
                    .await
                    .unwrap();
                let started = ToolTiming::start(1000);
                assert!(matches!(
                    store
                        .save_tool_timing(other, session, "one", &started)
                        .await,
                    Err(StorageError::NotFound(_))
                ));
                assert!(matches!(
                    store
                        .save_tool_timing(user, session, "missing", &started)
                        .await,
                    Err(StorageError::NotFound(_))
                ));
                store
                    .save_tool_timing(user, session, "one", &started)
                    .await
                    .unwrap();
                let finished = started.sample(2400, true);
                store
                    .save_tool_timing(user, session, "one", &finished)
                    .await
                    .unwrap();
                store
                    .save_tool_timing(user, session, "one", &started)
                    .await
                    .unwrap();
                assert_eq!(
                    store.list_tool_steps(session).await.unwrap()[0].timing,
                    Some(finished)
                );
                assert!(matches!(
                    store
                        .save_tool_timing(user, session, "one", &ToolTiming::start(999))
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                store
                    .complete_tool_step(user, session, "one", true, "host", None)
                    .await
                    .unwrap();
                let second = store
                    .insert_message(session, Role::User, "second", 2)
                    .await
                    .unwrap();
                store
                    .upsert_tool_step(session, second.id, "two", "host_info", "host", 2, None)
                    .await
                    .unwrap();
                store.db.execute("CREATE TRIGGER fail_timing BEFORE UPDATE OF timing ON tool_steps BEGIN SELECT RAISE(ABORT, 'failed'); END", &[]).await.unwrap();
                assert!(
                    store
                        .save_tool_timing(user, session, "two", &ToolTiming::start(3000))
                        .await
                        .is_err()
                );
                assert_eq!(
                    store.list_tool_steps(session).await.unwrap()[1].timing,
                    None
                );
                store
                    .db
                    .execute("DROP TRIGGER fail_timing", &[])
                    .await
                    .unwrap();
                store
                    .save_tool_timing(
                        user,
                        session,
                        "two",
                        &ToolTiming::start(3000).sample(3400, true),
                    )
                    .await
                    .unwrap();
                store
                    .complete_tool_step(user, session, "two", false, "failed", None)
                    .await
                    .unwrap();
                let branch = store
                    .fork_session(user, session, second.id, 3)
                    .await
                    .unwrap();
                let steps = store.list_tool_steps(branch.session.id).await.unwrap();
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].timing, Some(finished));
                store
                    .prepare_rewind(user, session, second.id)
                    .await
                    .unwrap();
                assert!(matches!(
                    store
                        .save_tool_timing(user, session, "one", &finished)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                store
                    .complete_rewind(user, session, second.id)
                    .await
                    .unwrap();
                let steps = store.list_tool_steps(session).await.unwrap();
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].timing, Some(finished));
            }
        });
    }
}
