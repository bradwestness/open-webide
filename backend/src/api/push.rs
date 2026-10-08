use super::*;
use openwebide_core::push::{
    PushConfig, PushEndpoint, PushStatus, PushSubscription, RunNotification,
};

pub(crate) async fn config(state: &AppState) -> Result<JsonResp, ApiError> {
    let key = crate::push::key_pair(&state.store).await?;
    Ok(json_response(
        200,
        &PushConfig {
            public_key: crate::push::public_key(&key),
        },
    ))
}
/// Return account identity and preference together so a worker cannot combine two accounts' responses.
pub(crate) async fn context(state: &AppState, user: AuthedUser) -> Result<JsonResp, ApiError> {
    let enabled = state
        .store
        .get_user_setting(user.id, "browser_notifications")
        .await?
        .as_deref()
        == Some("true");
    Ok(json_response(
        200,
        &json!({"user_id": user.id, "enabled": enabled}),
    ))
}

pub(crate) async fn subscribe(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let subscription: PushSubscription = parse_json(read_body(req, AUTH_BODY_LIMIT).await?)?;
    crate::push::subscription_key(&subscription)?;
    state
        .store
        .save_push_subscription(user.id, &subscription, now())
        .await?;
    Ok(json_response(200, &PushStatus { subscribed: true }))
}
pub(crate) async fn subscription(
    req: Request,
    state: &AppState,
    user: AuthedUser,
    remove: bool,
) -> Result<JsonResp, ApiError> {
    let endpoint: PushEndpoint = parse_json(read_body(req, AUTH_BODY_LIMIT).await?)?;
    if endpoint.endpoint.len() > 4096 {
        return Err(ApiError::bad_request("Push endpoint is too long"));
    }
    if remove {
        state
            .store
            .remove_push_subscription(user.id, &endpoint.endpoint)
            .await?;
    }
    Ok(json_response(
        200,
        &PushStatus {
            subscribed: state
                .store
                .has_push_subscription(user.id, &endpoint.endpoint)
                .await?,
        },
    ))
}
pub(crate) async fn notify(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let session = session_id(path)?;
    let event: RunNotification = parse_json(read_body(req, AUTH_BODY_LIMIT).await?)?;
    event.validate().map_err(ApiError::bad_request)?;
    state
        .store
        .queue_run_notification(user.id, session, &event, now())
        .await?;
    Ok(json_response(200, &json!({"ok":true})))
}
pub(crate) async fn dispatch(state: &AppState) -> Result<JsonResp, ApiError> {
    let delivered = crate::push::dispatch(
        &state.store,
        &CompletionPush {
            store: &state.store,
        },
        now(),
    )
    .await?;
    Ok(json_response(200, &json!({"processed":delivered})))
}

/// Transport adapter delegates completion policy to the shared evidence workflow.
struct CompletionPush<'a> {
    store: &'a openwebide_storage::Store<crate::state::AppDb>,
}
impl crate::push::PushTransport for CompletionPush<'_> {
    async fn summary(&self, user: UserId, session: i64, message: i64) -> Option<String> {
        let summary = super::completion::summary(self.store, user, session, Some(message)).await?;
        let session = self.store.get_session(session, user).await.ok()?;
        Some(format!(
            "{} — {summary}",
            session.name.chars().take(80).collect::<String>()
        ))
    }
    async fn send(&self, request: spin_sdk::http::Request<Vec<u8>>) -> Result<u16, String> {
        crate::push::PushTransport::send(&crate::push::SpinPushTransport, request).await
    }
}
