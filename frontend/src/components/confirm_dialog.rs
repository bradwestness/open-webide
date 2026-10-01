use leptos::prelude::*;
use openwebide_frontend::state::ui::UiState;

/// A themed confirmation dialog. Shown while `req` is `Some`; clicking the
/// overlay, the close button, or "Cancel" runs `on_close` (which clears the
/// request), while "Confirm" runs the request's `action` and then closes.
#[component]
pub fn ConfirmDialog() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let req = ui.confirm.read_only();
    let on_close = Callback::new(move |_| ui.clear_confirm());
    view! {
        <Show when=move || req.get().is_some() fallback=|| ()>
            <div class="modal-overlay" on:click=move |_| on_close.run(())>
                <div class="modal modal-sm" on:click=move |e: web_sys::MouseEvent| e.stop_propagation()>
                    <div class="modal-header">
                        <h2>{move || req.with(|r| r.as_ref().map(|r| r.title.clone()).unwrap_or_default())}</h2>
                        <button
                            class="icon-btn"
                            title="Close"
                            on:click=move |_| on_close.run(())
                        >
                            "✕"
                        </button>
                    </div>
                    <div class="modal-body">
                        <p class="confirm-message">
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
                </div>
            </div>
        </Show>
    }
}
