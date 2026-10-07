use super::{
    Modal,
    ui::{DialogActions, DialogBody, DialogSize},
};
use crate::state::ui::UiState;
use leptos::prelude::*;

// Trusted build-time markup: all dependency metadata and notice text is escaped
// by the shared inventory generator. No network or workspace access is needed.
const CONTENT: &str = include_str!(concat!(env!("OUT_DIR"), "/about.html"));

#[component]
pub fn About() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let close = Callback::new(move |()| ui.about_open.set(false));
    view! {
        <Modal title="About Open WebIDE".to_string().into() on_close=close size=DialogSize::Wide>
            <DialogBody><div inner_html=CONTENT /></DialogBody>
            <DialogActions><button class="btn" on:click=move |_| close.run(())>"Close"</button></DialogActions>
        </Modal>
    }
}
