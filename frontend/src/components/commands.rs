use super::modal::Modal;
use crate::{
    commands::{COMMANDS, COMPOSER_SHORTCUTS, Command, search},
    state::ui::UiState,
    state_actions::commands::CommandActions,
};
use leptos::prelude::*;

#[component]
pub fn CommandDialogs() -> impl IntoView {
    let ui = expect_context::<UiState>();
    view! {
        <Show when=move || ui.palette_open.get()><CommandPalette /></Show>
        <Show when=move || ui.shortcuts_open.get()><KeyboardShortcuts /></Show>
    }
}
#[component]
fn CommandPalette() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let actions = expect_context::<CommandActions>();
    let scope = actions.scope.get_untracked();
    let query = RwSignal::new(String::new());
    let results = Memo::new(move |_| search(&query.get()));
    let selected = RwSignal::new(None::<usize>);
    Effect::new(move |_| {
        let context = actions.context.get();
        results.with(|results| {
            selected.set(
                results
                    .iter()
                    .position(|command| command.command.unavailable(context).is_none()),
            );
        });
    });
    let activate = move |index: usize| {
        if actions.scope.get_untracked() != scope {
            return;
        }
        if let Some(command) = results.with_untracked(|results| results.get(index).copied()) {
            actions.run.run(command.command);
        }
    };
    let close = Callback::new(move |()| ui.palette_open.set(false));
    view! {
        <Modal title="Command palette".to_string().into() on_close=close class="modal modal-sm" describedby="command-palette-help">
            <div class="modal-body command-palette">
                <label class="form-label" for="command-search">"Search commands"</label>
                <input id="command-search" class="form-input command-search" type="search" autocomplete="off" placeholder="Type a command…" role="combobox"
                    aria-autocomplete="list" aria-expanded="true" aria-controls="command-results"
                    aria-activedescendant=move || selected.get().and_then(|index| results.with(|results| results.get(index).map(|command| format!("command-{}", command.id))))
                    prop:value=move || query.get()
                    on:input=move |event| query.set(event_target_value(&event))
                    on:keydown=move |event: web_sys::KeyboardEvent| {
                        if event.is_composing() { return; }
                        match event.key().as_str() {
                            "ArrowDown" | "ArrowUp" => {
                                event.prevent_default();
                                let choices: Vec<_> = results.with_untracked(|results| results.iter().enumerate().filter(|(_,command)| command.command.unavailable(actions.context.get_untracked()).is_none()).map(|(index,_)| index).collect());
                                if choices.is_empty() { return; }
                                let current = selected.get_untracked().and_then(|index| choices.iter().position(|choice| *choice == index));
                                let next = if event.key() == "ArrowUp" { current.map_or(choices.len()-1, |index| (index + choices.len()-1)%choices.len()) } else { current.map_or(0, |index| (index+1)%choices.len()) };
                                selected.set(Some(choices[next]));
                                if let Some(document) = web_sys::window().and_then(|window| window.document())
                                    && let Some(id) = results.with_untracked(|results| results.get(choices[next]).map(|command| command.id))
                                    && let Some(element) = document.get_element_by_id(&format!("command-{id}")) { element.scroll_into_view_with_bool(false); }
                            }
                            "Enter" => { event.prevent_default(); if let Some(index) = selected.get_untracked() { activate(index); } }
                            _ => {}
                        }
                    }
                />
                <p id="command-palette-help" class="form-hint">"↑ / ↓ to choose · Enter to run · Escape to close"</p>
                <div id="command-results" class="command-results" role="listbox" aria-label="Commands">
                    {move || results.get().into_iter().enumerate().map(|(index, command)| view! {
                        <button id=format!("command-{}",command.id) class=move || format!("btn ghost command-option{}", if selected.get() == Some(index) { " selected" } else { "" })
                            role="option" aria-selected=move || (selected.get() == Some(index)).to_string()
                            disabled=move || command.command.unavailable(actions.context.get()).is_some()
                            on:click=move |_| activate(index)>
                            <span>{command.label}<Show when=move || command.command.unavailable(actions.context.get()).is_some()><span class="form-hint command-unavailable">{move || command.command.unavailable(actions.context.get()).unwrap_or_default()}</span></Show></span>
                            <kbd>{command.shortcut}</kbd>
                        </button>
                    }).collect::<Vec<_>>()}
                    <Show when=move || results.with(Vec::is_empty)><p class="empty" role="status">"No matching commands"</p></Show>
                </div>
            </div>
            <div class="modal-footer">
                <button class="btn ghost" on:click=move |_| actions.run.run(Command::Shortcuts)>"Keyboard shortcuts"</button>
                <button class="btn" on:click=move |_| close.run(())>"Close"</button>
            </div>
        </Modal>
    }
}
#[component]
fn KeyboardShortcuts() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let close = Callback::new(move |()| ui.shortcuts_open.set(false));
    view! {
        <Modal title="Keyboard shortcuts".to_string().into() on_close=close class="modal modal-sm" describedby="shortcut-help">
            <div class="modal-body keyboard-shortcuts">
                <p id="shortcut-help" class="form-hint">"Use Ctrl on Windows/Linux or ⌘ on macOS. Global commands pause while a dialog is open. Composer bindings apply while the prompt is focused."</p>
                <h3>"Workspace"</h3>
                <dl>{COMMANDS.iter().filter(|command| command.shortcut.starts_with("Ctrl")).map(|command| view! { <div class="shortcut-row"><dt><kbd>{command.shortcut}</kbd></dt><dd>{command.label}</dd></div> }).collect::<Vec<_>>()}</dl>
                <h3>"Chat composer"</h3>
                <dl>{COMPOSER_SHORTCUTS.iter().map(|(key, action)| view! { <div class="shortcut-row"><dt><kbd>{*key}</kbd></dt><dd>{*action}</dd></div> }).collect::<Vec<_>>()}</dl>
                <h3>"Command palette"</h3>
                <dl><div class="shortcut-row"><dt><kbd>"↑ / ↓"</kbd></dt><dd>"Choose a command"</dd></div><div class="shortcut-row"><dt><kbd>"Enter / Escape"</kbd></dt><dd>"Run / close"</dd></div></dl>
            </div>
            <div class="modal-footer"><button class="btn" on:click=move |_| close.run(())>"Close"</button></div>
        </Modal>
    }
}
