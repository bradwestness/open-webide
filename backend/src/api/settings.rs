use super::*;
// -- settings ------------------------------------------------------------------

pub(crate) async fn get_settings(state: &AppState, user: AuthedUser) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let settings = state.store.all_user_settings(user_id).await?;
    Ok(json_response(200, &settings))
}

#[derive(Deserialize)]
pub(super) struct SettingBody {
    key: String,
    value: String,
}

pub(crate) async fn set_setting(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let body = read_body(req, SETTINGS_BODY_LIMIT).await?;
    let setting: SettingBody = parse_json(body)?;
    state
        .store
        .set_user_setting(user_id, &setting.key, &setting.value)
        .await?;
    Ok(json_response(200, &json!({ "key": setting.key })))
}
