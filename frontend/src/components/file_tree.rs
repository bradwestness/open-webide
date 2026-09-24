use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use openwebide_core::{FileEntry, SearchHit, vfs::SearchOptions};
use web_sys::wasm_bindgen::JsCast;

/// The project file explorer: a collapsible directory tree with create and
/// search actions. Directory contents are loaded lazily and cached in
/// `entries` (keyed by directory path, root = "").
#[component]
pub fn FileTree(
    entries: ReadSignal<HashMap<String, Vec<FileEntry>>>,
    expanded: ReadSignal<HashSet<String>>,
    open_file: ReadSignal<Option<String>>,
    search_results: ReadSignal<Option<Vec<SearchHit>>>,
    on_toggle: Callback<String>,
    on_open: Callback<String>,
    on_new_file: Callback<()>,
    on_new_dir: Callback<()>,
    on_search: Callback<(String, SearchOptions)>,
    include_ignored: ReadSignal<bool>,
    on_toggle_include_ignored: Callback<()>,
    on_clear_search: Callback<()>,
    #[prop(default = Signal::derive(|| false))] needs_grant: Signal<bool>,
    #[prop(optional)] on_grant_access: Option<Callback<()>>,
    #[prop(default = Signal::derive(|| None))] git_status: Signal<
        Option<openwebide_core::GitRepoStatus>,
    >,
    #[prop(into, optional)] width: Option<Signal<f64>>,
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
        flat.set(result);
    });

    view! {
        <div
            class="file-tree"
            style=move || width.map(|w| format!("width: {}px; flex: none;", w.get())).unwrap_or_default()
        >
            <div class="file-tree-header">
                <h2>"Explorer"</h2>
                <span class="file-tree-actions">
                    <button class="icon-btn" title="New file" on:click=move |_| on_new_file.run(())>
                        "📄"
                    </button>
                    <button class="icon-btn" title="New folder" on:click=move |_| on_new_dir.run(())>
                        "📁"
                    </button>
                </span>
            </div>
            <div
                class="file-tree-search"
                style="display: flex; gap: 4px; align-items: center;"
            >
                <input
                    type="text"
                    class="search-input"
                    style="flex: 1;"
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
                                on_search.run((
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
                    "📂"
                </button>
            </div>
            <Show when=move || needs_grant.get() fallback=|| ()>
                <div style="padding: 12px; text-align: center;">
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
            <Show
                when=move || search_results.get().is_some()
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
                    }
                }
            >
                <div class="tree-root search-results">
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
            </Show>
        </div>
    }
}
