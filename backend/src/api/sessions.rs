use super::*;
// -- sessions --------------------------------------------------------------------

pub(crate) async fn list_sessions(
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let sessions = state.store.list_sessions(user_id).await?;
    Ok(json_response(200, &sessions))
}

#[derive(Deserialize)]
pub(super) struct CreateSessionBody {
    name: String,
    #[serde(default)]
    connection_id: Option<i64>,
    #[serde(default)]
    system_prompt_id: Option<i64>,
    #[serde(default)]
    project_id: Option<i64>,
}

pub(crate) async fn create_session(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let new: CreateSessionBody = parse_json(body)?;
    let session = state
        .store
        .create_session(
            &new.name,
            new.connection_id,
            new.system_prompt_id,
            new.project_id,
            user_id,
            now(),
        )
        .await?;
    Ok(json_response(201, &session))
}

#[derive(Deserialize)]
pub(super) struct RenameSessionBody {
    name: String,
}

pub(crate) async fn rename_session(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let rename: RenameSessionBody = parse_json(body)?;
    let session = state
        .store
        .rename_session(id, &rename.name, user_id)
        .await?;
    Ok(json_response(200, &session))
}

pub(crate) async fn delete_session(
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    state.store.delete_session(id, user_id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

/// Request cancellation of the session's in-flight run. The flag is picked
/// up by the streaming request at its next step boundary (before the next
/// model call or tool execution); a run that is not in flight is unaffected.
pub(crate) async fn cancel_session(
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    // Verify ownership before setting the flag.
    state.store.get_session(id, user_id).await?;
    state.store.request_cancel(id, now_ms()).await?;
    Ok(json_response(200, &json!({ "cancelled": id })))
}

#[derive(Deserialize)]
pub(super) struct PermissionBody {
    approved: bool,
}

/// Record the user's decision on a gated tool call. The waiting call consumes
/// it on its next poll, so it answers that call only; a decision for a call
/// that is no longer waiting is harmless (decisions are cleared when the run
/// ends).
pub(crate) async fn set_tool_permission(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let (id, tool_call_id) = permission_path(path)?;
    // Verify ownership before recording the decision.
    state.store.get_session(id, user_id).await?;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let decision: PermissionBody = parse_json(body)?;
    state
        .store
        .set_tool_permission(id, tool_call_id, decision.approved)
        .await?;
    Ok(json_response(200, &json!({ "ok": true })))
}

pub(crate) async fn list_messages(
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    // Verify ownership before exposing the (unscoped) message list.
    state.store.get_session(id, user_id).await?;
    // Messages plus the agent's tool steps, interleaved, so a reloaded session
    // shows its steps again.
    let conversation = state.store.list_conversation(id).await?;
    Ok(json_response(200, &conversation))
}

#[derive(Deserialize)]
pub(super) struct SendMessageBody {
    pub(super) content: String,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) editor_context: Option<EditorContext>,
}

pub(super) async fn build_run_plan(
    state: &AppState,
    user_id: UserId,
    session_id: i64,
    send: SendMessageBody,
) -> Result<RunPlan, ApiError> {
    let session = state.store.get_session(session_id, user_id).await?;
    let connection_id = session
        .connection_id
        .ok_or_else(|| ApiError::bad_request("session has no connection; pick one first"))?;
    let connection = state.store.get_connection(connection_id).await?;
    let system_prompt = match session.system_prompt_id {
        Some(id) => Some(state.store.get_system_prompt(id).await?.content),
        None => None,
    };
    let system_prompt = Some(with_temporal_context(system_prompt, now()));
    let mut history = openwebide_core::tool_history(
        state.store.list_messages(session_id).await?,
        &state.store.list_tool_steps(session_id).await?,
    );
    for message in &mut history {
        if message.role == Role::Assistant {
            message.content = openwebide_core::strip_reasoning(&message.content).to_string();
        }
    }
    let full_content = match &send.editor_context {
        Some(ctx) => format!("{}{}", ctx.format_prompt_injection(), send.content),
        None => send.content,
    };
    // Remote-mode projects run the agentic loop with workspace tools;
    // everything else is plain chat.
    let (is_remote, base) = match session.project_id {
        Some(id) => match state.store.get_project(id, user_id).await {
            Ok(project) if project.mode == WorkspaceMode::Remote => {
                (true, project.path.unwrap_or_default())
            }
            Ok(_) | Err(openwebide_storage::StorageError::NotFound(_)) => (false, String::new()),
            Err(error) => return Err(error.into()),
        },
        None => (false, String::new()),
    };

    Ok(RunPlan {
        user_content: full_content,
        request: ChatRequest {
            connection_id,
            system_prompt,
            model: send.model,
            messages: history,
            tools: if is_remote {
                workspace_tools()
            } else {
                Vec::new()
            },
        },
        connection,
        kind: if is_remote {
            RunKind::Agent { project_path: base }
        } else {
            RunKind::Chat
        },
    })
}

pub(crate) async fn run_plan(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let session_id = session_id(path)?;
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let send: SendMessageBody = parse_json(body)?;
    let plan = build_run_plan(state, user_id, session_id, send).await?;
    Ok(json_response(200, &plan))
}

/// Send a user message and stream the assistant reply back as SSE.
///
/// `state` is taken by value: the store is moved into the response body so
/// the assistant message can be persisted from inside the stream.
pub(crate) async fn send_session_message(
    req: Request,
    state: AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let started_ms = now_ms();
    let user_id = user.id;
    let session_id = session_id(path)?;
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let send: SendMessageBody = parse_json(body)?;

    let plan = build_run_plan(&state, user_id, session_id, send).await?;
    let user_message = state
        .store
        .insert_message(session_id, Role::User, &plan.user_content, now())
        .await?;
    let mut request = plan.request;
    request.messages.push(user_message.clone());
    let memo = ToolStreamMemo::new(plan.connection.tool_stream_unsupported);
    let connection_id = plan.connection.id;
    let tool_stream_revision = plan.connection.tool_stream_revision;
    let provider =
        Provider::for_connection_with_memo(&plan.connection, SpinHttpClient, memo.clone());
    let store = Arc::new(state.store);
    let cancel = CancelFlag::new(store.clone(), session_id, started_ms);
    let gate = PermissionPoller::new(store.clone(), session_id, started_ms);
    let memo_store = store.clone();
    let stream = if let RunKind::Agent { project_path } = plan.kind {
        agent_stream(
            store.clone(),
            session_id,
            user_message,
            request,
            provider,
            project_path,
            AgentConfig::default(),
            cancel,
            gate,
        )
    } else {
        message_stream(
            store,
            session_id,
            user_message,
            provider.chat_stream(&request),
            started_ms,
        )
    };

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(box_body(SseBody::new(Box::pin(stream.then(
            move |event| {
                let store = memo_store.clone();
                let memo = memo.clone();
                async move {
                    if memo.take_unrecorded()
                        && let Err(error) = store
                            .set_tool_stream_unsupported(connection_id, tool_stream_revision)
                            .await
                    {
                        eprintln!("session {session_id}: set_tool_stream_unsupported: {error}");
                    }
                    event
                }
            },
        )))))
        .expect("valid status and headers"))
}

/// Parse `/api/sessions/<id>...` into the session id.
/// Parse `/api/sessions/<id>/permissions/<tool_call_id>`.
#[derive(Deserialize)]
pub(super) struct PersistMessageBody {
    role: Role,
    content: String,
    #[serde(default)]
    usage: Option<TurnTelemetry>,
    #[serde(default)]
    tool_calls: Option<Vec<openwebide_core::ToolCall>>,
}

pub(crate) async fn persist_message(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    state.store.get_session(id, user_id).await?;
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let msg: PersistMessageBody = parse_json(body)?;
    let message = state
        .store
        .insert_interim_message(
            id,
            msg.role,
            &msg.content,
            now(),
            msg.usage.as_ref(),
            msg.tool_calls.as_deref(),
        )
        .await?;
    Ok(json_response(201, &message))
}

/// Resolve a connection's context window: the connection's configured value,
/// else provider discovery. A provider error surfaces as `null`, not a 5xx,
/// so an unreachable runtime doesn't raise a banner.
#[derive(Deserialize)]
pub(super) struct UpsertToolStepBody {
    anchor_message_id: i64,
    tool_call_id: String,
    name: String,
    summary: String,
    #[serde(default)]
    diff: Option<FileDiff>,
}

pub(crate) async fn upsert_tool_step(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    state.store.get_session(id, user_id).await?;
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let step: UpsertToolStepBody = parse_json(body)?;
    state
        .store
        .upsert_tool_step(
            id,
            step.anchor_message_id,
            &step.tool_call_id,
            &step.name,
            &step.summary,
            now(),
            step.diff.as_ref(),
        )
        .await?;
    Ok(json_response(200, &json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(super) struct CompleteToolStepBody {
    tool_call_id: String,
    ok: bool,
    result_summary: String,
    #[serde(default)]
    diff: Option<FileDiff>,
}

pub(crate) async fn complete_tool_step(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = session_id(path)?;
    state.store.get_session(id, user_id).await?;
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let step: CompleteToolStepBody = parse_json(body)?;
    state
        .store
        .complete_tool_step(
            id,
            &step.tool_call_id,
            step.ok,
            &step.result_summary,
            step.diff.as_ref(),
        )
        .await?;
    Ok(json_response(200, &json!({ "ok": true })))
}
