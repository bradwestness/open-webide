use super::modal::Modal;
use super::ui::{DialogActions, DialogBody};
use crate::{
    commands::{COMMANDS, COMPOSER_SHORTCUTS, Command},
    state::ui::UiState,
    state_actions::commands::CommandActions,
    state_actions::omnibar::OmnibarActions,
};
use leptos::prelude::*;

#[component]
pub fn CommandDialogs() -> impl IntoView {
    let ui = expect_context::<UiState>();
    view! {
        <Show when=move || ui.about_open.get()><super::About /></Show>
        <Show when=move || ui.palette_open.get()><Omnibar /></Show>
        <Show when=move || ui.shortcuts_open.get()><KeyboardShortcuts /></Show>
    }
}
#[component]
fn Omnibar() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let actions = expect_context::<OmnibarActions>();
    let commands = expect_context::<CommandActions>();
    let scope = actions.scope.get_untracked();
    let query = actions.query;
    let results = actions.entries;
    let selected = RwSignal::new(None::<usize>);
    let previous_query = StoredValue::new(String::new());
    let selected_id = StoredValue::new(None::<String>);
    Effect::new(move |_| {
        let query = query.get();
        results.with(|results| {
            let current = if previous_query.get_value() == query {
                selected_id.get_value().and_then(|id| {
                    results
                        .iter()
                        .position(|entry| entry.id == id && entry.unavailable.is_none())
                })
            } else {
                None
            };
            let current =
                current.or_else(|| results.iter().position(|entry| entry.unavailable.is_none()));
            selected.set(current);
            selected_id.set_value(current.map(|index| results[index].id.clone()));
        });
        previous_query.set_value(query);
    });
    let activate = move |index: usize| {
        if actions.scope.get_untracked() != scope {
            return;
        }
        if let Some(entry) = results.with_untracked(|results| results.get(index).cloned())
            && entry.unavailable.is_none()
        {
            actions.activate.run(entry.action);
        }
    };
    let close = Callback::new(move |()| ui.palette_open.set(false));
    view! {
        <Modal title=Signal::derive(|| "Search".to_string()) on_close=close class="modal omnibar-modal" describedby="command-palette-help">
            <DialogBody class="command-palette omnibar">
                <label class="form-label" for="command-search">"Commands, files, projects and sessions"</label>
                <input id="command-search" class="form-input command-search" data-loading=move || actions.loading.get().to_string() type="search" autocomplete="off" maxlength="256" placeholder="Search anything…" role="combobox"
                    aria-autocomplete="list" aria-expanded="true" aria-controls="command-results"
                    aria-activedescendant=move || selected.get().and_then(|index|results.with(|results|results.get(index).map(|entry|entry.id.clone())))
                    prop:value=move || query.get()
                    on:input=move |event| query.set(event_target_value(&event))
                    on:keydown=move |event: web_sys::KeyboardEvent| {
                        if event.is_composing() { return; }
                        match event.key().as_str() {
                            "ArrowDown" | "ArrowUp" => {
                                event.prevent_default();
                                let choices:Vec<_>=results.with_untracked(|results|results.iter().enumerate().filter(|(_,entry)|entry.unavailable.is_none()).map(|(index,_)|index).collect());
                                if choices.is_empty() { return; }
                                let current=selected.get_untracked().and_then(|index|choices.iter().position(|choice|*choice==index));
                                let next=if event.key()=="ArrowUp" { current.map_or(choices.len()-1,|index|(index+choices.len()-1)%choices.len()) } else { current.map_or(0,|index|(index+1)%choices.len()) };
                                let index=choices[next];selected.set(Some(index));
                                if let Some(id)=results.with_untracked(|results|results.get(index).map(|entry|entry.id.clone())) {
                                    selected_id.set_value(Some(id.clone()));
                                    if let Some(element)=document().get_element_by_id(&id) { element.scroll_into_view_with_bool(false); }
                                }
                            }
                            "Enter" => { event.prevent_default();if let Some(index)=selected.get_untracked() { activate(index); } }
                            _=>{}
                        }
                    }
                />
                <p id="command-palette-help" class="form-hint">"↑ / ↓ to choose · Enter to open · Escape to close · > commands · / files · # projects · @ sessions"</p>
                <Show when=move || actions.loading.get()><p class="form-hint" role="status">"Loading filenames and sessions…"</p></Show>
                <Show when=move || actions.notice.get().is_some()><p class="form-hint" role="status">{move || actions.notice.get()}</p></Show>
                <div id="command-results" class="command-results" role="listbox" aria-label="Search results">
                    {move || results.get().into_iter().enumerate().map(|(index,entry)| {
                        let reason=entry.unavailable.clone();
                        view! {
                            <button id=entry.id class=move || format!("btn ghost command-option{}",if selected.get()==Some(index){" selected"}else{""}) data-result-kind=entry.kind role="option" aria-selected=move || (selected.get()==Some(index)).to_string() disabled=entry.unavailable.is_some() on:click=move |_|activate(index)>
                                <span class="omnibar-entry"><span class="omnibar-entry-label">{entry.label}</span><span class="form-hint omnibar-entry-detail">{reason.unwrap_or(entry.detail)}</span></span>
                                <span class="omnibar-entry-kind">{entry.kind}</span><kbd>{entry.shortcut}</kbd>
                            </button>
                        }
                    }).collect::<Vec<_>>()}
                    <Show when=move || results.with(Vec::is_empty)><p class="empty" role="status">"No matching results"</p></Show>
                </div>
            </DialogBody>
            <DialogActions>
                <button class="btn ghost" on:click=move |_| commands.run.run(Command::Shortcuts)>"Keyboard shortcuts"</button>
                <button class="btn" on:click=move |_|close.run(())>"Close"</button>
            </DialogActions>
        </Modal>
    }
}
#[component]
fn KeyboardShortcuts() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let close = Callback::new(move |()| ui.shortcuts_open.set(false));
    view! {
        <Modal title="Keyboard shortcuts".to_string().into() on_close=close class="modal modal-sm" describedby="shortcut-help">
            <DialogBody class="keyboard-shortcuts">
                <p id="shortcut-help" class="form-hint">"Use Ctrl on Windows/Linux or ⌘ on macOS. Global commands pause while a dialog is open. Composer bindings apply while the prompt is focused."</p>
                <h3>"Workspace"</h3>
                <dl>{COMMANDS.iter().filter(|command| command.shortcut.starts_with("Ctrl")).map(|command| view! { <div class="shortcut-row"><dt><kbd>{command.shortcut}</kbd></dt><dd>{command.label}</dd></div> }).collect::<Vec<_>>()}</dl>
                <h3>"Chat composer"</h3>
                <dl>{COMPOSER_SHORTCUTS.iter().map(|(key, action)| view! { <div class="shortcut-row"><dt><kbd>{*key}</kbd></dt><dd>{*action}</dd></div> }).collect::<Vec<_>>()}</dl>
                <h3>"Search"</h3>
                <dl><div class="shortcut-row"><dt><kbd>"↑ / ↓"</kbd></dt><dd>"Choose a result"</dd></div><div class="shortcut-row"><dt><kbd>"Enter / Escape"</kbd></dt><dd>"Open / close"</dd></div></dl>
            </DialogBody>
            <DialogActions><button class="btn" on:click=move |_| close.run(())>"Close"</button></DialogActions>
        </Modal>
    }
}
