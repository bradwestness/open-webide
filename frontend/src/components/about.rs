use super::{
    Modal,
    ui::{DialogActions, DialogBody, LogoMark},
};
use crate::state::ui::UiState;
use leptos::prelude::*;

// The shared build generator escapes all metadata and notice text.
const OVERVIEW: &str = include_str!(concat!(env!("OUT_DIR"), "/about-overview.html"));
const SOFTWARE_URL: &str = env!("OPENWEBIDE_SOFTWARE_URL");

#[component]
pub fn About() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let close = Callback::new(move |()| ui.about_open.set(false));
    let software = RwSignal::new(false);
    let notices = RwSignal::new(None::<Result<String, String>>);
    let pending = RwSignal::new(false);
    let load = move || {
        if pending.get_untracked()
            || notices.with_untracked(|result| result.as_ref().is_some_and(Result::is_ok))
        {
            return;
        }
        pending.set(true);
        notices.set(None);
        leptos::task::spawn_local(async move {
            let result = async {
                let response = gloo_net::http::Request::get(SOFTWARE_URL)
                    .send()
                    .await
                    .map_err(|_| ())?;
                if !response.ok() {
                    return Err(());
                }
                response.text().await.map_err(|_| ())
            }
            .await
            .map_err(|()| "Could not load open-source software.".to_string());
            notices.try_set(Some(result));
            pending.try_set(false);
        });
    };
    Effect::new(move || {
        if software.get() {
            load();
        }
    });
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
        <Modal title="About Open WebIDE".to_string().into() on_close=close class="modal about-modal">
            <div class="ui-segmented-control about-tabs" role="tablist" aria-label="About pages" on:keydown=on_key>
                <button class="ui-seg-btn" class:active=move || !software.get() type="button" role="tab" id="about-tab-overview" aria-controls="about-panel" aria-selected=move || (!software.get()).to_string() tabindex=move || if software.get() { -1 } else { 0 } node_ref=overview_tab on:click=move |_| software.set(false)>"About"</button>
                <button class="ui-seg-btn" class:active=move || software.get() type="button" role="tab" id="about-tab-software" aria-controls="about-panel" aria-selected=move || software.get().to_string() tabindex=move || if software.get() { 0 } else { -1 } node_ref=software_tab on:click=move |_| software.set(true)>"Open-source software"</button>
            </div>
            <DialogBody class="about-body"><div class="about-content" class:about-overview=move || !software.get() class:about-software=move || software.get() id="about-panel" role="tabpanel" tabindex="0" aria-labelledby=move || if software.get() { "about-tab-software" } else { "about-tab-overview" }>
                {move || if software.get() {
                    match notices.get() {
                        Some(Ok(html)) => view! { <div inner_html=html /> }.into_any(),
                        Some(Err(error)) => view! { <p role="alert">{error}</p><button class="btn" on:click=move |_| load()>"Retry"</button> }.into_any(),
                        None => view! { <p role="status">"Loading open-source software…"</p> }.into_any(),
                    }
                } else {
                    view! {
                        <div>
                            <LogoMark class="about-logo" />
                            <div inner_html=OVERVIEW />
                            <a href="https://openwebide.com/" target="_blank" rel="noopener noreferrer">"openwebide.com"</a>
                        </div>
                    }.into_any()
                }}
            </div></DialogBody>
            <DialogActions><button class="btn" on:click=move |_| close.run(())>"Close"</button></DialogActions>
        </Modal>
    }
}
