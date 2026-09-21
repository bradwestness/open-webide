use leptos::prelude::*;

/// A pending single-field input request. `on_submit` runs with the entered
/// text (trimmed) when the user submits via Enter or the submit button.
#[derive(Clone)]
pub struct PromptRequest {
    pub title: String,
    /// The field's initial value (pre-filled; empty for a fresh input).
    pub value: String,
    /// Hint shown when the field is empty.
    pub placeholder: String,
    /// Label for the submit button (e.g. "Create").
    pub submit_label: String,
    pub on_submit: Callback<String>,
}

/// A themed single-field input dialog. Shown while `req` is `Some`; clicking
/// the overlay, the close button, or "Cancel" runs `on_close` (which clears
/// the request), while submitting runs the request's `on_submit` with the
/// entered text and then closes.
#[component]
pub fn PromptDialog(
    req: ReadSignal<Option<PromptRequest>>,
    on_close: Callback<()>,
) -> impl IntoView {
    let input_ref = NodeRef::<leptos::html::Input>::new();
    // Pre-fill the field's initial value and focus it when the dialog opens.
    Effect::new(move || {
        if req.get().is_some()
            && let Some(el) = input_ref.get()
        {
            let value = req.with(|r| r.as_ref().map(|r| r.value.clone()).unwrap_or_default());
            el.set_value(&value);
            let _ = el.focus();
        }
    });
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
                        <input
                            type="text"
                            class="form-input"
                            placeholder=move || req.with(|r| r.as_ref().map(|r| r.placeholder.clone()).unwrap_or_default())
                            node_ref=input_ref
                            on:keydown=move |e: leptos::ev::KeyboardEvent| {
                                if e.key() == "Enter" {
                                    e.prevent_default();
                                    submit_prompt(&req, &input_ref, &on_close);
                                }
                            }
                        />
                    </div>
                    <div class="modal-footer">
                        <button class="btn" on:click=move |_| on_close.run(())>"Cancel"</button>
                        <button
                            class="btn send"
                            on:click=move |_| submit_prompt(&req, &input_ref, &on_close)
                        >
                            {move || req.with(|r| r.as_ref().map(|r| r.submit_label.clone()).unwrap_or_else(|| "Create".to_string()))}
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}

/// Submit the prompt dialog: read the field, run `on_submit` with the trimmed
/// text (if non-empty), then close.
fn submit_prompt(
    req: &ReadSignal<Option<PromptRequest>>,
    input_ref: &NodeRef<leptos::html::Input>,
    on_close: &Callback<()>,
) {
    let Some(r) = req.get() else {
        return;
    };
    let Some(el) = input_ref.get() else {
        return;
    };
    let value = el.value().trim().to_string();
    if value.is_empty() {
        return;
    }
    r.on_submit.run(value);
    on_close.run(());
}
