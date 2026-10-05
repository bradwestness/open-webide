//! Clipboard transport with an HTTP/LAN fallback, shared by copy actions.
use wasm_bindgen::JsCast;

pub async fn copy_text(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("Browser unavailable")?;
    let navigator = window.navigator();
    if let Ok(value) = js_sys::Reflect::get(navigator.as_ref(), &"clipboard".into())
        && let Ok(clipboard) = value.dyn_into::<web_sys::Clipboard>()
        && wasm_bindgen_futures::JsFuture::from(clipboard.write_text(text))
            .await
            .is_ok()
    {
        return Ok(());
    }
    let document = window.document().ok_or("Document unavailable")?;
    let active = document.active_element();
    let textarea = document
        .create_element("textarea")
        .map_err(|_| "Could not copy text")?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .map_err(|_| "Could not copy text")?;
    textarea.set_value(text);
    textarea
        .set_attribute("aria-hidden", "true")
        .map_err(|_| "Could not copy text")?;
    textarea
        .set_attribute("style", "position:fixed;left:-10000px;top:0")
        .map_err(|_| "Could not copy text")?;
    document
        .body()
        .ok_or("Document unavailable")?
        .append_child(&textarea)
        .map_err(|_| "Could not copy text")?;
    textarea.select();
    let result = document
        .unchecked_ref::<web_sys::HtmlDocument>()
        .exec_command("copy");
    textarea.remove();
    if let Some(active) = active.and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = active.focus();
    }
    if result.unwrap_or(false) {
        Ok(())
    } else {
        Err("Clipboard access was denied".into())
    }
}
