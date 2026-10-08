use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::{
    components::ui::{Icon, IconButton, IconName},
    state::workspace::WorkspaceState,
    state_actions::workspace::WorkspaceActions,
};

/// File navigation uses the same selected-tab styling and icon actions as projects.
#[component]
pub(super) fn EditorTabs() -> impl IntoView {
    let workspace = expect_context::<WorkspaceState>();
    let actions = use_context::<WorkspaceActions>();
    let paths = Memo::new(move |_| {
        workspace
            .active_project
            .get()
            .map_or_else(Vec::new, |project| {
                workspace
                    .editor_tabs
                    .with(|tabs| tabs.get(&project).cloned().unwrap_or_default())
            })
    });
    view! {
        <Show when=move || actions.is_some() && !paths.with(Vec::is_empty)>
            <div class="tabbar editor-file-tabs" role="tablist" aria-label="Open files">
                <For each=move || paths.get() key=Clone::clone children=move |path| {
                    let selected_path = path.clone();
                    let dirty_path = path.clone();
                    let label_path = path.clone();
                    let open_path = path.clone();
                    let close_path = path.clone();
                    let keyboard_path = path.clone();
                    let menu_path = path.clone();
                    let action_path = path.clone();
                    let on_tab_action = Callback::new(move |action| { if let Some(actions) = actions { actions.file_tab_action.run((action_path.clone(), action)); } });
                    let position = Signal::derive(move || paths.with(|paths| paths.iter().position(|path| path == &menu_path).map(|index| (index, paths.len()))));
                    let entry = openwebide_core::FileEntry { path: path.clone(), name: path.rsplit('/').next().unwrap_or(&path).to_string(), is_dir: false, size: 0 };

                    let selected = Signal::derive(move || workspace.open_file.with(|file| file.as_ref() == Some(&selected_path)));
                    let dirty = Signal::derive(move || {
                        if workspace.open_file.with(|file| file.as_ref() == Some(&dirty_path)) {
                            workspace.dirty.get()
                        } else {
                            workspace.active_project.get().is_some_and(|project| {
                                workspace.editor_buffers.with(|buffers| buffers.get(&(project, dirty_path.clone())).is_some_and(|buffer| buffer.dirty))
                            })
                        }
                    });
                    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                    view! {
                        <div class="tab editor-file-tab" class:active=move || selected.get() role="presentation" data-context-menu="">
                            <super::file_tree::FileEntryMenu entry=entry context_only=true>
                                <super::tab_actions::TabActionItems position=position on_action=on_tab_action />
                            </super::file_tree::FileEntryMenu>
                            <button type="button" class="btn tab-select" role="tab" title=path.clone()
                                aria-selected=move || selected.get().to_string()
                                tabindex=move || if selected.get() { "0" } else { "-1" }
                                data-editor-tab=path
                                on:click=move |_| { if !selected.get_untracked() && let Some(actions) = actions { actions.request_open.run(open_path.clone()); } }
                                on:keydown=move |event: web_sys::KeyboardEvent| {
                                    let items = paths.get_untracked();
                                    let Some(index) = items.iter().position(|path| path == &keyboard_path) else { return; };
                                    let next = match event.key().as_str() {
                                        "ArrowRight" => items.get((index + 1) % items.len()),
                                        "ArrowLeft" => items.get((index + items.len() - 1) % items.len()),
                                        "Home" => items.first(),
                                        "End" => items.last(),
                                        _ => None,
                                    };
                                    if let Some(next) = next {
                                        event.prevent_default();
                                        if workspace.open_file.with_untracked(|file| file.as_ref() != Some(next)) && let Some(actions) = actions { actions.request_open.run(next.clone()); }
                                        if let Some(target) = event.current_target().and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                            && let Ok(Some(list)) = target.closest(".editor-file-tabs")
                                            && let Ok(buttons) = list.query_selector_all("[data-editor-tab]") {
                                            for i in 0..buttons.length() {
                                                if let Some(button) = buttons.item(i).and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
                                                    && button.get_attribute("data-editor-tab").as_deref() == Some(next) { let _ = button.focus(); break; }
                                            }
                                        }
                                    }
                                }>
                                <span class="tab-name">{name}</span>
                                <span class="editor-tab-dirty" class:is-dirty=move || dirty.get() aria-hidden=move || (!dirty.get()).to_string() aria-label="Unsaved changes">"●"</span>
                            </button>
                            <IconButton class="tab-close" label=Signal::derive(move || format!("Close {label_path}"))
                                on_click=Callback::new(move |_| { if let Some(actions) = actions { actions.close_file.run(close_path.clone()); } })>
                                <Icon name=IconName::X />
                            </IconButton>
                        </div>
                    }
                } />
            </div>
        </Show>
    }
}
