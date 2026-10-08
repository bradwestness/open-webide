use leptos::prelude::*;

use crate::state::projects::ProjectsState;

/// Rider-style project tabs: each open project is a tab; the active one is
/// highlighted. Opening and recent-project actions live in the app menu.
#[component]
pub fn TabBar(
    on_select: Callback<i64>,
    on_select_chat: Callback<()>,
    on_close: Callback<i64>,
    #[prop(default = Callback::new(|_| ()))] on_tab_action: Callback<(i64, crate::tabs::TabAction)>,
) -> impl IntoView {
    let projects = expect_context::<ProjectsState>();
    let open_tabs = Signal::derive(move || projects.open_tabs());
    let active_project = projects.active_project.read_only();
    view! {
        <div class="tabbar-wrap">
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
                                data-project-tab=id.to_string()
                                data-context-menu=""
                                on:click=move |_| on_select.run(id)
                            >
                                <super::tab_actions::TabActions position=Signal::derive(move || projects.open_tab_ids.with(|tabs| tabs.iter().position(|tab| *tab == id).map(|index| (index, tabs.len())))) on_action=Callback::new(move |action| on_tab_action.run((id, action))) />
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
                                    <crate::components::ui::Icon name=crate::components::ui::IconName::X />
                                </button>
                            </div>
                        }
                    }
                />
            </div>
            <button
                class=move || if active_project.get().is_none() { "tab chat-tab active" } else { "tab chat-tab" }
                title="Chat without a project — web tools and host information"
                aria-label="Chat without a project"
                aria-pressed=move || active_project.get().is_none().to_string()
                on:click=move |_| { on_select_chat.run(()); }
            >
                <super::ui::Icon name=super::ui::IconName::MessageCircle />
            </button>

        </div>
    }
}
