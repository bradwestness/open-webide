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
    let delivered =
        crate::push::dispatch(&state.store, &crate::push::SpinPushTransport, now()).await?;
    Ok(json_response(200, &json!({"processed":delivered})))
}
