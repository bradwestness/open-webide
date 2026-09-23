use leptos::prelude::*;
use openwebide_core::Project;

/// Rider-style project tabs: each open project is a tab; the active one is
/// highlighted. The left side holds the "Open local" / "Open remote" buttons
/// and a "Recent" dropdown listing saved projects that are not currently
/// open (click a row to re-open it as a tab, ✕ to delete it).
#[component]
pub fn TabBar(
    open_tabs: ReadSignal<Vec<Project>>,
    projects: ReadSignal<Vec<Project>>,
    active_project: ReadSignal<Option<i64>>,
    on_select: Callback<i64>,
    on_close: Callback<i64>,
    on_open_local: Callback<()>,
    on_open_remote: Callback<()>,
    on_open_project: Callback<i64>,
    on_delete_project: Callback<i64>,
) -> impl IntoView {
    let show_recent = RwSignal::new(false);
    view! {
        <div class="tabbar-wrap">
            <div class="tabbar">
                <button class="tab-action" on:click=move |_| on_open_local.run(())>
                    "Open local"
                </button>
                <button class="tab-action" on:click=move |_| on_open_remote.run(())>
                    "Open remote"
                </button>
                <button
                    class=move || {
                        if show_recent.get() {
                            "tab-action recent open".to_string()
                        } else {
                            "tab-action recent".to_string()
                        }
                    }
                    on:click=move |_| show_recent.update(|v| *v = !*v)
                >
                    "Recent ▾"
                </button>
                <For
                    each=move || open_tabs.get()
                    key=|p| p.id
                    children=move |p| {
                        let id = p.id;
                        let name = p.name.clone();
                        let mode = p.mode;
                        view! {
                            <div
                                class=move || {
                                    if active_project.get() == Some(id) {
                                        "tab active".to_string()
                                    } else {
                                        "tab".to_string()
                                    }
                                }
                                on:click=move |_| on_select.run(id)
                            >
                                <span class="tab-name">
                                    {name}
                                </span>
                                <span class="tab-mode">{mode.as_str()}</span>
                                <button
                                    class="icon-btn tab-close"
                                    title="Close project"
                                    on:click=move |e: web_sys::MouseEvent| {
                                        e.stop_propagation();
                                        on_close.run(id);
                                    }
                                >
                                    "✕"
                                </button>
                            </div>
                        }
                    }
                />
            </div>
            <Show when=move || show_recent.get() fallback=|| ()>
                <div class="recent-backdrop" on:click=move |_| show_recent.set(false) />
                <div class="recent-menu">
                    <For
                        each=move || {
                            let open = open_tabs.get();
                            let open_keys: std::collections::HashSet<String> = open
                                .iter()
                                .map(project_workspace_key)
                                .collect();
                            let mut seen_keys = std::collections::HashSet::new();
                            let mut recent = Vec::new();

                            for p in projects.get().into_iter().rev() {
                                let key = project_workspace_key(&p);
                                if open.iter().any(|t| t.id == p.id) || open_keys.contains(&key) {
                                    continue;
                                }
                                if seen_keys.insert(key) {
                                    recent.push(p);
                                }
                            }
                            recent
                        }
                        key=|p| p.id
                        children=move |p| {
                            let id = p.id;
                            let name = p.name.clone();
                            let mode = p.mode;
                            view! {
                                <div
                                    class="recent-item"
                                    on:click=move |_| {
                                        on_open_project.run(id);
                                        show_recent.set(false);
                                    }
                                >
                                    <span class="recent-name">{name}</span>
                                    <span class="tab-mode">{mode.as_str()}</span>
                                    <button
                                        class="icon-btn tab-close"
                                        title="Delete project"
                                        on:click=move |e: web_sys::MouseEvent| {
                                            e.stop_propagation();
                                            on_delete_project.run(id);
                                        }
                                    >
                                        "✕"
                                    </button>
                                </div>
                            }
                        }
                    />
                    <Show
                        when=move || {
                            let open = open_tabs.get();
                            let open_keys: std::collections::HashSet<String> = open
                                .iter()
                                .map(project_workspace_key)
                                .collect();
                            let mut seen_keys = std::collections::HashSet::new();
                            let mut count = 0;
                            for p in projects.get().into_iter().rev() {
                                let key = project_workspace_key(&p);
                                if open.iter().any(|t| t.id == p.id) || open_keys.contains(&key) {
                                    continue;
                                }
                                if seen_keys.insert(key) {
                                    count += 1;
                                }
                            }
                            count == 0
                        }
                        fallback=|| ()
                    >
                        <p class="empty">"No other saved projects."</p>
                    </Show>
                </div>
            </Show>
        </div>
    }
}

fn project_workspace_key(p: &Project) -> String {
    let mode_str = p.mode.as_str();
    if let Some(path) = &p.path {
        format!("{}:{}", mode_str, path.trim_matches('/'))
    } else {
        format!("{}:name:{}", mode_str, p.name.trim())
    }
}
