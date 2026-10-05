use crate::{state::chat::ChatState, state_actions::approvals};
use leptos::prelude::*;
use openwebide_core::ApprovalMode;

#[component]
pub fn ApprovalModePicker() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let open = RwSignal::new(false);
    let select_mode = approvals::mode_selector(chat);
    let mode = Signal::derive(move || {
        chat.active_session
            .get()
            .map(|session| {
                chat.approval_mode
                    .with(|modes| modes.get(&session).copied().unwrap_or_default())
            })
            .unwrap_or_else(|| chat.draft_approval_mode.get())
    });
    view! {
        <super::dropdown::Dropdown class="approval-mode-picker" menu_class="approval-mode-menu" aria_label="Approval mode" trigger_class="btn tui-mode-badge" open=open above=true label=move || view! { <span class:mode-awaiting=move || mode.get() == ApprovalMode::Yolo>{move || format!("[{}]", mode.get().label().to_uppercase())}</span> }>
            {ApprovalMode::CHOICES.into_iter().map(|choice| view! {
                <button type="button" class="ui-dropdown-item recent-item" role="menuitemradio" aria-checked=move || (mode.get() == choice).to_string() on:click=move |_| { select_mode.run(choice); open.set(false); }>
                    <span class="ui-dropdown-item-text">
                        <span>{choice.label()}</span>
                        <span class="ui-dropdown-item-description">{choice.description()}</span>
                    </span>
                </button>
            }).collect_view()}
        </super::dropdown::Dropdown>
    }
}
