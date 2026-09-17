use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use openwebide_core::{FileEntry, WorkspaceMode};
use web_sys::wasm_bindgen::JsCast;

/// The project file explorer: a collapsible directory tree with create and
/// search actions. Directory contents are loaded lazily and cached in
/// `entries` (keyed by directory path, root = "").
#[component]
pub fn FileTree(
    mode: ReadSignal<WorkspaceMode>,
    entries: ReadSignal<HashMap<String, Vec<FileEntry>>>,
    expanded: ReadSignal<HashSet<String>>,
    open_file: ReadSignal<Option<String>>,
    search_results: ReadSignal<Option<Vec<FileEntry>>>,
    error: ReadSignal<Option<String>>,
    on_toggle: Callback<String>,
    on_open: Callback<String>,
    on_new_file: Callback<()>,
    on_new_dir: Callback<()>,
    on_search: Callback<String>,
    on_clear_search: Callback<()>,
) -> impl IntoView {
    let search_input = NodeRef::<leptos::html::Input>::new();

    // Flatten the (lazily loaded) tree into a list of (entry, depth) pairs,
    // following only expanded directories. Recomputes when `entries` or
    // `expanded` change. A flat list avoids a recursive component, which
    // would otherwise create a recursive opaque return type.
    let flat = RwSignal::new(Vec::<(FileEntry, u32)>::new());
    Effect::new(move || {
        let map = entries.get();
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
        let mut guard = flat.write();
        *guard = result;
    });

    view! {
        <div class="file-tree">
            <div class="file-tree-header">
                <h2>"Explorer"</h2>
                <span class="file-tree-actions">
                    <button class="icon-btn" title="New file" on:click=move |_| on_new_file.run(())>
                        "🗎"
                    </button>
                    <button class="icon-btn" title="New folder" on:click=move |_| on_new_dir.run(())>
                        "🗀"
                    </button>
                </span>
            </div>
            <div class="file-tree-search">
                <input
                    type="text"
                    class="search-input"
                    placeholder="Search files…"
                    node_ref=search_input
                    on:input=move |e: web_sys::Event| {
                        if let Some(target) = e.target()
                            && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                        {
                            let q = input.value();
                            if q.is_empty() {
                                on_clear_search.run(());
                            } else {
                                on_search.run(q);
                            }
                        }
                    }
                />
            </div>
            <Show when=move || error.get().is_some() fallback=|| ()>
                <div class="tree-error">{move || error.get().unwrap_or_default()}</div>
            </Show>
            <Show
                when=move || search_results.get().is_some()
                fallback=move || {
                    view! {
                        <Show
                            when=move || mode.get() == WorkspaceMode::Local
                            fallback=move || {
                                view! {
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
                                                            {move || if is_dir {
                                                                if expanded.get().contains(&path_icon) {
                                                                    "▾"
                                                                } else {
                                                                    "▸"
                                                                }
                                                            } else {
                                                                "·"
                                                            }}
                                                        </span>
                                                        <span class="tree-name">{name}</span>
                                                    </div>
                                                }
                                            }
                                        />
                                    </div>
                                }
                            }
                        >
                            <p class="empty">"Local mode opens a folder in your browser (coming soon)."</p>
                        </Show>
                    }
                }
            >
                <div class="tree-root search-results">
                    <For
                        each=move || search_results.get().unwrap_or_default()
                        key=|e| e.path.clone()
                        children=move |e| {
                            let path_class = e.path.clone();
                            let path_click = e.path.clone();
                            let name = e.name.clone();
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
                                    <span class="tree-icon">"·"</span>
                                    <span class="tree-name">{name}</span>
                                </div>
                            }
                        }
                    />
                </div>
            </Show>
        </div>
    }
}
