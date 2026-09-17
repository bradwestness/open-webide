use leptos::prelude::*;
use openwebide_core::Project;

/// Rider-style project tabs: each open project is a tab; the active one is
/// highlighted. A trailing "+" opens the new-project form in the sidebar.
#[component]
pub fn TabBar(
    open_tabs: ReadSignal<Vec<Project>>,
    active_project: ReadSignal<Option<i64>>,
    on_new: Callback<()>,
    on_select: Callback<i64>,
    on_close: Callback<i64>,
) -> impl IntoView {
    view! {
        <div class="tabbar">
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
                        >
                            <span class="tab-name" on:click=move |_| on_select.run(id)>
                                {name}
                            </span>
                            <span class="tab-mode">{mode.as_str()}</span>
                            <button
                                class="icon-btn tab-close"
                                title="Close project"
                                on:click=move |_| on_close.run(id)
                            >
                                "✕"
                            </button>
                        </div>
                    }
                }
            />
            <button
                class="icon-btn tab-new"
                title="New project"
                on:click=move |_| on_new.run(())
            >
                "+"
            </button>
        </div>
    }
}
