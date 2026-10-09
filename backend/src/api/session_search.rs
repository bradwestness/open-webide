//! Natural-language search with literal fallback and evidence-based explanations.
use super::*;
use openwebide_core::{AssistanceKind, AssistanceRequest, SessionSearch, SessionSearchResults};

pub(crate) async fn search(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let search: SessionSearch = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    Ok(json_response(
        200,
        &resolve(&state.store, user.id, &search).await?,
    ))
}
async fn resolve(
    store: &openwebide_storage::Store<crate::state::AppDb>,
    user: UserId,
    search: &SessionSearch,
) -> Result<SessionSearchResults, ApiError> {
    search.validate().map_err(ApiError::bad_request)?;
    let mut results = SessionSearchResults {
        sessions: store
            .search_sessions(user, search.project_id, &search.query, search.archived)
            .await?,
        ..Default::default()
    };
    let mut terms = vec![search.query.trim().to_owned()];
    if let Some(connection_id) = store
        .model_setup(user)
        .await?
        .defaults
        .primary
        .map(|selection| selection.server_id)
    {
        let request = AssistanceRequest {
            model: None,
            staged_draft: false,
            connection_id,
            project_id: search.project_id,
            session_id: None,
            kind: AssistanceKind::Search,
            input: search.query.clone(),
        };
        if let Ok(Some(query)) = super::assistance::execute(store, user, &request).await {
            terms.extend(
                query
                    .split_whitespace()
                    .map(|term| term.trim_matches(['"', '\'', ',', '.', ';']).to_owned())
                    .filter(|term| term.chars().count() >= 3)
                    .take(6),
            );
            results.rewritten_query = Some(query);
        }
    }
    for term in &terms {
        for session in store
            .search_sessions(user, search.project_id, term, search.archived)
            .await?
            .into_iter()
            .take(40)
        {
            results.explanations.entry(session.id).or_insert_with(|| {
                format!(
                    "Matches ‘{term}’ in {}.",
                    if session.name.to_lowercase().contains(&term.to_lowercase()) {
                        "the session name"
                    } else {
                        "conversation history"
                    }
                )
            });
            if results.sessions.len() < 80
                && !results
                    .sessions
                    .iter()
                    .any(|existing| existing.id == session.id)
            {
                results.sessions.push(session);
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{NewProject, UserRole};
    use openwebide_storage::{Store, rusqlite_db::RusqliteDb};
    #[test]
    fn literal_fallback_and_explanations_are_owned_in_every_scope() {
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
                    .create_session("Menu icons", None, None, project, owner, 1)
                    .await
                    .unwrap();
                let private = store
                    .create_session("Menu icons", None, None, None, other, 1)
                    .await
                    .unwrap();
                let query = SessionSearch {
                    project_id: project,
                    query: "icons".into(),
                    archived: false,
                };
                let results = resolve(&store, owner, &query).await.unwrap();
                assert_eq!(
                    results
                        .sessions
                        .iter()
                        .map(|session| session.id)
                        .collect::<Vec<_>>(),
                    [session.id]
                );
                assert!(results.explanations[&session.id].contains("session name"));
                assert!(!results.explanations.contains_key(&private.id));
                assert!(results.rewritten_query.is_none());
                store
                    .insert_message(session.id, Role::User, "help with colors", 2)
                    .await
                    .unwrap();
                let query = SessionSearch {
                    query: "colors".into(),
                    ..query
                };
                assert!(
                    resolve(&store, owner, &query).await.unwrap().explanations[&session.id]
                        .contains("conversation history")
                );
            }
        });
    }
}
