use leptos::prelude::*;
use web_sys::wasm_bindgen::JsCast;

/// The code editor: a full-height textarea bound to the open file's content,
/// with a header showing the path, a dirty indicator, and a Save button.
#[component]
pub fn Editor(
    open_file: ReadSignal<Option<String>>,
    content: ReadSignal<String>,
    set_content: WriteSignal<String>,
    dirty: ReadSignal<bool>,
    set_dirty: WriteSignal<bool>,
    on_save: Callback<()>,
    error: ReadSignal<Option<String>>,
) -> impl IntoView {
    let ta = NodeRef::<leptos::html::Textarea>::new();

    // Mirror the loaded content into the textarea without clobbering the
    // user's in-progress edits (same pattern as the chat composer).
    Effect::new(move || {
        let value = content.get();
        if let Some(el) = ta.get()
            && el.value() != value
        {
            el.set_value(&value);
        }
    });

    view! {
        <div class="editor">
            <div class="editor-header">
                <span class="editor-path" title="Open file">
                    {move || match open_file.get() {
                        Some(p) => {
                            if dirty.get() {
                                format!("{p} ●")
                            } else {
                                p
                            }
                        }
                        None => "No file open".to_string(),
                    }}
                </span>
                <button
                    class="btn editor-save"
                    disabled=move || !dirty.get()
                    on:click=move |_| on_save.run(())
                >
                    "Save"
                </button>
            </div>
            <Show when=move || error.get().is_some() fallback=|| ()>
                <div class="editor-error">{move || error.get().unwrap_or_default()}</div>
            </Show>
            <Show
                when=move || open_file.get().is_some()
                fallback=move || {
                    view! {
                        <p class="empty editor-empty">"Select a file to edit, or create a new one in the explorer."</p>
                    }
                }
            >
                <textarea
                    class="editor-textarea"
                    spellcheck="false"
                    node_ref=ta
                    on:input=move |e: web_sys::Event| {
                        if let Some(target) = e.target()
                            && let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        {
                            set_content.set(textarea.value());
                            set_dirty.set(true);
                        }
                    }
                />
            </Show>
        </div>
    }
}
