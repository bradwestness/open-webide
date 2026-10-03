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
        <span class="approval-mode-picker" on:keydown=move |event| { if event.key() == "Escape" { open.set(false); event.stop_propagation(); } }>
            <button class=move || if mode.get() == ApprovalMode::Yolo { "btn tui-mode-badge mode-awaiting" } else { "btn tui-mode-badge mode-normal" } title="Approval mode (Shift+Tab in the composer)" aria-haspopup="menu" aria-expanded=move || open.get()
                on:click=move |_| open.update(|value| *value = !*value)>
                {move || format!("[{}]", mode.get().label().to_uppercase())}
            </button>
            <Show when=move || open.get()>
                <div class="recent-backdrop" on:click=move |_| open.set(false) />
                <div class="recent-menu approval-mode-menu" role="menu">
                    {ApprovalMode::CHOICES.into_iter().map(|choice| view! {
                        <button class="btn recent-item" role="menuitemradio" aria-checked=move || mode.get() == choice
                            on:click=move |_| { select_mode.run(choice); open.set(false); }>{choice.label()}</button>
                    }).collect_view()}
                    <p class="form-hint">"Auto uses the fast model or primary model. YOLO approves all tools, including commands."</p>
                </div>
            </Show>
        </span>
    }
}
