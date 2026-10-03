//! Shared approval service used by thin browser, bridge, and SSE adapters.
use super::*;
use crate::state::AppDb;
use openwebide_core::{ApprovalCheck, ApprovalDecision, ApprovalMode};
use openwebide_storage::Store;

pub(crate) async fn check(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let session = path
        .strip_prefix("/api/sessions/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| ApiError::bad_request("Expected a session id."))?;
    let check: ApprovalCheck = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    Ok(json_response(
        200,
        &decision(&state.store, user.id, session, &check).await?,
    ))
}

pub(crate) async fn decision(
    store: &Store<AppDb>,
    user: UserId,
    session: i64,
    check: &ApprovalCheck,
) -> Result<ApprovalDecision, ApiError> {
    store.get_session(session, user).await?;
    let mode = store
        .get_user_setting(user, &ApprovalMode::setting_key(session))
        .await?
        .and_then(|value| serde_json::from_str::<ApprovalMode>(&value).ok())
        .unwrap_or_default();
    if mode.auto_approves(&check.call.name) {
        return Ok(ApprovalDecision { approved: true });
    }
    if mode != ApprovalMode::Auto {
        return Ok(ApprovalDecision::default());
    }
    let Some((runtime, request)) = classifier_plan(store, user, session, check).await? else {
        return Ok(ApprovalDecision::default());
    };
    let mut transport = runtime.transport;
    transport.timeout_seconds = transport.timeout_seconds.min(15);
    let provider = Provider::for_connection(
        &runtime.connection,
        SpinHttpClient::default().with_transport(transport),
    );
    let classify = openwebide_agent::policy::classify(&provider, &request);
    let timeout = spin_sdk::time::sleep(std::time::Duration::from_secs(15));
    futures::pin_mut!(classify, timeout);
    let approved = match futures::future::select(classify, timeout).await {
        futures::future::Either::Left((approved, _)) => approved,
        futures::future::Either::Right(_) => false,
    };
    Ok(ApprovalDecision { approved })
}

pub(crate) async fn classifier_plan(
    store: &Store<AppDb>,
    user: UserId,
    session: i64,
    check: &ApprovalCheck,
) -> Result<Option<(openwebide_core::ModelRuntime, ChatRequest)>, ApiError> {
    store.get_session(session, user).await?;
    let primary =
        super::model_setup::runtime_store(store, user, check.connection_id, check.model.as_deref())
            .await?;
    let runtime = if let Some(fast) = &primary.settings.fast {
        super::model_setup::runtime_store(store, user, fast.server_id, Some(&fast.model)).await?
    } else {
        primary
    };
    let messages = store.list_messages(session).await?;
    let user_request = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(|message| message.content.as_str())
        .unwrap_or_default();
    let Some(request) = openwebide_agent::policy::classifier_request(check, &runtime, user_request)
    else {
        return Ok(None);
    };
    Ok(Some((runtime, request)))
}

pub(crate) struct ApprovalAdapter {
    pub store: Arc<Store<AppDb>>,
    pub user: UserId,
    pub session: i64,
    pub connection_id: i64,
    pub model: Option<String>,
}
impl openwebide_agent::policy::ApprovalSource for ApprovalAdapter {
    async fn check(&self, call: &openwebide_core::ToolCall) -> bool {
        decision(
            &self.store,
            self.user,
            self.session,
            &ApprovalCheck {
                connection_id: self.connection_id,
                model: self.model.clone(),
                call: call.clone(),
            },
        )
        .await
        .is_ok_and(|decision| decision.approved)
    }
}
