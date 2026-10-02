use leptos::prelude::*;

use crate::state::projects::ProjectsState;

/// Rider-style project tabs: each open project is a tab; the active one is
/// highlighted. The left side holds the "Open local" / "Open remote" buttons
/// and a "Recent" dropdown listing saved projects that are not currently
/// open (click a row to re-open it as a tab, ✕ to delete it).
#[component]
pub fn TabBar(
    on_select: Callback<i64>,
    on_close: Callback<i64>,
    on_open_local: Callback<()>,
    on_open_remote: Callback<()>,
    on_open_project: Callback<i64>,
    on_delete_project: Callback<i64>,
) -> impl IntoView {
    let projects = expect_context::<ProjectsState>();
    let open_tabs = Signal::derive(move || projects.open_tabs());
    let recent_projects = projects.recent_projects;
    let active_project = projects.active_project.read_only();
    let show_recent = RwSignal::new(false);
    view! {
        <div class="tabbar-wrap">
            <div class="tabbar">
                <button
                    class="tab-action"
                    title="Open a project on this device"
                    on:click=move |_| on_open_local.run(())
                >
                    "Open local"
                </button>
                <button
                    class="tab-action"
                    title="Open a project on the device hosting Open WebIDE"
                    on:click=move |_| on_open_remote.run(())
                >
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
                        each=move || recent_projects.get()
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
                        when=move || recent_projects.with(Vec::is_empty)
                        fallback=|| ()
                    >
                        <p class="empty">"No other saved projects."</p>
                    </Show>
                </div>
            </Show>
        </div>
    }
}
