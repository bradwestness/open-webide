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

const SYSTEM_THEME: &str = "matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'";

pub(crate) async fn theme_script(
    state: &AppState,
    user: Option<AuthedUser>,
) -> Result<JsonResp, ApiError> {
    let theme = match user {
        Some(user) => state.store.get_user_setting(user.id, "theme").await?,
        None => None,
    };
    let value = match theme.as_deref() {
        Some("dark") => "'dark'",
        Some("light") => "'light'",
        _ => SYSTEM_THEME,
    };
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/javascript")
        .header("cache-control", "no-store")
        .body(box_body(FullBody::new(Bytes::from(format!(
            "document.documentElement.setAttribute('data-theme',{value});\n"
        )))))
        .expect("valid status and headers"))
}
