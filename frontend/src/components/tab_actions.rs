use crate::tabs::TabAction;
use leptos::prelude::*;

/// Shared context actions for both tab strips; the ordinary strip stays compact.
#[component]
pub(super) fn TabActions(
    position: Signal<Option<(usize, usize)>>,
    on_action: Callback<TabAction>,
) -> impl IntoView {
    view! {
        <super::dropdown::ActionMenu aria_label="Tab actions" context_only=true>
            <TabActionItems position=position on_action=on_action />
        </super::dropdown::ActionMenu>
    }
}

#[component]
pub(super) fn TabActionItems(
    position: Signal<Option<(usize, usize)>>,
    on_action: Callback<TabAction>,
) -> impl IntoView {
    let auth = use_context::<crate::state::auth::AuthState>();
    let account = auth.map(|auth| auth.generation.get_untracked());
    view! {
            {[
                (TabAction::CloseOthers, "Close others"),
                (TabAction::CloseLeft, "Close all to left"),
                (TabAction::CloseRight, "Close all to right"),
                (TabAction::MoveLeft, "Move left"),
                (TabAction::MoveRight, "Move right"),
            ].into_iter().map(move |(action, label)| view! {
                <button type="button" class="ui-dropdown-item" role="menuitem"
                    disabled=move || position.get().is_none_or(|(index, count)| match action {
                        TabAction::CloseOthers => count < 2,
                        TabAction::CloseLeft | TabAction::MoveLeft => index == 0,
                        TabAction::CloseRight | TabAction::MoveRight => index + 1 >= count,
                    }) on:click=move |event: web_sys::MouseEvent| {
                        use wasm_bindgen::JsCast;
                        if auth.and_then(|auth| auth.generation.try_get_untracked()) != account { return; }
                        if event.current_target().and_then(|target| target.dyn_into::<web_sys::Element>().ok()).is_some_and(|target| target.is_connected()) { on_action.run(action); }
                    }>{label}</button>
            }).collect_view()}
    }
}
