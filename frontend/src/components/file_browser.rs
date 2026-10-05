use super::modal::Modal;
use super::ui::{DialogActions, DialogBody};
use leptos::prelude::*;
use openwebide_core::FileEntry;
use wasm_bindgen_futures::spawn_local;

use crate::backend::Api;

/// A modal file browser for picking a host folder (remote-mode projects).
/// It navigates the host mount tree (the preopen root, e.g. `~/source`); the
/// "Select folder" button runs `on_select` with the current directory's path
/// (relative to the mount root). Local-mode projects keep the system picker.
#[component]
pub fn FileBrowser(on_close: Callback<()>, on_select: Callback<String>) -> impl IntoView {
    let api = expect_context::<Api>();
    let current = RwSignal::new(String::new());
    let entries = RwSignal::new(Vec::<FileEntry>::new());
    let error = RwSignal::new(Option::<String>::None);
    let loading = RwSignal::new(false);

    // Load the directory at `current` whenever it changes (and once at start).
    Effect::new(move || {
        let dir = current.get();
        let api = api;
        loading.set(true);
        spawn_local(async move {
            let result = api.with_value(Clone::clone).browse(&dir).await;
            // Drop a stale response: `current` may have changed again while
            // this request was in flight (a quick navigation into one
            // directory and out before the listing arrives). The newer
            // request owns updating these signals.
            if current.try_get_untracked().as_ref() != Some(&dir) {
                return;
            }
            match result {
                Ok(list) => {
                    entries.set(list);
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            loading.set(false);
        });
    });

    view! {
        <Modal title=Signal::derive(|| "Choose a folder".to_string()) on_close=on_close description=Signal::derive(|| "On the device hosting Open WebIDE".to_string())>
            <DialogBody>
                <div class="browser-path">
                    {move || {
                        let c = current.get();
                        if c.is_empty() {
                            "~/source".to_string()
                        } else {
                            format!("~/source/{c}")
                        }
                    }}
                </div>
                <Show when=move || error.get().is_some() fallback=|| ()>
                    <div class="browser-error">{move || error.get().unwrap_or_default()}</div>
                </Show>
                <div class="browser-list">
                    <Show when=move || loading.get() fallback=|| ()>
                        <div class="browser-empty">"Loading…"</div>
                    </Show>
                    <Show when=move || !current.get().is_empty() fallback=|| ()>
                        <button
                            class="browser-item"
                            on:click=move |_| {
                                current.update(|c| {
                                    if let Some(idx) = c.rfind('/') {
                                        *c = c[..idx].to_string();
                                    } else {
                                        *c = String::new();
                                    }
                                });
                            }
                        >
                            <span class="browser-icon"><crate::components::ui::Icon name=crate::components::ui::IconName::ArrowUp /></span>
                            <span class="browser-name">".."</span>
                        </button>
                    </Show>
                    <For
                        each=move || entries.get()
                        key=|e| e.path.clone()
                        children=move |e| {
                            let name = e.name.clone();
                            let path = e.path.clone();
                            if e.is_dir {
                                view! {
                                    <button type="button" class="browser-item dir" on:click=move |_| current.set(path.clone())>
                                        <span class="browser-icon"><crate::components::ui::Icon name=crate::components::ui::IconName::Folder /></span>
                                        <span class="browser-name">{name}</span>
                                    </button>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="browser-item">
                                        <span class="browser-icon"><crate::components::ui::Icon name=crate::components::ui::IconName::File /></span>
                                        <span class="browser-name">{name}</span>
                                    </div>
                                }.into_any()
                            }
                        }
                    />
                </div>
            </DialogBody>
            <DialogActions>
                <button class="btn" on:click=move |_| on_close.run(())>"Cancel"</button>
                <button
                    class="btn send"
                    on:click=move |_| {
                        on_select.run(current.get());
                        on_close.run(());
                    }
                >
                    "Select folder"
                </button>
            </DialogActions>
        </Modal>
    }
}
