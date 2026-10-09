//! Shared skill persistence for UI, Spin, browser and bridge adapters.
use super::*;
use openwebide_core::{ProjectSkill, ProjectSkills, SkillCommand};
impl<D: Db> Store<D> {
    pub async fn project_skills(
        &self,
        user: UserId,
        project: i64,
    ) -> Result<ProjectSkills, StorageError> {
        self.get_project(project, user).await?;
        let enabled = self
            .get_user_setting(user, &format!("project_skills_{project}"))
            .await?
            .as_deref()
            != Some("false");
        let rows = self.db.execute("SELECT s.id,s.revision,s.updated_at,s.draft,p.prepared FROM project_skills s LEFT JOIN project_plugin_skills m ON m.skill_id=s.id LEFT JOIN project_plugins p ON p.id=m.plugin_id WHERE s.user_id = ? AND s.project_id = ? ORDER BY s.name,s.id", &[DbValue::Int(user.get()), DbValue::Int(project)]).await?;
        let entries = rows
            .rows
            .iter()
            .map(|row| {
                Ok(ProjectSkill {
                    plugin: row
                        .get_text_opt(4)
                        .map(|json| {
                            let prepared: openwebide_core::plugins::PreparedPlugin =
                                serde_json::from_str(json)
                                    .map_err(|error| StorageError::Db(error.to_string()))?;
                            Ok::<_, StorageError>(openwebide_core::plugins::PluginSkillOrigin {
                                publisher: prepared.manifest.publisher,
                                name: prepared.manifest.name,
                                version: prepared.manifest.version,
                                commit: prepared.source.commit,
                            })
                        })
                        .transpose()?,
                    id: row.get_int(0)?,
                    revision: row.get_int(1)?,
                    updated_at: row.get_int(2)?,
                    draft: serde_json::from_str(row.get_text(3)?)
                        .map_err(|e| StorageError::Db(e.to_string()))?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ProjectSkills { enabled, entries })
    }
    pub async fn session_skills(
        &self,
        user: UserId,
        session: i64,
    ) -> Result<ProjectSkills, StorageError> {
        match self.get_session(session, user).await?.project_id {
            Some(project) => self.project_skills(user, project).await,
            None => Ok(ProjectSkills {
                enabled: false,
                entries: Vec::new(),
            }),
        }
    }
    pub fn skill_command<'a>(
        &'a self,
        user: UserId,
        project: i64,
        command: &'a SkillCommand,
        agent: bool,
        now: i64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ProjectSkills, StorageError>> + Send + 'a>,
    > {
        Box::pin(async move {
            command.validate().map_err(StorageError::InvalidRequest)?;
            self.db.transaction(|tx| async move {
                let store = Store::new(tx);
                let current = store.project_skills(user, project).await?;
                if agent && (!current.enabled || matches!(command, SkillCommand::SetEnabled { .. })) {
                    return Err(StorageError::InvalidRequest("Project skills are disabled or this operation is unavailable to agents".into()));
                }
                let scope = [DbValue::Int(user.get()), DbValue::Int(project)];
                if let SkillCommand::Update {id,..} | SkillCommand::Delete {id,..} = command
                    && current.entries.iter().any(|entry|entry.id==*id && entry.plugin.is_some()) {
                    return Err(StorageError::InvalidRequest("Manage package skills through Settings → Plugins.".into()));
                }
                match command {
                    SkillCommand::Create { draft } | SkillCommand::Update { draft, .. } => {
                        if current.entries.iter().any(|entry| entry.draft.name == draft.name && !matches!(command, SkillCommand::Update { id, .. } if *id == entry.id)) {
                            return Err(StorageError::Conflict("A skill with this name already exists".into()));
                        }
                        let json = serde_json::to_string(draft).map_err(|e| StorageError::Db(e.to_string()))?;
                        if let SkillCommand::Update { id, revision, .. } = command {
                            if agent && current.entries.iter().any(|entry| entry.id == *id && !entry.draft.enabled) { return Err(StorageError::InvalidRequest("This skill is disabled".into())); }
                            let changed = store.db.execute("UPDATE project_skills SET name = ?, draft = ?, revision = revision + 1, updated_at = ? WHERE id = ? AND user_id = ? AND project_id = ? AND revision = ?", &[DbValue::Text(draft.name.clone()), DbValue::Text(json), DbValue::Int(now), DbValue::Int(*id), scope[0].clone(), scope[1].clone(), DbValue::Int(*revision)]).await?;
                            if changed.changes != 1 { return Err(StorageError::Conflict("Skill changed or was removed. Refresh before editing.".into())); }
                        } else {
                            if current.entries.len() >= openwebide_core::skills::MAX_SKILLS { return Err(StorageError::InvalidRequest("This project already has 100 skills".into())); }
                            store.db.execute("INSERT INTO project_skills (user_id, project_id, name, draft, updated_at) VALUES (?, ?, ?, ?, ?)", &[scope[0].clone(), scope[1].clone(), DbValue::Text(draft.name.clone()), DbValue::Text(json), DbValue::Int(now)]).await?;
                        }
                    }
                    SkillCommand::Delete { id, revision } => {
                        let changed = store.db.execute("DELETE FROM project_skills WHERE id = ? AND user_id = ? AND project_id = ? AND revision = ?", &[DbValue::Int(*id), scope[0].clone(), scope[1].clone(), DbValue::Int(*revision)]).await?;
                        if changed.changes != 1 { return Err(StorageError::Conflict("Skill changed or was removed. Refresh before deleting.".into())); }
                    }
                    SkillCommand::SetEnabled { enabled } => store.set_user_setting(user, &format!("project_skills_{project}"), if *enabled { "true" } else { "false" }).await?,
                    SkillCommand::List { query } => {
                        let query = query.to_lowercase();
                        return Ok(ProjectSkills { enabled: current.enabled, entries: current.entries.into_iter().filter(|entry| (!agent || entry.draft.enabled) && (entry.draft.name.to_lowercase().contains(&query) || entry.draft.description.to_lowercase().contains(&query))).collect() });
                    }
                    SkillCommand::Read { id, resource } => {
                        let entry = current.entries.into_iter().find(|entry| entry.id == *id && (!agent || entry.draft.enabled)).ok_or_else(|| StorageError::NotFound("skill".into()))?;
                        if resource.as_ref().is_some_and(|name| !entry.draft.resources.iter().any(|r| &r.name == name)) { return Err(StorageError::NotFound("skill resource".into())); }
                        return Ok(ProjectSkills { enabled: current.enabled, entries: vec![entry] });
                    }
                }
                store.project_skills(user, project).await
            }).await
        })
    }
    pub async fn session_skill_command(
        &self,
        user: UserId,
        session: i64,
        command: &SkillCommand,
        now: i64,
    ) -> Result<ProjectSkills, StorageError> {
        let project = self
            .get_session(session, user)
            .await?
            .project_id
            .ok_or_else(|| {
                StorageError::InvalidRequest("Project skills require a project".into())
            })?;
        self.skill_command(user, project, command, true, now).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use openwebide_core::{SkillDraft, SkillResource};
    fn draft(name: &str) -> SkillDraft {
        SkillDraft {
            name: name.into(),
            description: "Review Rust build changes".into(),
            instructions: "Run approved checks 🦀".into(),
            enabled: true,
            resources: vec![SkillResource {
                name: "references/build.md".into(),
                content: "cargo test".into(),
                binary: false,
            }],
            metadata: Default::default(),
        }
    }
    #[test]
    fn skills_database_contract_is_owned_shared_revision_safe_disabled_and_cascaded_in_both_modes()
    {
        futures::executor::block_on(async {
            for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
                store.migrate().await.unwrap();
                let user = store
                    .insert_user("owner", "hash", UserRole::Admin, 0)
                    .await
                    .unwrap()
                    .id;
                let other = store
                    .insert_user("other", "hash", UserRole::User, 0)
                    .await
                    .unwrap()
                    .id;
                let project = store
                    .create_project(
                        &NewProject {
                            name: "project".into(),
                            mode,
                            path: Some("test".into()),
                        },
                        user,
                        0,
                    )
                    .await
                    .unwrap()
                    .id;
                let second = store
                    .create_project(
                        &NewProject {
                            name: "second".into(),
                            mode,
                            path: Some("second".into()),
                        },
                        user,
                        0,
                    )
                    .await
                    .unwrap()
                    .id;
                let first = store
                    .create_session("first", None, None, Some(project), user, 0)
                    .await
                    .unwrap()
                    .id;
                let next = store
                    .create_session("next", None, None, Some(project), user, 0)
                    .await
                    .unwrap()
                    .id;
                let create = SkillCommand::Create {
                    draft: draft("build-check"),
                };
                let saved = store
                    .session_skill_command(user, first, &create, 1)
                    .await
                    .unwrap();
                let entry = &saved.entries[0];
                assert_eq!(store.session_skills(user, next).await.unwrap(), saved);
                assert!(store.project_skills(other, project).await.is_err());
                assert!(
                    store
                        .session_skill_command(other, first, &create, 1)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .session_skill_command(user, next, &create, 1)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .project_skills(user, second)
                        .await
                        .unwrap()
                        .entries
                        .is_empty()
                );
                assert_eq!(
                    store
                        .session_skill_command(
                            user,
                            next,
                            &SkillCommand::List {
                                query: "RUST".into()
                            },
                            2
                        )
                        .await
                        .unwrap()
                        .entries
                        .len(),
                    1
                );
                assert!(
                    store
                        .session_skill_command(
                            user,
                            next,
                            &SkillCommand::Read {
                                id: entry.id,
                                resource: Some("missing".into())
                            },
                            2
                        )
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .skill_command(
                            user,
                            second,
                            &SkillCommand::Delete {
                                id: entry.id,
                                revision: 1
                            },
                            false,
                            2
                        )
                        .await
                        .is_err()
                );
                let mut changed = draft("build-check");
                changed.instructions = "Updated instruction".into();
                let updated = store
                    .session_skill_command(
                        user,
                        next,
                        &SkillCommand::Update {
                            id: entry.id,
                            revision: 1,
                            draft: changed.clone(),
                        },
                        2,
                    )
                    .await
                    .unwrap();
                assert_eq!(updated.entries[0].revision, 2);
                assert!(matches!(
                    store
                        .session_skill_command(
                            user,
                            first,
                            &SkillCommand::Delete {
                                id: entry.id,
                                revision: 1
                            },
                            3
                        )
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                changed.enabled = false;
                store
                    .skill_command(
                        user,
                        project,
                        &SkillCommand::Update {
                            id: entry.id,
                            revision: 2,
                            draft: changed,
                        },
                        false,
                        3,
                    )
                    .await
                    .unwrap();
                assert!(
                    store
                        .session_skill_command(
                            user,
                            next,
                            &SkillCommand::List {
                                query: String::new()
                            },
                            3
                        )
                        .await
                        .unwrap()
                        .entries
                        .is_empty()
                );
                assert!(
                    store
                        .session_skill_command(
                            user,
                            next,
                            &SkillCommand::Read {
                                id: entry.id,
                                resource: None
                            },
                            3
                        )
                        .await
                        .is_err()
                );
                store
                    .skill_command(
                        user,
                        project,
                        &SkillCommand::SetEnabled { enabled: false },
                        false,
                        4,
                    )
                    .await
                    .unwrap();
                for command in [
                    &create,
                    &SkillCommand::List {
                        query: String::new(),
                    },
                    &SkillCommand::SetEnabled { enabled: true },
                ] {
                    assert!(
                        store
                            .session_skill_command(user, next, command, 4)
                            .await
                            .is_err()
                    );
                }
                assert_eq!(
                    store
                        .project_skills(user, project)
                        .await
                        .unwrap()
                        .entries
                        .len(),
                    1
                );
                store.delete_session(first, user).await.unwrap();
                assert_eq!(
                    store
                        .session_skills(user, next)
                        .await
                        .unwrap()
                        .entries
                        .len(),
                    1
                );
                store
                    .skill_command(
                        user,
                        project,
                        &SkillCommand::Delete {
                            id: entry.id,
                            revision: 3,
                        },
                        false,
                        5,
                    )
                    .await
                    .unwrap();
                for id in 0..openwebide_core::skills::MAX_SKILLS {
                    store
                        .skill_command(
                            user,
                            project,
                            &SkillCommand::Create {
                                draft: draft(&format!("skill-{id}")),
                            },
                            false,
                            6,
                        )
                        .await
                        .unwrap();
                }
                assert!(
                    store
                        .skill_command(user, project, &create, false, 7)
                        .await
                        .is_err()
                );
                store.delete_project(project, user).await.unwrap();
                assert_eq!(
                    store
                        .db
                        .execute("SELECT COUNT(*) FROM project_skills", &[])
                        .await
                        .unwrap()
                        .rows[0]
                        .get_int(0)
                        .unwrap(),
                    0
                );
            }
        });
    }
    #[test]
    fn skills_survive_reopening_the_database() {
        futures::executor::block_on(async {
            let path = std::env::temp_dir().join(format!(
                "openwebide-skill-persistence-{}.db",
                std::process::id()
            ));
            let store = Store::new(RusqliteDb::open(&path).unwrap());
            store.migrate().await.unwrap();
            let user = store
                .insert_user("owner", "hash", UserRole::Admin, 0)
                .await
                .unwrap()
                .id;
            let project = store
                .create_project(
                    &NewProject {
                        name: "project".into(),
                        mode: WorkspaceMode::Local,
                        path: None,
                    },
                    user,
                    0,
                )
                .await
                .unwrap()
                .id;
            let saved = store
                .skill_command(
                    user,
                    project,
                    &SkillCommand::Create {
                        draft: draft("build-check"),
                    },
                    false,
                    1,
                )
                .await
                .unwrap();
            drop(store);
            let reopened = Store::new(RusqliteDb::open(&path).unwrap());
            reopened.migrate().await.unwrap();
            assert_eq!(reopened.project_skills(user, project).await.unwrap(), saved);
            drop(reopened);
            std::fs::remove_file(path).unwrap();
        });
    }
}
