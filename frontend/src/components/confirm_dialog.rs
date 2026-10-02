use super::modal::Modal;
use crate::state::ui::UiState;
use leptos::prelude::*;

/// A themed confirmation dialog. Shown while `req` is `Some`; clicking the
/// overlay, the close button, or "Cancel" runs `on_close` (which clears the
/// request), while "Confirm" runs the request's `action` and then closes.
#[component]
pub fn ConfirmDialog() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let req = ui.confirm.read_only();
    let on_close = Callback::new(move |()| ui.clear_confirm());
    view! {
        <Show when=move || req.get().is_some() fallback=|| ()>
            <Modal title=Signal::derive(move || req.with(|r| r.as_ref().map(|r| r.title.clone()).unwrap_or_default())) on_close=on_close class="modal modal-sm" describedby="confirm-description">
                <div class="modal-body">
                    <p class="confirm-message" id="confirm-description">
                        {move || req.with(|r| r.as_ref().map(|r| r.message.clone()).unwrap_or_default())}
                    </p>
                </div>
                <div class="modal-footer">
                    <button class="btn" on:click=move |_| on_close.run(())>"Cancel"</button>
                    <button
                        class="btn danger"
                        on:click=move |_| {
                            if let Some(r) = req.get() {
                                r.action.run(());
                            }
                            on_close.run(());
                        }
                    >
                        {move || req.with(|r| r.as_ref().map(|r| r.confirm_label.clone()).unwrap_or_else(|| "Confirm".to_string()))}
                    </button>
                </div>
            </Modal>
        </Show>
    }
}
