//! Durable session goals and compare-and-swap conversation summaries.
use super::*;

impl<D: Db> Store<D> {
    pub async fn save_manual_compaction(
        &self,
        user: UserId,
        session: i64,
        through: i64,
        compaction: &openwebide_core::Compaction,
        started_ms: i64,
        now: i64,
    ) -> Result<ChatMessage, StorageError> {
        let mut compaction = compaction.clone();
        compaction.through_message_id = through;
        let content = compaction
            .stored_content()
            .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
        self.db
            .transaction(|tx| async move {
                let store = Store::new(tx);
                store.get_session(session, user).await?;
                store.ensure_not_rewinding(session).await?;
                if store.cancel_requested_since(session, started_ms).await? {
                    return Err(StorageError::Conflict(
                        "Compaction stopped. Original history has been retained.".into(),
                    ));
                }
                let latest = store
                    .db
                    .execute(
                        "SELECT MAX(id) FROM messages WHERE session_id = ?",
                        &[DbValue::Int(session)],
                    )
                    .await?;
                if latest.rows.first().and_then(|row| row.get_int(0).ok()) != Some(through) {
                    return Err(StorageError::Conflict(
                        "The conversation changed while compacting; retry when the run is idle."
                            .into(),
                    ));
                }
                store
                    .insert_interim_message_unlocked(
                        session,
                        Role::System,
                        &content,
                        now,
                        None,
                        None,
                    )
                    .await
            })
            .await
    }
}

impl<D: Db> Store<D> {
    pub async fn get_goal(
        &self,
        user: UserId,
        session: i64,
    ) -> Result<Option<openwebide_core::Goal>, StorageError> {
        self.get_session(session, user).await?;
        let result = self
            .db
            .execute(
                "SELECT goal FROM session_goals WHERE session_id = ?",
                &[DbValue::Int(session)],
            )
            .await?;
        result
            .rows
            .first()
            .map(|row| {
                serde_json::from_str(row.get_text(0)?)
                    .map_err(|error| StorageError::Db(error.to_string()))
            })
            .transpose()
    }
    pub async fn update_goal(
        &self,
        user: UserId,
        session: i64,
        expected_revision: u64,
        command: openwebide_core::GoalCommand,
        now: i64,
    ) -> Result<openwebide_core::Goal, StorageError> {
        self.db
            .transaction(|tx| async move {
                Store::new(tx)
                    .change_goal(user, session, expected_revision, command, now)
                    .await
            })
            .await
    }
    async fn change_goal(
        &self,
        user: UserId,
        session: i64,
        expected_revision: u64,
        command: openwebide_core::GoalCommand,
        now: i64,
    ) -> Result<openwebide_core::Goal, StorageError> {
        let existing = self.get_goal(user, session).await?;
        self.ensure_not_rewinding(session).await?;
        if existing.as_ref().map_or(0, |goal| goal.revision) != expected_revision {
            return Err(StorageError::Conflict(
                "The goal changed in another window; refresh before updating it.".into(),
            ));
        }
        let goal = openwebide_core::Goal::transition(existing.as_ref(), session, command, now)
            .map_err(StorageError::InvalidValue)?;
        self.save_goal(&goal).await?;
        Ok(goal)
    }
    async fn save_goal(&self, goal: &openwebide_core::Goal) -> Result<(), StorageError> {
        let json =
            serde_json::to_string(goal).map_err(|error| StorageError::Db(error.to_string()))?;
        self.db.execute("INSERT INTO session_goals (session_id, goal) VALUES (?, ?) ON CONFLICT(session_id) DO UPDATE SET goal = excluded.goal", &[DbValue::Int(goal.session_id), DbValue::Text(json)]).await?;
        Ok(())
    }
    /// Activation and its host task commit together; legacy goals opt in on resume.
    pub async fn dispatch_goal(
        &self,
        user: UserId,
        session: i64,
        expected_revision: u64,
        command: openwebide_core::GoalCommand,
        binding: Option<&openwebide_core::scheduled::HostBinding>,
        now: i64,
    ) -> Result<openwebide_core::Goal, StorageError> {
        use openwebide_core::scheduled::{HostBinding, Schedule, SessionTarget, TaskDraft};
        self.db.transaction(|tx| async move {
            let store=Store::new(tx);
            let session_data=store.get_session(session,user).await?;
            let activating=matches!(command,openwebide_core::GoalCommand::Start{..}|openwebide_core::GoalCommand::Resume);
            let mut goal=store.change_goal(user,session,expected_revision,command,now).await?;
            let existing=store.db.execute("SELECT task_id FROM goal_workers WHERE session_id=?", &[DbValue::Int(session)]).await?;
            let task=existing.rows.first().map(|row|row.get_int(0)).transpose()?;
            if let Some(task)=task {store.cancel_scheduled_pending(task).await?;}
            if activating {
                let local=if let Some(project)=session_data.project_id {store.get_project(project,user).await?.mode==WorkspaceMode::Local} else {false};
                let key=format!("scheduled_host_{}",session_data.project_id.unwrap_or(0));
                if let Some(binding)=binding {
                    if binding.host_id.is_empty() || binding.host_id.len()>256 || binding.path.is_empty() || binding.path.len()>4096 {
                        return Err(StorageError::InvalidRequest("Invalid execution host.".into()));
                    }
                    store.set_user_setting(user,&key,&serde_json::to_string(binding).map_err(|e|StorageError::Db(e.to_string()))?).await?;
                }
                let saved=store.get_user_setting(user,&key).await?.map(|value|serde_json::from_str::<HostBinding>(&value).map_err(|e|StorageError::Db(e.to_string()))).transpose()?;
                let bound=if local {Some(saved.ok_or_else(||StorageError::InvalidRequest("Connect this folder to its host before starting unattended goal work.".into()))?)} else {None};
                let host=bound.as_ref().map_or("server",|binding|binding.host_id.as_str());
                let path=bound.as_ref().map_or(DbValue::Null,|binding|DbValue::Text(binding.path.clone()));
                let draft=TaskDraft{session_target:SessionTarget::Existing,auto_title:false,title:"Session goal".into(),prompt:goal.prompt(),session_id:session,model:None,schedule:Schedule::Once{at:now+1},enabled:true};
                let json=serde_json::to_string(&draft).map_err(|e|StorageError::Db(e.to_string()))?;
                let task=if let Some(task)=task {
                    store.db.execute("UPDATE scheduled_tasks SET draft=?,enabled=1,next_run=?,host_id=?,path=?,revision=revision+1 WHERE id=?", &[DbValue::Text(json),DbValue::Int(now+1),DbValue::Text(host.into()),path,DbValue::Int(task)]).await?;
                    task
                } else {
                    store.db.execute("INSERT INTO scheduled_tasks(user_id,project_id,session_id,draft,enabled,next_run,host_id,path) VALUES (?,?,?,?,1,?,?,?)", &[DbValue::Int(user.get()),session_data.project_id.map_or(DbValue::Null,DbValue::Int),DbValue::Int(session),DbValue::Text(json),DbValue::Int(now+1),DbValue::Text(host.into()),path]).await?.last_insert_rowid
                };
                let revision=i64::try_from(goal.revision).map_err(|_|StorageError::InvalidValue("Goal revision exhausted".into()))?;
                store.db.execute("INSERT INTO goal_workers(session_id,task_id,revision) VALUES (?,?,?) ON CONFLICT(session_id) DO UPDATE SET revision=excluded.revision,turns=0,stalls=0", &[DbValue::Int(session),DbValue::Int(task),DbValue::Int(revision)]).await?;
                goal.worker=true;
                goal.note=Some("Queued on the execution host.".into());
            } else if let Some(task)=task {
                store.db.execute("UPDATE scheduled_tasks SET enabled=0,next_run=NULL WHERE id=?", &[DbValue::Int(task)]).await?;
                // Host control checks the durable goal revision, including after resume.
            }
            store.save_goal(&goal).await?;
            Ok(goal)
        }).await
    }
    /// Foreground input withdraws unconsumed goal prompts before taking queue priority.
    pub(super) async fn yield_goal_pending(
        &self,
        session: i64,
        now: i64,
    ) -> Result<(), StorageError> {
        let rows=self.db.execute("SELECT DISTINCT g.task_id FROM goal_workers g JOIN scheduled_runs r ON r.task_id=g.task_id WHERE g.session_id=? AND r.status IN ('queued','claimed') AND r.queued_id IS NOT NULL", &[DbValue::Int(session)]).await?;
        for row in rows.rows {
            let task = row.get_int(0)?;
            self.cancel_scheduled_pending(task).await?;
            self.db
                .execute(
                    "UPDATE scheduled_tasks SET next_run=? WHERE id=? AND enabled=1",
                    &[DbValue::Int(now + 1), DbValue::Int(task)],
                )
                .await?;
        }
        Ok(())
    }
    pub(super) async fn pause_removed_goal_prompt(
        &self,
        user: UserId,
        session: i64,
        key: openwebide_core::QueuedPromptKey,
    ) -> Result<(), StorageError> {
        let rows=self.db.execute("SELECT g.task_id FROM goal_workers g JOIN scheduled_runs r ON r.task_id=g.task_id JOIN queued_prompts q ON q.id=r.queued_id WHERE g.session_id=? AND q.id=? AND q.revision=?", &[DbValue::Int(session),DbValue::Int(key.id),DbValue::Int(key.revision)]).await?;
        if let Some(row) = rows.rows.first()
            && let Some(mut goal) = self.get_goal(user, session).await?
        {
            goal.status = openwebide_core::GoalStatus::Paused;
            goal.revision = goal
                .revision
                .checked_add(1)
                .ok_or_else(|| StorageError::InvalidValue("Goal revision exhausted".into()))?;
            goal.note = Some("Queued goal prompt removed. Continue when ready.".into());
            self.save_goal(&goal).await?;
            self.db
                .execute(
                    "UPDATE scheduled_tasks SET enabled=0,next_run=NULL WHERE id=?",
                    &[DbValue::Int(row.get_int(0)?)],
                )
                .await?;
        }
        Ok(())
    }
    /// Goal cancellation uses lifecycle revisions, avoiding wall-clock races on pause/resume.
    pub async fn goal_run_cancelled(
        &self,
        user: UserId,
        session: i64,
        token: &str,
    ) -> Result<bool, StorageError> {
        self.get_session(session, user).await?;
        let rows=self.db.execute("SELECT r.goal_revision FROM goal_workers g JOIN scheduled_runs r ON r.task_id=g.task_id WHERE g.session_id=? AND ('scheduled-' || r.id)=?", &[DbValue::Int(session),DbValue::Text(token.into())]).await?;
        let Some(row) = rows.rows.first() else {
            return Ok(false);
        };
        let goal = self.get_goal(user, session).await?;
        Ok(goal.is_none_or(|goal| {
            goal.status != openwebide_core::GoalStatus::Active
                || i64::try_from(goal.revision).ok() != row.get_int_opt(0)
        }))
    }
    pub async fn goal_run_context(
        &self,
        host: &str,
        run: i64,
    ) -> Result<Option<(UserId, openwebide_core::Goal, i64)>, StorageError> {
        let rows=self.db.execute("SELECT t.user_id,g.session_id,r.message_id FROM goal_workers g JOIN scheduled_tasks t ON t.id=g.task_id JOIN scheduled_runs r ON r.task_id=t.id WHERE t.host_id=? AND r.id=? AND r.goal_revision=g.revision AND r.message_id IS NOT NULL AND r.status IN ('claimed','running','blocked')", &[DbValue::Text(host.into()),DbValue::Int(run)]).await?;
        let Some(row) = rows.rows.first() else {
            return Ok(None);
        };
        let user = UserId::new(row.get_int(0)?);
        let goal = self.get_goal(user, row.get_int(1)?).await?;
        goal.filter(|goal| goal.status == openwebide_core::GoalStatus::Active)
            .map(|goal| Ok((user, goal, row.get_int(2)?)))
            .transpose()
    }
    pub(super) async fn apply_goal_result(
        &self,
        host: &str,
        result: &openwebide_core::scheduled::DispatchResult,
        assessment: Option<&GoalTurnAssessment>,
        now: i64,
    ) -> Result<(), StorageError> {
        use openwebide_core::{
            GoalStatus,
            goal::GoalVerdict,
            scheduled::{Schedule, TaskDraft},
        };
        let rows=self.db.execute("SELECT t.user_id,g.session_id,g.task_id,g.revision,g.turns,g.stalls,t.draft FROM goal_workers g JOIN scheduled_tasks t ON t.id=g.task_id JOIN scheduled_runs r ON r.task_id=t.id WHERE t.host_id=? AND r.id=? AND r.goal_revision=g.revision", &[DbValue::Text(host.into()),DbValue::Int(result.run_id)]).await?;
        let Some(row) = rows.rows.first() else {
            return Ok(());
        };
        let user = UserId::new(row.get_int(0)?);
        let session = row.get_int(1)?;
        let task = row.get_int(2)?;
        let Some(mut goal) = self.get_goal(user, session).await? else {
            return Ok(());
        };
        if goal.status != GoalStatus::Active
            || i64::try_from(goal.revision).ok() != Some(row.get_int(3)?)
        {
            return Ok(());
        }
        if matches!(result.status.as_str(), "queued" | "running" | "blocked") {
            return Ok(());
        }
        let turns = row.get_int(4)? + 1;
        let stalls = if assessment.is_some_and(|a| a.used_tools) {
            0
        } else {
            row.get_int(5)? + 1
        };
        if result.status == "complete" {
            if let Some(assessment) = assessment {
                let latest = self
                    .db
                    .execute(
                        "SELECT max(id) FROM messages WHERE session_id=?",
                        &[DbValue::Int(session)],
                    )
                    .await?;
                if latest.rows.first().and_then(|row| row.get_int_opt(0))
                    != Some(assessment.last_message)
                {
                    // A user turn raced evaluation. Let that turn finish before reassessing.
                    self.db
                        .execute(
                            "UPDATE scheduled_tasks SET next_run=? WHERE id=?",
                            &[DbValue::Int(now + 1), DbValue::Int(task)],
                        )
                        .await?;
                    return Ok(());
                }
                goal.note = Some(assessment.evaluation.reason.clone());
                goal.status = match assessment.evaluation.verdict {
                    GoalVerdict::Complete => GoalStatus::Completed,
                    GoalVerdict::Blocked => GoalStatus::Paused,
                    GoalVerdict::Continue => GoalStatus::Active,
                };
            } else {
                goal.status = GoalStatus::Paused;
                goal.note = Some(
                    "Goal evaluation unavailable. Review the result and resume to retry.".into(),
                );
            }
            if goal.status == GoalStatus::Active && (stalls >= 3 || turns >= 100) {
                goal.status = GoalStatus::Paused;
                goal.note=Some(if stalls>=3 {"Paused after three turns without tool use. Review progress before resuming."} else {"Paused after 100 turns. Review progress before resuming."}.into());
            }
        } else {
            goal.status = GoalStatus::Paused;
            goal.note = Some(result.detail.clone());
        }
        if goal.status == GoalStatus::Active {
            let mut draft: TaskDraft = serde_json::from_str(row.get_text(6)?)
                .map_err(|e| StorageError::Db(e.to_string()))?;
            draft.schedule = Schedule::Once { at: now + 1 };
            draft.prompt = format!(
                "{}\n\nLatest goal evaluation: {}",
                goal.prompt(),
                goal.note
                    .as_deref()
                    .unwrap_or("Continue checking the objective.")
            );
            self.db
                .execute(
                    "UPDATE scheduled_tasks SET next_run=?,draft=? WHERE id=?",
                    &[
                        DbValue::Int(now + 1),
                        DbValue::Text(
                            serde_json::to_string(&draft)
                                .map_err(|e| StorageError::Db(e.to_string()))?,
                        ),
                        DbValue::Int(task),
                    ],
                )
                .await?;
        } else {
            goal.revision = goal
                .revision
                .checked_add(1)
                .ok_or_else(|| StorageError::InvalidValue("Goal revision exhausted".into()))?;
            self.db
                .execute(
                    "UPDATE scheduled_tasks SET enabled=0,next_run=NULL WHERE id=?",
                    &[DbValue::Int(task)],
                )
                .await?;
        }
        goal.updated_at = now;
        self.db
            .execute(
                "UPDATE goal_workers SET turns=?,stalls=? WHERE session_id=?",
                &[
                    DbValue::Int(turns),
                    DbValue::Int(stalls),
                    DbValue::Int(session),
                ],
            )
            .await?;
        self.save_goal(&goal).await?;
        Ok(())
    }
}

/// Evaluation is tied to an immutable conversation boundary, checked again at commit.
#[derive(Clone, Debug)]
pub struct GoalTurnAssessment {
    pub evaluation: openwebide_core::goal::GoalEvaluation,
    pub last_message: i64,
    pub used_tools: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use openwebide_core::{GoalCommand, GoalStatus};
    #[test]
    fn goals_and_manual_summaries_are_owned_atomic_and_durable_in_both_modes() {
        futures::executor::block_on(async {
            for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
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
                let project = store
                    .create_project(
                        &NewProject {
                            name: "p".into(),
                            mode,
                            path: Some("p".into()),
                        },
                        user,
                        1,
                    )
                    .await
                    .unwrap();
                let session = store
                    .create_session("chat", None, None, Some(project.id), user, 1)
                    .await
                    .unwrap()
                    .id;
                assert!(store.get_goal(user, session).await.unwrap().is_none());
                let goal = store
                    .update_goal(
                        user,
                        session,
                        0,
                        GoalCommand::Start {
                            objective: "Fix tests".into(),
                        },
                        1,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store.get_goal(user, session).await.unwrap(),
                    Some(goal.clone())
                );
                assert!(store.get_goal(other, session).await.is_err());
                assert!(
                    store
                        .update_goal(other, session, 1, GoalCommand::Pause, 2)
                        .await
                        .is_err()
                );
                assert!(matches!(
                    store
                        .update_goal(user, session, 0, GoalCommand::Complete, 2)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                let paused = store
                    .update_goal(user, session, 1, GoalCommand::Pause, 2)
                    .await
                    .unwrap();
                assert_eq!(paused.status, GoalStatus::Paused);
                let resumed = store
                    .update_goal(user, session, 2, GoalCommand::Resume, 3)
                    .await
                    .unwrap();
                assert_eq!(resumed.objective, goal.objective);
                let complete = store
                    .update_goal(user, session, 3, GoalCommand::Complete, 4)
                    .await
                    .unwrap();
                assert_eq!(complete.status, GoalStatus::Completed);
                assert_eq!(complete.started_at, Some(1));
                assert_eq!(
                    store
                        .get_goal(user, session)
                        .await
                        .unwrap()
                        .unwrap()
                        .completed_duration_seconds(),
                    Some(3)
                );
                let original = store
                    .insert_message(session, Role::User, "original prompt", 1)
                    .await
                    .unwrap();
                let summary = openwebide_core::Compaction {
                    summary: "Progress saved".into(),
                    retained: vec![original.clone()],
                    through_message_id: 0,
                };
                assert!(
                    store
                        .save_manual_compaction(other, session, original.id, &summary, 0, 2)
                        .await
                        .is_err()
                );
                let later = store
                    .insert_message(session, Role::Assistant, "answer", 2)
                    .await
                    .unwrap();
                assert!(matches!(
                    store
                        .save_manual_compaction(user, session, original.id, &summary, 0, 3)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                store.request_cancel(session, 10).await.unwrap();
                assert!(
                    store
                        .save_manual_compaction(user, session, later.id, &summary, 9, 3)
                        .await
                        .is_err()
                );
                let saved = store
                    .save_manual_compaction(user, session, later.id, &summary, 11, 3)
                    .await
                    .unwrap();
                assert_eq!(
                    openwebide_core::Compaction::parse(&saved.content)
                        .unwrap()
                        .through_message_id,
                    later.id
                );
                let messages = store.list_messages(session).await.unwrap();
                assert_eq!(messages[0], original);
                assert_eq!(messages[1], later);
                assert_eq!(messages.len(), 3);
                store.delete_session(session, user).await.unwrap();
                assert!(store.get_goal(user, session).await.is_err());
            }
        });
    }
}

#[cfg(test)]
#[path = "goal_worker_tests.rs"]
mod worker_tests;
