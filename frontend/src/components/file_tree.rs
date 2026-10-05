use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use openwebide_core::{FileEntry, vfs::SearchOptions};
use web_sys::wasm_bindgen::JsCast;

use crate::state::{
    git::GitState, layout::LayoutState, projects::ProjectsState, workspace::WorkspaceState,
};

/// The project file explorer: a collapsible directory tree with create and
/// search actions. Directory contents are loaded lazily and cached in
/// `entries` (keyed by directory path, root = "").
#[component]
pub fn FileTree(
    on_toggle: Callback<String>,
    on_open: Callback<String>,
    on_new_file: Callback<()>,
    on_new_dir: Callback<()>,
    #[prop(optional)] on_grant_access: Option<Callback<()>>,
) -> impl IntoView {
    let workspace = expect_context::<WorkspaceState>();
    let projects = expect_context::<ProjectsState>();
    let git = expect_context::<GitState>();
    let layout = expect_context::<LayoutState>();

    let entries = workspace.entries.read_only();
    let expanded = workspace.expanded.read_only();
    let open_file = workspace.open_file.read_only();
    let tree_width = layout.tree_width.read_only();
    let git_status: Signal<Option<openwebide_core::GitRepoStatus>> =
        Signal::derive(move || git.status.get());
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
            <super::ui::PanelToolbar class="file-tree-header">
                <super::dropdown::ActionMenu aria_label="File actions">
                    <button role="menuitem" class="ui-dropdown-item recent-item icon-btn" title="New file" on:click=move |_| on_new_file.run(())>
                        <crate::components::ui::Icon name=crate::components::ui::IconName::File />
                    <span>"New file"</span></button>
                    <button role="menuitem" class="ui-dropdown-item recent-item icon-btn" title="New folder" on:click=move |_| on_new_dir.run(())>
                        <crate::components::ui::Icon name=crate::components::ui::IconName::Folder />
                    <span>"New folder"</span></button>
                </super::dropdown::ActionMenu>
            </super::ui::PanelToolbar>
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
                        <div class="tree-root">
                            <For
                                each=move || flat.get()
                                key=|e| e.0.path.clone()
                                children=move |(entry, depth)| {
                                    let is_dir = entry.is_dir;
                                    let name = entry.name.clone();
                                    let path_class = entry.path.clone();
                                    let path_click = entry.path.clone();
                                     let path_icon = entry.path.clone();
                                    let path_badge = entry.path.clone();
                                    view! {
                                        <div
                                            class=move || {
                                                if open_file.get().as_deref()
                                                    == Some(path_class.as_str())
                                                {
                                                    "tree-item selected".to_string()
                                                } else {
                                                    "tree-item".to_string()
                                                }
                                            }
                                            style=move || {
                                                format!("padding-left: {}px", 8 + depth as usize * 14)
                                            }
                                            on:click=move |_| {
                                                if is_dir {
                                                    on_toggle.run(path_click.clone());
                                                } else {
                                                    on_open.run(path_click.clone());
                                                }
                                            }
                                        >
                                            <span class="tree-icon">
                                                <super::ui::Icon name=Signal::derive(move || if is_dir {
                                                    if expanded.get().contains(&path_icon) { super::ui::IconName::FolderOpen } else { super::ui::IconName::Folder }
                                                } else { super::ui::IconName::File }) />
                                            </span>
                                            <span class="tree-name">{name}</span>
                                            {
                                                let path_badge = path_badge.clone();
                                                move || {
                                                    let status_opt = git_status.get();
                                                    let status = status_opt.as_ref()?;
                                                    if is_dir {
                                                        let dir_prefix = format!("{path_badge}/");
                                                        let has_modified = status.files.keys().any(|k| k.starts_with(&dir_prefix));
                                                        if has_modified {
                                                            Some(view! { <span class="git-badge git-badge-dir" title="Contains modified files">"•"</span> }.into_any())
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        status.files.get(&path_badge).map(|s| {
                                                            let badge = s.badge();
                                                            let css_class = s.css_class();
                                                            view! { <span class=format!("git-badge {css_class}") title=css_class>{badge}</span> }.into_any()
                                                        })
                                                    }
                                                }
                                            }
                                        </div>
                                    }
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
