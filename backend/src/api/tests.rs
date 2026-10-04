#[test]
fn settings_body_limit_accepts_maximum_prompt_history() {
    for character in ['😀', '\0'] {
        let history: Vec<String> = (0..200)
            .map(|i| {
                let prefix = format!("{i:03}");
                prefix + &character.to_string().repeat(1997)
            })
            .collect();
        let value = serde_json::to_string(&history).unwrap();
        let body = serde_json::to_vec(&serde_json::json!({
            "key": "prompt_history",
            "value": value,
        }))
        .unwrap();
        assert!(body.len() <= super::SETTINGS_BODY_LIMIT);
    }
}

use super::auth::*;
use super::bridge::*;
use super::files::*;
use super::projects::*;
use super::sessions::*;
use super::*;

#[test]
fn run_plan_rebuilds_tool_history_and_falls_back_across_gaps() {
    futures::executor::block_on(async {
        let store = openwebide_storage::Store::new(crate::state::AppDb::open_in_memory().unwrap());
        store.migrate().await.unwrap();
        let user = store
            .insert_user("u", "hash", openwebide_core::UserRole::Admin, 1)
            .await
            .unwrap();
        let connection = store
            .insert_connection(&NewConnection {
                name: "server".into(),
                kind: openwebide_core::ProviderKind::LlamaCpp,
                base_url: "http://server".into(),
                model: None,
                context_limit: None,
            })
            .await
            .unwrap();
        let state = AppState { store };
        for (row_count, complete_second) in [(2, true), (2, false), (1, true)] {
            let session = state
                .store
                .create_session("s", Some(connection.id), None, None, user.id, 1)
                .await
                .unwrap();
            let calls: Vec<_> = (0..2)
                .map(|i| openwebide_core::ToolCall {
                    id: format!("wire-{i}"),
                    name: "read_file".into(),
                    arguments: format!("{{\"path\":\"{i}\"}}"),
                })
                .collect();
            let interim = state
                .store
                .insert_interim_message(
                    session.id,
                    Role::Assistant,
                    "checking",
                    1,
                    None,
                    Some(&calls),
                )
                .await
                .unwrap();
            for i in 0..row_count {
                let id = format!("step-{i}");
                state
                    .store
                    .upsert_tool_step(session.id, interim.id, &id, "read_file", "read", 1, None)
                    .await
                    .unwrap();
                if i == 0 || complete_second {
                    state
                        .store
                        .complete_tool_step(
                            user.id,
                            session.id,
                            &id,
                            true,
                            &format!("result-{i}"),
                            None,
                        )
                        .await
                        .unwrap();
                }
            }
            let plan = build_run_plan(
                &state,
                user.id,
                session.id,
                SendMessageBody {
                    content: "next".into(),
                    model: None,
                    editor_context: None,
                    queued_prompt: None,
                },
            )
            .await
            .unwrap();
            let history = plan.request.messages;
            assert_eq!(history[0].content, "checking");
            if row_count == 1 {
                assert_eq!(history.len(), 1);
                assert!(history[0].tool_calls.is_none());
            } else {
                assert_eq!(history.len(), 3);
                assert_eq!(history[0].tool_calls.as_ref(), Some(&calls));
                for i in 0..2 {
                    assert_eq!(history[i + 1].role, Role::Tool);
                    assert_eq!(
                        history[i + 1].tool_call_id.as_deref(),
                        Some(calls[i].id.as_str())
                    );
                    assert_eq!(
                        history[i + 1].content,
                        if i == 1 && !complete_second {
                            "result not recorded".into()
                        } else {
                            format!("result-{i}")
                        }
                    );
                }
            }
        }
    });
}

#[derive(Clone, Default)]
struct MemoHttp(std::sync::Arc<std::sync::Mutex<Vec<bool>>>);

impl openwebide_llm::HttpClient for MemoHttp {
    async fn get_json(&self, _: &str) -> Result<serde_json::Value, openwebide_llm::ProviderError> {
        unreachable!()
    }
    async fn post_json(
        &self,
        _: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, openwebide_llm::ProviderError> {
        self.0
            .lock()
            .unwrap()
            .push(body["stream"].as_bool().unwrap());
        Ok(json!({"choices":[{"message":{"content":"done"}}]}))
    }
    fn post_stream(
        &self,
        _: &str,
        _: &serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn futures::Stream<Item = Result<Bytes, openwebide_llm::ProviderError>>
                + Send
                + 'static,
        >,
    > {
        self.0.lock().unwrap().push(true);
        Box::pin(futures::stream::empty())
    }
}

#[test]
fn persisted_memo_skips_streaming() {
    futures::executor::block_on(async {
        let connection = openwebide_core::Connection {
            id: 1,
            name: "server".into(),
            kind: openwebide_core::ProviderKind::LlamaCpp,
            base_url: "http://server".into(),
            model: None,
            enabled: true,
            context_limit: None,
            tool_stream_unsupported: true,
            tool_stream_revision: 0,
        };
        let http = MemoHttp::default();
        let provider = Provider::for_connection(&connection, http.clone());
        let request = ChatRequest {
            model_settings: Default::default(),
            connection_id: 1,
            system_prompt: None,
            model: Some("model".into()),
            messages: vec![],
            tools: vec![],
        };
        let chunks = provider
            .chat_tools_stream(&request)
            .collect::<Vec<_>>()
            .await;
        assert!(chunks.iter().all(Result::is_ok));
        assert_eq!(*http.0.lock().unwrap(), vec![false]);
    });
}

#[test]
fn run_plan_prepares_chat_and_remote_agent_without_mutations() {
    futures::executor::block_on(async {
        let store = openwebide_storage::Store::new(crate::state::AppDb::open_in_memory().unwrap());
        store.migrate_with(&|_| true).await.unwrap();
        let user = store
            .insert_user("alice", "hash", openwebide_core::UserRole::Admin, 1)
            .await
            .unwrap();
        let other = store
            .insert_user("bob", "hash", openwebide_core::UserRole::User, 1)
            .await
            .unwrap();
        let connection = store
            .insert_connection(&NewConnection {
                name: "local".into(),
                kind: openwebide_core::ProviderKind::Ollama,
                base_url: "http://localhost:11434".into(),
                model: Some("model".into()),
                context_limit: None,
            })
            .await
            .unwrap();
        let prompt = store
            .insert_system_prompt("coder", "Be helpful")
            .await
            .unwrap();
        let project = store
            .create_project(
                &NewProject {
                    name: "app".into(),
                    mode: WorkspaceMode::Remote,
                    path: Some("repos/app".into()),
                },
                user.id,
                1,
            )
            .await
            .unwrap();
        let state = AppState { store };
        for project_id in [None, Some(project.id)] {
            let session = state
                .store
                .create_session(
                    "s",
                    Some(connection.id),
                    Some(prompt.id),
                    project_id,
                    user.id,
                    1,
                )
                .await
                .unwrap();
            let history = state
                .store
                .insert_message(session.id, Role::Assistant, "<think>r</think>earlier", 2)
                .await
                .unwrap();
            state.store.request_cancel(session.id, 1000).await.unwrap();
            let context = EditorContext {
                file_path: "src/main.rs".into(),
                cursor_line: 1,
                cursor_col: 1,
                selection: None,
            };
            let make_body = || SendMessageBody {
                content: "go".into(),
                model: Some("chosen".into()),
                editor_context: Some(context.clone()),
                queued_prompt: None,
            };
            let plan = build_run_plan(&state, user.id, session.id, make_body())
                .await
                .unwrap();
            assert_eq!(
                plan.user_content,
                if project_id.is_some() {
                    format!("{}go", context.format_prompt_injection())
                } else {
                    "go".into()
                }
            );
            let mut resolved_connection = connection.clone();
            resolved_connection.model = Some("chosen".into());
            assert_eq!(plan.connection, resolved_connection);
            assert_eq!(
                state.store.get_connection(connection.id).await.unwrap(),
                connection.clone()
            );
            assert_eq!(plan.request.connection_id, connection.id);
            assert_eq!(plan.request.model.as_deref(), Some("chosen"));
            assert!(
                plan.request
                    .system_prompt
                    .as_ref()
                    .unwrap()
                    .starts_with("Be helpful\n\nCurrent Date & Time: ")
            );
            let mut model_history = history.clone();
            model_history.content = "earlier".into();
            assert_eq!(plan.request.messages, vec![model_history]);
            if project_id.is_some() {
                assert_eq!(
                    plan.kind,
                    RunKind::Agent {
                        project_path: "repos/app".into()
                    }
                );
                assert_eq!(plan.request.tools, workspace_tools());
            } else {
                assert_eq!(plan.kind, RunKind::WebChat);
                assert_eq!(
                    plan.request.tools,
                    openwebide_agent::session::projectless_tools()
                );
            }
            assert_eq!(
                state.store.list_messages(session.id).await.unwrap(),
                vec![history]
            );
            assert!(
                state
                    .store
                    .cancel_requested_since(session.id, 0)
                    .await
                    .unwrap()
            );
            assert!(
                build_run_plan(&state, other.id, session.id, make_body())
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn test_health() {
    let resp = health();
    assert_eq!(resp.status(), 200);
}

#[test]
fn test_raw_headers() {
    let headers = raw_headers("logo.svg");
    assert!(headers.contains(&("x-content-type-options", "nosniff".to_string())));
    assert!(headers.contains(&("content-security-policy", "sandbox".to_string())));
    assert!(headers.contains(&("content-type", "image/svg+xml".to_string())));

    let headers = raw_headers("x.html");
    assert!(headers.contains(&("x-content-type-options", "nosniff".to_string())));
    assert!(headers.contains(&("content-security-policy", "sandbox".to_string())));
    assert!(headers.contains(&("content-type", "application/octet-stream".to_string())));
    assert!(headers.contains(&("content-disposition", "attachment".to_string())));

    let headers = raw_headers("a.png");
    assert!(headers.contains(&("content-security-policy", "sandbox".to_string())));
    assert!(headers.contains(&("content-type", "image/png".to_string())));
}

#[test]
fn test_normalize_project_path() {
    use openwebide_core::WorkspaceMode;
    assert_eq!(
        normalize_project_path(WorkspaceMode::Remote, Some("repos/x/".to_string()))
            .map_err(|_| ())
            .unwrap()
            .unwrap(),
        "repos/x"
    );
    assert_eq!(
        normalize_project_path(WorkspaceMode::Remote, Some("./a//b".to_string()))
            .map_err(|_| ())
            .unwrap()
            .unwrap(),
        "a/b"
    );
    assert!(normalize_project_path(WorkspaceMode::Remote, Some("../x".to_string())).is_err());
    assert_eq!(
        normalize_project_path(WorkspaceMode::Remote, Some("/".to_string()))
            .map_err(|_| ())
            .unwrap()
            .unwrap(),
        ""
    );
    assert_eq!(
        normalize_project_path(WorkspaceMode::Remote, Some("///".to_string()))
            .map_err(|_| ())
            .unwrap()
            .unwrap(),
        ""
    );
    assert_eq!(
        normalize_project_path(WorkspaceMode::Remote, Some("".to_string()))
            .map_err(|_| ())
            .unwrap()
            .unwrap(),
        ""
    );
}

#[test]
fn test_strip_base() {
    use openwebide_core::FileEntry;
    let e = vec![FileEntry {
        name: "main.rs".into(),
        path: "repos/x/src/main.rs".into(),
        is_dir: false,
        size: 0,
    }];
    let stripped = strip_base("repos/x/", e);
    assert_eq!(stripped[0].path, "src/main.rs");
}

#[test]
fn read_limited_enforces_byte_limit() {
    futures::executor::block_on(async {
        let limit = 10;
        // limit-1 and limit bytes are accepted.
        for n in [limit - 1, limit] {
            let body = http_body_util::Full::new(Bytes::from(vec![b'x'; n]));
            let text = super::read_limited(http_body_util::Limited::new(body, limit), limit)
                .await
                .unwrap();
            assert_eq!(text, "x".repeat(n));
        }
        // limit+1 bytes is refused with 413, not a generic 400.
        let body = http_body_util::Full::new(Bytes::from(vec![b'x'; limit + 1]));
        let err = super::read_limited(http_body_util::Limited::new(body, limit), limit)
            .await
            .unwrap_err();
        assert_eq!(err.into_response().status().as_u16(), 413);
    });
}

#[test]
fn read_limited_rejects_non_utf8() {
    futures::executor::block_on(async {
        let body = http_body_util::Full::new(Bytes::from(vec![0xff, 0xfe, 0xfd]));
        let err = super::read_limited(http_body_util::Limited::new(body, 1024), 1024)
            .await
            .unwrap_err();
        assert_eq!(err.into_response().status().as_u16(), 400);
    });
}

#[test]
fn include_ignored_flag_parsing() {
    assert!(is_include_ignored(Some("1".into())));
    assert!(is_include_ignored(Some("true".into())));
    assert!(!is_include_ignored(None));
    assert!(!is_include_ignored(Some("0".into())));
    assert!(!is_include_ignored(Some("yes".into())));
    assert!(!is_include_ignored(Some("TRUE".into())));
    assert!(!is_include_ignored(Some("".into())));
}

#[test]
fn test_bridge_token() {
    use crate::state::{AppDb, AppState};
    use openwebide_core::UserRole;
    use openwebide_storage::Store;

    futures::executor::block_on(async {
        let db = AppDb::open_in_memory().unwrap();
        let store = Store::new(db);
        store.migrate().await.unwrap();
        let user_record = store
            .insert_user("u", "hash", UserRole::User, 1)
            .await
            .unwrap();
        let user = user_record.public();

        let state = AppState { store };

        // Without secret
        let err = bridge_token(&state, AuthedUser::from(user.clone()))
            .await
            .unwrap_err();
        assert_eq!(err.into_response().status().as_u16(), 503);

        // With secret
        let test_secret = "test-secret-12345678901234567890";
        state
            .store
            .set_setting("bridge_secret_cache", test_secret)
            .await
            .unwrap();

        let resp = bridge_token(&state, AuthedUser::from(user.clone()))
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);

        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        let token = json["token"].as_str().unwrap();
        let expires_at = json["expires_at"].as_i64().unwrap();

        let now = crate::state::now();
        assert!(expires_at > now);
        assert!(expires_at <= now + 120);

        let claims = openwebide_auth::verify_token_at(test_secret, token, now).unwrap();
        assert_eq!(claims.user_id, user.id.get());
    });
}

#[test]
fn project_database_error_does_not_downgrade_to_chat() {
    use openwebide_storage::db::Db;
    futures::executor::block_on(async {
        let path = std::env::temp_dir().join(format!(
            "openwebide-project-error-{}.sqlite",
            std::process::id()
        ));
        let db = crate::state::AppDb::open(&path).unwrap();
        let store = openwebide_storage::Store::new(db);
        store.migrate().await.unwrap();
        let user = store
            .insert_user("db-error", "hash", openwebide_core::UserRole::User, 1)
            .await
            .unwrap();
        let connection = store
            .insert_connection(&NewConnection {
                name: "server".into(),
                kind: openwebide_core::ProviderKind::LlamaCpp,
                base_url: "http://server".into(),
                model: None,
                context_limit: None,
            })
            .await
            .unwrap();
        let project = store
            .create_project(
                &NewProject {
                    name: "remote".into(),
                    mode: WorkspaceMode::Remote,
                    path: Some("repo".into()),
                },
                user.id,
                1,
            )
            .await
            .unwrap();
        let session = store
            .create_session("s", Some(connection.id), None, Some(project.id), user.id, 1)
            .await
            .unwrap();
        let state = AppState { store };
        let sabotage = crate::state::AppDb::open(&path).unwrap();
        sabotage
            .execute("ALTER TABLE projects RENAME TO unavailable_projects", &[])
            .await
            .unwrap();
        let result = build_run_plan(
            &state,
            user.id,
            session.id,
            SendMessageBody {
                content: "hello".into(),
                model: None,
                editor_context: None,
                queued_prompt: None,
            },
        )
        .await;
        drop(state);
        drop(sabotage);
        std::fs::remove_file(path).unwrap();
        let error = result.unwrap_err();
        assert!(error.to_string().contains("projects"));
        assert_eq!(error.into_response().status().as_u16(), 500);
    });
}

#[test]
fn pending_edit_handlers_and_resolution_body_contract() {
    futures::executor::block_on(async {
        let state = AppState::new().await.unwrap();
        let user = state
            .store
            .insert_user("owner", "hash", openwebide_core::UserRole::User, 1)
            .await
            .unwrap()
            .public();
        let other = state
            .store
            .insert_user("other", "hash", openwebide_core::UserRole::User, 1)
            .await
            .unwrap()
            .public();
        let project = state
            .store
            .create_project(
                &NewProject {
                    name: "p".into(),
                    mode: WorkspaceMode::Remote,
                    path: None,
                },
                user.id,
                1,
            )
            .await
            .unwrap();
        let session = state
            .store
            .create_session("s", None, None, Some(project.id), user.id, 1)
            .await
            .unwrap();
        let diff = FileDiff {
            path: "nested/a.txt".into(),
            old: None,
            new: "new".into(),
            old_unavailable: false,
            backup_path: None,
        };
        state
            .store
            .upsert_tool_step(session.id, 1, "source", "write_file", "write", 1, None)
            .await
            .unwrap();
        state
            .store
            .complete_tool_step(user.id, session.id, "source", true, "written", Some(&diff))
            .await
            .unwrap();
        let path = format!("/api/projects/{}/pending-edits", project.id);
        let response = list_pending_edits(&state, &path, AuthedUser::from(user.clone()))
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let edits: Vec<openwebide_core::PersistedEdit> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(edits[0].diff, diff);
        assert!(
            list_pending_edits(&state, &path, AuthedUser::from(other))
                .await
                .is_err()
        );
        assert!(
            list_pending_edits(
                &state,
                &(path.clone() + "/extra"),
                AuthedUser::from(user.clone())
            )
            .await
            .is_err()
        );
        for body in [
            r#"{"path":"a","revision":"bad","decision":"accepted"}"#,
            r#"{"path":"a","revision":1,"decision":"unknown"}"#,
            r#"{"path":"a","decision":"rejected"}"#,
        ] {
            assert!(parse_json::<openwebide_core::ResolveEditRequest>(body.into()).is_err());
        }
        let request: openwebide_core::ResolveEditRequest =
            parse_json(r#"{"path":"nested/a.txt","revision":1,"decision":"rejected"}"#.into())
                .unwrap();
        let resolved = state
            .store
            .resolve_pending_edit(user.id, project.id, &request)
            .await
            .unwrap();
        assert_eq!(resolved.decision, openwebide_core::EditDecision::Rejected);
        assert!(
            state
                .store
                .list_pending_edits(user.id, project.id)
                .await
                .unwrap()
                .is_empty()
        );
    });
}

#[test]
fn resolution_validation_rejects_pending_and_nonpositive_revisions() {
    use openwebide_core::{EditDecision, ResolveEditRequest};
    for revision in [-1, 0, 1] {
        for decision in [
            EditDecision::Pending,
            EditDecision::Accepted,
            EditDecision::Rejected,
        ] {
            let request = ResolveEditRequest {
                path: "a".into(),
                revision,
                decision,
            };
            assert_eq!(
                validate_edit_resolution(&request).is_ok(),
                revision > 0 && decision != EditDecision::Pending
            );
        }
    }
}

#[test]
fn browser_runtime_scrubs_secrets_and_native_runtime_keeps_them() {
    futures::executor::block_on(async {
        let store = openwebide_storage::Store::new(crate::state::AppDb::open_in_memory().unwrap());
        store.migrate().await.unwrap();
        let user = store
            .insert_user("u", "hash", openwebide_core::UserRole::Admin, 1)
            .await
            .unwrap();
        let connection = store
            .insert_connection(&NewConnection {
                name: "server".into(),
                kind: openwebide_core::ProviderKind::Ollama,
                base_url: "http://localhost:11434".into(),
                model: Some("main".into()),
                context_limit: None,
            })
            .await
            .unwrap();
        store
            .save_server_settings(
                connection.id,
                &openwebide_core::ServerSettingsUpdate {
                    api_key: Some("test-secret".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let state = AppState { store };
        for native in [false, true] {
            let runtime = super::model_setup::runtime(&state, user.id, connection.id, None)
                .await
                .unwrap();
            let response = super::model_setup::runtime_response(runtime, native);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let runtime: openwebide_core::ModelRuntime = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                runtime.transport.api_key.as_deref(),
                native.then_some("test-secret")
            );
            assert_eq!(runtime.settings.auto_compact_threshold, Some(85));
            assert_eq!(runtime.settings.fast.unwrap().model, "main");
        }
    });
}

#[test]
fn approval_modes_are_user_scoped_and_edit_mode_never_approves_commands() {
    futures::executor::block_on(async {
        use openwebide_core::{ApprovalCheck, ApprovalMode, ToolCall};
        let store = openwebide_storage::Store::new(crate::state::AppDb::open_in_memory().unwrap());
        store.migrate().await.unwrap();
        let user = store
            .insert_user("alice", "hash", openwebide_core::UserRole::Admin, 1)
            .await
            .unwrap();
        let other = store
            .insert_user("bob", "hash", openwebide_core::UserRole::User, 1)
            .await
            .unwrap();
        let session = store
            .create_session("s", None, None, None, user.id, 1)
            .await
            .unwrap();
        for (mode, write, command) in [
            (ApprovalMode::Default, false, false),
            (ApprovalMode::AutoAcceptEdits, true, false),
            (ApprovalMode::Yolo, true, true),
        ] {
            store
                .set_user_setting(
                    user.id,
                    &ApprovalMode::setting_key(session.id),
                    &serde_json::to_string(&mode).unwrap(),
                )
                .await
                .unwrap();
            for (tool, expected) in [("write_file", write), ("run_command", command)] {
                let check = ApprovalCheck {
                    connection_id: 1,
                    model: None,
                    call: ToolCall {
                        id: "a1t1c0".into(),
                        name: tool.into(),
                        arguments: "{}".into(),
                    },
                };
                assert_eq!(
                    super::approvals::decision(&store, user.id, session.id, &check)
                        .await
                        .unwrap()
                        .approved,
                    expected
                );
                assert!(
                    super::approvals::decision(&store, other.id, session.id, &check)
                        .await
                        .is_err()
                );
            }
        }
    });
}

#[test]
fn auto_classifier_uses_primary_without_fast_model_and_respects_fast_override() {
    futures::executor::block_on(async {
        use openwebide_core::{ApprovalCheck, ModelDefaults, ModelSelection, ToolCall};
        let store = openwebide_storage::Store::new(crate::state::AppDb::open_in_memory().unwrap());
        store.migrate().await.unwrap();
        let user = store
            .insert_user("u", "hash", openwebide_core::UserRole::Admin, 1)
            .await
            .unwrap();
        let server = store
            .insert_connection(&NewConnection {
                name: "server".into(),
                kind: openwebide_core::ProviderKind::Ollama,
                base_url: "http://localhost:11434".into(),
                model: Some("main".into()),
                context_limit: None,
            })
            .await
            .unwrap();
        let session = store
            .create_session("s", Some(server.id), None, None, user.id, 1)
            .await
            .unwrap();
        store
            .insert_message(session.id, Role::User, "Update the README", 2)
            .await
            .unwrap();
        let check = ApprovalCheck {
            connection_id: server.id,
            model: Some("selected-main".into()),
            call: ToolCall {
                id: "a1t1c0".into(),
                name: "write_file".into(),
                arguments: r#"{"path":"README.md","content":"updated"}"#.into(),
            },
        };
        let (_, request) = super::approvals::classifier_plan(&store, user.id, session.id, &check)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.model.as_deref(), Some("selected-main"));
        assert!(request.messages[0].content.contains("Update the README"));
        store
            .save_model_defaults(
                user.id,
                &ModelDefaults {
                    fast: Some(ModelSelection {
                        server_id: server.id,
                        model: "fast".into(),
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let (_, request) = super::approvals::classifier_plan(&store, user.id, session.id, &check)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.model.as_deref(), Some("fast"));
        assert_eq!(request.model_settings.tools, Some(false));
        assert!(request.tools.is_empty());
    });
}

#[test]
fn project_file_paths_normalize_before_joining_and_do_not_escape_project() {
    futures::executor::block_on(async {
        let store = openwebide_storage::Store::new(crate::state::AppDb::open_in_memory().unwrap());
        store.migrate_with(&|_| true).await.unwrap();
        let user = store
            .insert_user("paths", "hash", openwebide_core::UserRole::Admin, 1)
            .await
            .unwrap();
        let project = store
            .create_project(
                &NewProject {
                    name: "app".into(),
                    mode: WorkspaceMode::Remote,
                    path: Some("repos/app".into()),
                },
                user.id,
                1,
            )
            .await
            .unwrap();
        let state = AppState { store };
        assert_eq!(
            remote_project_path(&state, user.id, project.id, "\\src\\.\\a.rs")
                .await
                .unwrap()
                .0,
            "repos/app/src/a.rs"
        );
        for path in ["../other", "src/../../other", ".spin/db", "a/.spin/db"] {
            assert_eq!(
                remote_project_path(&state, user.id, project.id, path)
                    .await
                    .unwrap_err()
                    .into_response()
                    .status()
                    .as_u16(),
                400
            );
        }
    });
}

#[test]
fn empty_search_queries_return_400_before_filesystem_access() {
    for value in [None, Some(String::new()), Some(" \t\n".into())] {
        let error = super::files::search_query(value).unwrap_err();
        assert_eq!(error.into_response().status().as_u16(), 400);
    }
    assert_eq!(
        super::files::search_query(Some("hello".into())).unwrap(),
        "hello"
    );
}

#[test]
fn todo_endpoints_share_owned_validation_and_durability_in_every_workspace() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            let state = AppState::new().await.unwrap();
            let owner = state
                .store
                .insert_user("owner", "hash", openwebide_core::UserRole::Admin, 1)
                .await
                .unwrap();
            let other = state
                .store
                .insert_user("other", "hash", openwebide_core::UserRole::User, 1)
                .await
                .unwrap();
            let project = if let Some(mode) = mode {
                Some(
                    state
                        .store
                        .create_project(
                            &NewProject {
                                name: "project".into(),
                                mode,
                                path: Some("test".into()),
                            },
                            owner.id,
                            1,
                        )
                        .await
                        .unwrap()
                        .id,
                )
            } else {
                None
            };
            let session = state
                .store
                .create_session("session", None, None, project, owner.id, 1)
                .await
                .unwrap()
                .id;
            let prompt = state
                .store
                .insert_message(session, Role::User, "task", 1)
                .await
                .unwrap();
            let user = AuthedUser {
                id: owner.id,
                role: owner.role,
            };
            let other = AuthedUser {
                id: other.id,
                role: other.role,
            };
            let path = format!("/api/sessions/{session}/todos");
            let plan = openwebide_core::TodoPlan {
                todos: vec![openwebide_core::TodoItem {
                    id: "inspect".into(),
                    content: "Inspect the code".into(),
                    status: openwebide_core::TodoStatus::Pending,
                }],
            };
            let request = |anchor, plan: &openwebide_core::TodoPlan| TodoPlanBody {
                anchor_message_id: anchor,
                plan: plan.clone(),
            };
            assert_eq!(
                get_todo_plan(&state, &path, user).await.unwrap().status(),
                200
            );
            assert_eq!(
                write_todo_plan_body(&state, session, user, request(prompt.id, &plan))
                    .await
                    .unwrap()
                    .status(),
                201
            );
            let response = get_todo_plan(&state, &path, user)
                .await
                .unwrap()
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes();
            let persisted: openwebide_core::TodoUpdate = serde_json::from_slice(&response).unwrap();
            assert_eq!(persisted.plan, plan);
            assert_eq!(persisted.anchor_message_id, prompt.id);
            assert_eq!(
                get_todo_plan(&state, &path, other)
                    .await
                    .unwrap_err()
                    .into_response()
                    .status(),
                404
            );
            assert_eq!(
                write_todo_plan_body(&state, session, other, request(prompt.id, &plan))
                    .await
                    .unwrap_err()
                    .into_response()
                    .status(),
                404
            );
            let invalid = openwebide_core::TodoPlan {
                todos: vec![openwebide_core::TodoItem {
                    id: "bad".into(),
                    content: String::new(),
                    status: openwebide_core::TodoStatus::Pending,
                }],
            };
            assert_eq!(
                write_todo_plan_body(&state, session, user, request(prompt.id, &invalid))
                    .await
                    .unwrap_err()
                    .into_response()
                    .status(),
                400
            );
            state
                .store
                .insert_message(session, Role::User, "new task", 2)
                .await
                .unwrap();
            assert_eq!(
                write_todo_plan_body(&state, session, user, request(prompt.id, &plan))
                    .await
                    .unwrap_err()
                    .into_response()
                    .status(),
                409
            );
            assert_eq!(
                state.store.get_todo_plan(user.id, session).await.unwrap(),
                Some(persisted)
            );
        }
    });
}
