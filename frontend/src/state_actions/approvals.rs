//! One mode-selection path for the TUI picker and keyboard shortcut.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, ui::UiState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::ApprovalMode;

pub fn current_mode(chat: ChatState) -> ApprovalMode {
    chat.active_session
        .get_untracked()
        .map(|session| {
            chat.approval_mode
                .with_untracked(|modes| modes.get(&session).copied().unwrap_or_default())
        })
        .unwrap_or_else(|| chat.draft_approval_mode.get_untracked())
}
pub async fn save_session_mode(api: Api, session: i64, mode: ApprovalMode) -> Result<(), String> {
    api.with_value(Clone::clone)
        .set_setting(
            &ApprovalMode::setting_key(session),
            &serde_json::to_string(&mode).map_err(|error| error.to_string())?,
        )
        .await
}
pub fn mode_selector(chat: ChatState) -> Callback<ApprovalMode> {
    let api = expect_context::<Api>();
    let auth = expect_context::<AuthState>();
    let ui = expect_context::<UiState>();
    Callback::new(move |mode| select_mode(api, auth, ui, chat, mode))
}

fn select_mode(api: Api, auth: AuthState, ui: UiState, chat: ChatState, mode: ApprovalMode) {
    let session = chat.active_session.get_untracked();
    let epoch = auth.generation.get_untracked();
    spawn_local(async move {
        let key = session
            .map(ApprovalMode::setting_key)
            .unwrap_or_else(|| "draft_approval_mode".into());
        let value = serde_json::to_string(&mode).expect("approval mode serializes");
        let result = api.with_value(Clone::clone).set_setting(&key, &value).await;
        if auth.generation.try_get_untracked() != Some(epoch) {
            return;
        }
        match result {
            Ok(()) => {
                if let Some(session) = session {
                    chat.set_approval_mode(session, mode);
                } else {
                    chat.draft_approval_mode.set(mode);
                }
            }
            Err(error) => ui.notify(format!("Could not save approval mode: {error}")),
        }
    });
}
