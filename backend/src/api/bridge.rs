use super::*;
pub(crate) async fn bridge_token(state: &AppState, user: AuthedUser) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let secret_opt = crate::bridge::bridge_secret(&state.store).await?;
    let secret =
        secret_opt.ok_or_else(|| ApiError::new(503, "bridge secret not configured".to_string()))?;

    let now_ts = crate::state::now();
    let expires_at = now_ts + crate::bridge::BRIDGE_TOKEN_TTL_SECS;
    // Epoch is unused for bridge tokens.
    let token = openwebide_auth::sign_token_expires(&secret, user_id.get(), expires_at, 0);

    Ok(json_response(
        200,
        &json!({ "token": token, "expires_at": expires_at }),
    ))
}
