use crate::tool_output::{PREVIEW_LINES, ToolOutputContent};
use leptos::{prelude::*, task::spawn_local};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

#[component]
pub fn ToolOutput(text: String) -> impl IntoView {
    let output = StoredValue::new(ToolOutputContent::parse(&text));
    let expanded = RwSignal::new(false);
    let copy_status = RwSignal::new("Copy");
    let copy_busy = RwSignal::new(false);
    let has_more = output.with_value(|output| output.lines.len() > PREVIEW_LINES);
    let omitted = output.with_value(|output| output.truncated);
    let copy = move |_| {
        if copy_busy.get_untracked() {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Ok(element) = document.create_element("div") else {
            return;
        };
        // Only HTML produced by the shared escaping ANSI renderer reaches this
        // detached element. Copy the full retained output even when collapsed.
        element.set_inner_html(&output.with_value(|output| output.html(true)));
        let plain = element.text_content().unwrap_or_default();
        let clipboard =
            js_sys::Reflect::get(window.navigator().as_ref(), &JsValue::from_str("clipboard"));
        if !clipboard.is_ok_and(|value| !value.is_null() && !value.is_undefined()) {
            copy_status.set(if legacy_copy(&document, &plain) {
                "Copied"
            } else {
                "Copy failed"
            });
            return;
        }
        let promise = window.navigator().clipboard().write_text(&plain);
        copy_busy.set(true);
        spawn_local(async move {
            let result = JsFuture::from(promise).await;
            let _ = copy_busy.try_set(false);
            let _ = copy_status.try_set(if result.is_ok() {
                "Copied"
            } else {
                "Copy failed"
            });
        });
    };
    view! {
        <div class="tui-tool-output">
            <pre class="tui-tool-summary-out" inner_html=move || output.with_value(|output| output.html(expanded.get())) />
            <div class="tui-tool-output-controls">
                <Show when=move || has_more>
                    <button class="btn ghost tui-output-expand" aria-expanded=move || expanded.get().to_string() on:click=move |_| expanded.update(|expanded| *expanded = !*expanded)>
                        {move || if expanded.get() { "Show less".to_string() } else { output.with_value(|output| format!("Show all {} lines", output.lines.len())) }}
                    </button>
                </Show>
                <button class="btn ghost tui-output-copy" title="Copy all retained output without ANSI codes" disabled=move || copy_busy.get() on:click=copy>{move || copy_status.get()}</button>
                <Show when=move || omitted><span class="form-hint">"Output truncated by terminal display limits; Copy includes retained output."</span></Show>
            </div>
        </div>
    }
}

// The synchronous compatibility path keeps Copy usable on HTTP LAN origins,
// where the modern Clipboard API is unavailable.
fn legacy_copy(document: &web_sys::Document, text: &str) -> bool {
    let Ok(element) = document.create_element("textarea") else {
        return false;
    };
    let Ok(input) = element.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return false;
    };
    let Some(body) = document.body() else {
        return false;
    };
    let focus = document.active_element();
    input.set_value(text);
    input.set_read_only(true);
    input.set_class_name("tui-copy-buffer");
    if body.append_child(&input).is_err() {
        return false;
    }
    input.select();
    let copied = document
        .unchecked_ref::<web_sys::HtmlDocument>()
        .exec_command("copy")
        .unwrap_or(false);
    input.remove();
    if let Some(focus) = focus.and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok()) {
        let _ = focus.focus();
    }
    copied
}
