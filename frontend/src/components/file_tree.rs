use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use openwebide_core::{FileEntry, vfs::SearchOptions};
use web_sys::wasm_bindgen::JsCast;

use crate::state::{
    git::GitState, layout::LayoutState, projects::ProjectsState, workspace::WorkspaceState,
};

/// A collapsible project file tree with directory contents loaded lazily and cached in
/// `entries` (keyed by directory path, root = "").
#[component]
pub fn FileTree(
    on_toggle: Callback<String>,
    on_open: Callback<String>,
    #[prop(optional)] on_grant_access: Option<Callback<()>>,
) -> impl IntoView {
    let workspace = expect_context::<WorkspaceState>();
    let projects = expect_context::<ProjectsState>();
    let layout = expect_context::<LayoutState>();

    let entries = workspace.entries.read_only();
    let expanded = workspace.expanded.read_only();
    let tree_width = layout.tree_width.read_only();
    let needs_grant = Signal::derive(move || {
        projects
            .active_project
            .get()
            .is_some_and(|id| projects.needs_grant.with(|ids| ids.contains(&id)))
    });

    // Flatten the (lazily loaded) tree into a list of (entry, depth) pairs,
    // following only expanded directories. Recomputes when `entries` or
    // `expanded` change. A flat list avoids a recursive component, which
    // would otherwise create a recursive opaque return type.
    let flat = RwSignal::new(Vec::<(FileEntry, u32)>::new());
    Effect::new(move || {
        let mut map = entries.get();
        for children in map.values_mut() {
            openwebide_core::vfs::sort_file_entries(children);
        }
        let exp = expanded.get();
        let mut result: Vec<(FileEntry, u32)> = Vec::new();
        fn traverse(
            map: &HashMap<String, Vec<FileEntry>>,
            exp: &HashSet<String>,
            dir: &str,
            depth: u32,
            result: &mut Vec<(FileEntry, u32)>,
        ) {
            if let Some(children) = map.get(dir) {
                for child in children {
                    result.push((child.clone(), depth));
                    if child.is_dir && exp.contains(&child.path) {
                        traverse(map, exp, &child.path, depth + 1, result);
                    }
                }
            }
        }
        traverse(&map, &exp, "", 0, &mut result);
        flat.set(result);
    });

    view! {
        <div
            class="file-tree"
            style=move || format!("width: {}px; flex: none;", tree_width.get())
        >
            <Show when=move || needs_grant.get() fallback=|| ()>
                <div style="padding: 12px; text-align: center;">
                    <p>"This browser needs permission to access the project folder. Grant access or select the folder again to reconnect it."</p>
                    <crate::components::Button
                        variant=crate::components::ButtonVariant::Primary
                        size=crate::components::ButtonSize::Sm
                        on_click=Callback::new(move |_| {
                            if let Some(cb) = &on_grant_access {
                                cb.run(());
                            }
                        })
                    >
                        "Grant folder access"
                    </crate::components::Button>
                </div>
            </Show>
                        <div class="tree-root" role="tree" aria-label="Project files">
                            <For
                                each=move || flat.get()
                                key=move |entry| (projects.active_project.get_untracked(), entry.0.path.clone(), entry.0.is_dir)
                                children=move |(entry, depth)| {
                                    view! { <FileTreeEntry entry=entry depth=depth on_toggle=on_toggle on_open=on_open /> }

                                }
                            />
                        </div>

        </div>
    }
}

/// Project search shares the workspace search state and actions in both modes.
#[component]
pub fn SearchPane(
    on_open: Callback<String>,
    on_search_input: Callback<(String, SearchOptions)>,
    on_cancel_search: Callback<()>,
    on_search: Callback<(String, SearchOptions)>,
    include_ignored: ReadSignal<bool>,
    on_toggle_include_ignored: Callback<()>,
    on_clear_search: Callback<()>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    on_cleanup(move || on_cancel_search.run(()));
    let workspace = expect_context::<WorkspaceState>();
    let search_results = workspace.search.read_only();
    let open_file = workspace.open_file.read_only();
    let query = RwSignal::new(String::new());
    let search_input = NodeRef::<leptos::html::Input>::new();
    let layout = expect_context::<crate::state::layout::LayoutState>();
    Effect::new(move |_| {
        workspace.active_project.track();
        query.set(String::new());
        if let Some(input) = search_input.get() {
            input.set_value("");
        }
    });
    Effect::new(move |_| {
        if layout
            .preferences
            .with(|prefs| prefs.files_view == crate::state::responsive::FilesView::Search)
            && let Some(input) = search_input.get()
        {
            let _ = input.focus();
        }
    });
    view! { <div class="search-pane">

            <super::ui::PanelSearchRow class="file-tree-search">
                <input
                    type="text"
                    class="form-input panel-search-input search-input"
                    aria-label="Search project files"
                    placeholder="Search files…"
                    node_ref=search_input
                    on:input=move |e: web_sys::Event| {
                        if let Some(target) = e.target()
                            && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                        {
                            let q = input.value();
                            query.set(q.clone());
                            if q.is_empty() {
                                on_clear_search.run(());
                            } else {
                                on_search_input.run((
                                    q,
                                    SearchOptions {
                                        include_ignored: include_ignored.get(),
                                    },
                                ));
                            }
                        }
                    }
                />
                <button
                    class="icon-btn"
                    title="Include ignored folders (.git, target, node_modules, dist)"
                    aria-pressed=move || include_ignored.get().to_string()
                    on:click=move |_| {
                        on_toggle_include_ignored.run(());
                        if let Some(input) = search_input.get() {
                            let q = input.value();
                            if !q.is_empty() {
                                on_search.run((
                                    q,
                                    SearchOptions {
                                        include_ignored: include_ignored.get(),
                                    },
                                ));
                            }
                        }
                    }
                >
                    <crate::components::ui::Icon name=crate::components::ui::IconName::FolderSearch />
                </button>
            </super::ui::PanelSearchRow>
                <div class="search-file-views" hidden=move || !query.get().is_empty()>{children.map(|children| children())}</div>
                <div class="tree-root search-results" hidden=move || query.get().is_empty()>
                    <For
                        each=move || search_results.get().unwrap_or_default()
                        key=|e| format!("{}:{}", e.path, e.line)
                        children=move |e| {
                            let path_class = e.path.clone();
                            let path_click = e.path.clone();
                            let path = e.path.clone();
                            let line = e.line;
                            let text = e.text.clone();
                            view! {
                                <div
                                    class=move || {
                                        if open_file.get().as_deref() == Some(path_class.as_str()) {
                                            "tree-item selected".to_string()
                                        } else {
                                            "tree-item".to_string()
                                        }
                                    }
                                    on:click=move |_| on_open.run(path_click.clone())
                                >
                                    <span class="tree-hit-path">{path}</span>
                                    <span class="tree-hit-line">{line}</span>
                                    <span class="tree-hit-text">{text}</span>
                                </div>
                            }
                        }
                    />
                </div>
    </div> }
}

/// Explorer creation actions can share a toolbar with the Files view switcher.
#[component]
pub fn FileActions(on_new_file: Callback<()>, on_new_dir: Callback<()>) -> impl IntoView {
    view! { <super::dropdown::ActionMenu aria_label="File actions">
        <button role="menuitem" class="ui-dropdown-item recent-item icon-btn" title="New file" on:click=move |_| on_new_file.run(())>
            <crate::components::ui::Icon name=crate::components::ui::IconName::File />
        <span>"New file"</span></button>
        <button role="menuitem" class="ui-dropdown-item recent-item icon-btn" title="New folder" on:click=move |_| on_new_dir.run(())>
            <crate::components::ui::Icon name=crate::components::ui::IconName::Folder />
        <span>"New folder"</span></button>
    </super::dropdown::ActionMenu> }
}

fn tree_rows(target: Option<web_sys::EventTarget>) -> Vec<web_sys::HtmlElement> {
    target
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|target| target.closest(".tree-root").ok().flatten())
        .and_then(|tree| tree.query_selector_all(".tree-item").ok())
        .map_or_else(Vec::new, |rows| {
            (0..rows.length())
                .filter_map(|index| rows.item(index))
                .filter_map(|row| row.dyn_into::<web_sys::HtmlElement>().ok())
                .collect()
        })
}

/// One entry owns its menu and gestures; every action uses the shared facade.
#[component]
pub(super) fn FileTreeEntry(
    entry: FileEntry,
    depth: u32,
    on_toggle: Callback<String>,
    on_open: Callback<String>,
    #[prop(default = false)] changes_only: bool,
) -> impl IntoView {
    use super::{
        dropdown::Dropdown,
        ui::{Icon, IconName},
    };
    use crate::state_actions::file_tree::FileTreeActions;
    use openwebide_core::{
        git::{GitPathAction, GitPathChanges},
        vfs::VfsEntryKind,
        workspace_entries::parent,
    };
    let workspace = expect_context::<WorkspaceState>();
    let git = expect_context::<GitState>();
    let actions = use_context::<FileTreeActions>();
    let is_dir = entry.is_dir;
    let name = entry.name.clone();
    let entry = StoredValue::new(entry);
    let open = RwSignal::new(false);
    let anchor = RwSignal::new(None::<(f64, f64)>);
    let changes = RwSignal::new(None::<Result<GitPathChanges, String>>);
    let menu_generation = StoredValue::new(0_u64);
    let owner = Owner::current().expect("File tree entry has an owner");
    let load_changes = Callback::new(move |()| {
        changes.set(None);
        menu_generation.update_value(|generation| *generation += 1);
        let generation = menu_generation.get_value();
        if let Some(actions) = actions {
            let epoch = actions.epoch.get_untracked();
            leptos::task::spawn_local(async move {
                let result = actions.path_changes().await;
                if actions.epoch.try_get_untracked() == Some(epoch)
                    && menu_generation.try_get_value() == Some(generation)
                    && open.try_get_untracked() == Some(true)
                {
                    changes.set(Some(result));
                }
            });
        }
    });
    let show = Callback::new(move |point: Option<(f64, f64)>| {
        if actions.is_none() {
            return;
        }
        anchor.set(point);
        open.set(true);
        load_changes.run(());
    });
    Effect::new(move |_| {
        if let Some(actions) = actions {
            actions.epoch.get();
        }
        open.set(false);
    });
    let root = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        if let Some(root) = root.get() {
            let _ = root.set_attribute("aria-level", &(depth + 1).to_string());
        }
    });
    super::context_menu::context_menu_target(
        move || {
            actions?.epoch.get();
            root.get()
                .map(|root| root.unchecked_ref::<web_sys::HtmlElement>().clone())
        },
        show,
    );
    let activate = Callback::new(move |()| {
        if is_dir {
            on_toggle.run(entry.get_value().path);
        } else {
            on_open.run(entry.get_value().path);
        }
    });
    let stage = Signal::derive(move || {
        changes.with(|changes| {
            changes
                .as_ref()
                .and_then(|changes| changes.as_ref().ok())
                .is_some_and(|changes| changes.has_unstaged(&entry.get_value().path))
        })
    });
    let unstage = Signal::derive(move || {
        changes.with(|changes| {
            changes
                .as_ref()
                .and_then(|changes| changes.as_ref().ok())
                .is_some_and(|changes| changes.has_staged(&entry.get_value().path))
        })
    });
    let revert = Signal::derive(move || {
        changes.with(|changes| {
            changes
                .as_ref()
                .and_then(|changes| changes.as_ref().ok())
                .is_some_and(|changes| {
                    changes.has_head && changes.has_tracked_changes(&entry.get_value().path)
                })
        })
    });
    let untracked = Signal::derive(move || {
        changes.with(|changes| {
            changes
                .as_ref()
                .and_then(|changes| changes.as_ref().ok())
                .is_some_and(|changes| {
                    changes.untracked.iter().any(|path| {
                        openwebide_core::workspace_entries::contains_path(
                            &entry.get_value().path,
                            path,
                        )
                    })
                })
        })
    });
    let review = Signal::derive(move || {
        stage.get()
            || unstage.get()
            || changes_only
            || git.status.with(|status| {
                status.as_ref().is_some_and(|status| {
                    status.files.keys().any(|path| {
                        openwebide_core::workspace_entries::contains_path(
                            &entry.get_value().path,
                            path,
                        )
                    })
                })
            })
    });
    let disabled = Signal::derive(move || {
        actions.is_none_or(FileTreeActions::disabled)
            || openwebide_core::workspace_entries::entry_path(&entry.get_value().path).is_err()
    });
    view! {
        <div node_ref=root data-context-menu="" class=move || if workspace.open_file.get().as_deref() == Some(&entry.get_value().path) {"tree-item selected"} else {"tree-item"}
            style=format!("padding-left: {}px", 8 + depth as usize * 14) tabindex="0" role="treeitem" aria-label=name.clone() data-tree-path=entry.get_value().path
            aria-expanded=move || is_dir.then(|| workspace.expanded.with(|dirs| dirs.contains(&entry.get_value().path)).to_string())
            on:click=move |_| activate.run(())
            on:keydown=move |event: web_sys::KeyboardEvent| {
                if event.target().and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                    .is_some_and(|target| target.closest(".ui-dropdown").ok().flatten().is_some()) {return;}
                let rows = tree_rows(event.target());
                let index = rows.iter().position(|row| row.get_attribute("data-tree-path").as_deref() == Some(&entry.get_value().path)).unwrap_or(0);
                let target = match event.key().as_str() {
                    "ArrowDown" => rows.get(index + 1),
                    "ArrowUp" => rows.get(index.saturating_sub(1)),
                    "Home" => rows.first(), "End" => rows.last(),
                    "ArrowRight" if is_dir => {
                        if workspace.expanded.with_untracked(|dirs| dirs.contains(&entry.get_value().path)) {rows.get(index + 1)}
                        else {on_toggle.run(entry.get_value().path); None}
                    }
                    "ArrowLeft" => {
                        if is_dir && workspace.expanded.with_untracked(|dirs| dirs.contains(&entry.get_value().path)) {on_toggle.run(entry.get_value().path); None}
                        else {rows.iter().find(|row| row.get_attribute("data-tree-path").as_deref() == Some(parent(&entry.get_value().path)))}
                    }
                    _ => None,
                };
                if matches!(event.key().as_str(), "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Home" | "End") {
                    event.prevent_default(); if let Some(target) = target {let _ = target.focus();} return;
                }
                if event.key() == "ContextMenu" || (event.key() == "F10" && event.shift_key()) {
                    event.prevent_default(); event.stop_propagation(); show.run(None);
                } else if matches!(event.key().as_str(), "Enter" | " ") {event.prevent_default(); activate.run(());}
            }
            >
            <span class="tree-icon"><Icon name=Signal::derive(move || if is_dir {
                if workspace.expanded.with(|dirs| dirs.contains(&entry.get_value().path)) {IconName::FolderOpen} else {IconName::Folder}
            } else {IconName::File}) /></span>
            <span class="tree-name">{name.clone()}</span>
            {move || git.status.with(|status| {
                let status = status.as_ref()?; let path = entry.get_value().path;
                if is_dir {
                    status.files.keys().any(|file| file.starts_with(&format!("{path}/")))
                        .then(|| view! {<span class="git-badge git-badge-dir" title="Contains modified files">"•"</span>}.into_any())
                } else {
                    status.files.get(&path).map(|status| view! {
                        <span class=format!("git-badge {}", status.css_class()) title=status.css_class()>{status.badge()}</span>
                    }.into_any())
                }
            })}
            {actions.map(move |actions| view! {
                <Dropdown aria_label="File actions" class="ui-action-menu tree-entry-menu" trigger_class="icon-btn ui-icon" hide_caret=true
                    open=open pointer_anchor=anchor.into() on_open=Callback::new(move |()| {anchor.set(None); load_changes.run(());})
                    label=|| view! {<Icon name=IconName::Ellipsis />}>
                    <div class="ui-action-items" on:click=move |event| {
                        if event.target().and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                            .is_some_and(|target| target.closest("button:not(:disabled)").ok().flatten().is_some()) {open.set(false);}
                    }>{move || changes.with(|result| result.as_ref().and_then(|result| result.as_ref().err()).map(|error| view! {<div class="form-hint" role="status">{format!("Git actions unavailable: {error}")}</div>}))}
                    {owner.with(|| view! {
                        {(!changes_only).then(|| view! {
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get()
                            on:click=move |_| {let entry = entry.get_value(); actions.create(if is_dir {&entry.path} else {parent(&entry.path)}, VfsEntryKind::File);}>"New file"</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() || !is_dir
                            on:click=move |_| {let entry = entry.get_value(); actions.create(&entry.path, VfsEntryKind::Directory);}>"New folder"</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() on:click=move |_| actions.move_entry(&entry.get_value(), true)>"Rename"</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() on:click=move |_| actions.move_entry(&entry.get_value(), false)>"Move"</button>
                        <button class="recent-item" role="menuitem" on:click=move |_| actions.copy_path(&entry.get_value().path)>"Copy path"</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() on:click=move |_| actions.delete(&entry.get_value())>"Delete"</button>
                        })}
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() || !stage.get()
                            on:click=move |_| actions.git_action(&entry.get_value().path, GitPathAction::Stage)>{move || if untracked.get() {"Add / track"} else {"Stage"}}</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() || !unstage.get()
                            on:click=move |_| actions.git_action(&entry.get_value().path, GitPathAction::Unstage)>"Unstage"</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() || !untracked.get()
                            on:click=move |_| actions.ignore(&entry.get_value())>"Ignore"</button>
                        <button class="recent-item" role="menuitem" disabled=move || disabled.get() || !revert.get()
                            on:click=move |_| actions.git_action(&entry.get_value().path, GitPathAction::Revert)>"Revert changes"</button>
                        <button class="recent-item" role="menuitem" title="Explain the purpose, behavior and how the code works" disabled=move || actions.disabled() on:click=move |_| actions.chat(&entry.get_value(), "Explain how this works:", false)>"Explain in chat"</button>
                        <button class="recent-item" role="menuitem" title="Give a brief overview of the purpose and key contents" disabled=move || actions.disabled() on:click=move |_| actions.chat(&entry.get_value(), "Give a concise overview of", false)>"Summarize in chat"</button>
                        <button class="recent-item" role="menuitem" disabled=move || actions.disabled() || !review.get()
                            on:click=move |_| actions.chat(&entry.get_value(), "Review changes for bugs and regressions in", true)>"Review changes in chat"</button>
                    })}</div>
                </Dropdown>
            })}
        </div>
    }
}
