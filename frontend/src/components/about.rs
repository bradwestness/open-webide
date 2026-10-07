use super::{
    Modal,
    ui::{DialogActions, DialogBody, DialogSize},
};
use crate::state::ui::UiState;
use leptos::prelude::*;

// The shared build generator escapes all metadata and notice text.
const OVERVIEW: &str = include_str!(concat!(env!("OUT_DIR"), "/about-overview.html"));
const SOFTWARE: &str = include_str!(concat!(env!("OUT_DIR"), "/about-software.html"));

#[component]
pub fn About() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let close = Callback::new(move |()| ui.about_open.set(false));
    let software = RwSignal::new(false);
    let overview_tab = NodeRef::<leptos::html::Button>::new();
    let software_tab = NodeRef::<leptos::html::Button>::new();
    let on_key = move |event: web_sys::KeyboardEvent| {
        let next = match event.key().as_str() {
            "ArrowLeft" | "ArrowRight" => !software.get_untracked(),
            "Home" => false,
            "End" => true,
            _ => return,
        };
        event.prevent_default();
        software.set(next);
        let tab = if next { software_tab } else { overview_tab };
        if let Some(tab) = tab.get_untracked() {
            let _ = tab.focus();
        }
    };
    view! {
        <Modal title="About Open WebIDE".to_string().into() on_close=close size=DialogSize::Wide>
            <div class="ui-segmented-control about-tabs" role="tablist" aria-label="About pages" on:keydown=on_key>
                <button class="ui-seg-btn" class:active=move || !software.get() type="button" role="tab" id="about-tab-overview" aria-controls="about-panel" aria-selected=move || (!software.get()).to_string() tabindex=move || if software.get() { -1 } else { 0 } node_ref=overview_tab on:click=move |_| software.set(false)>"About"</button>
                <button class="ui-seg-btn" class:active=move || software.get() type="button" role="tab" id="about-tab-software" aria-controls="about-panel" aria-selected=move || software.get().to_string() tabindex=move || if software.get() { 0 } else { -1 } node_ref=software_tab on:click=move |_| software.set(true)>"Open-source software"</button>
            </div>
            <DialogBody><div class="about-content" id="about-panel" role="tabpanel" tabindex="0" aria-labelledby=move || if software.get() { "about-tab-software" } else { "about-tab-overview" } inner_html=move || if software.get() { SOFTWARE } else { OVERVIEW } /></DialogBody>
            <DialogActions><button class="btn" on:click=move |_| close.run(())>"Close"</button></DialogActions>
        </Modal>
    }
}
