//! Shared panel actions and user-scoped persistence; independent of workspace mode.
use crate::{
    backend::Api,
    state::{
        auth::AuthState,
        layout::{LayoutState, PANEL_VISIBILITY_KEY, Panel, PanelVisibility},
        ui::UiState,
    },
};
use leptos::{prelude::*, task::spawn_local};

#[derive(Clone, Copy)]
pub struct LayoutActions {
    pub toggle: Callback<Panel>,
    pub show: Callback<Panel>,
}
impl LayoutActions {
    pub fn new(api: Api, layout: LayoutState, auth: AuthState, ui: UiState) -> Self {
        let pending = StoredValue::new(None::<PanelVisibility>);
        let saving = StoredValue::new(None::<u64>);
        Effect::new(move |_| {
            auth.generation.track();
            layout.panels.set(PanelVisibility::default());
            layout.panel_revision.set(0);
            layout
                .sidebar_width
                .set(crate::state::layout::ActiveResizer::Sidebar.default());
            layout
                .tree_width
                .set(crate::state::layout::ActiveResizer::Tree.default());
            layout
                .chat_width
                .set(crate::state::layout::ActiveResizer::Chat.default());
            layout.active_resizer.set(Default::default());
            pending.set_value(None);
        });
        let change = Callback::new(move |(panel, visible): (Panel, bool)| {
            let mut panels = layout.panels.get_untracked();
            if panels.visible(panel) == visible {
                return;
            }
            panels.set(panel, visible);
            layout.panels.set(panels);
            layout.panel_revision.update(|value| *value += 1);
            layout.active_resizer.set(Default::default());
            let viewport = web_sys::window()
                .and_then(|window| window.inner_width().ok())
                .and_then(|value| value.as_f64())
                .unwrap_or(1200.0);
            layout.fit(viewport);
            if auth.user.get_untracked().is_none() {
                return;
            }
            pending.set_value(Some(panels));
            let epoch = auth.generation.get_untracked();
            if saving.get_value() == Some(epoch) {
                return;
            }
            saving.set_value(Some(epoch));
            spawn_local(async move {
                while auth.generation.try_get_untracked() == Some(epoch) {
                    let Some(panels) = pending.try_update_value(Option::take).flatten() else {
                        break;
                    };
                    let value =
                        serde_json::to_string(&panels).expect("panel visibility serializes");
                    let result = api
                        .with_value(Clone::clone)
                        .set_setting(PANEL_VISIBILITY_KEY, &value)
                        .await;
                    if auth.generation.try_get_untracked() != Some(epoch) {
                        return;
                    }
                    if let Err(message) = result {
                        ui.notify(format!("Could not save panel layout: {message}"));
                    }
                }
                if saving.try_get_value() == Some(Some(epoch)) {
                    saving.set_value(None);
                }
            });
        });
        Self {
            toggle: Callback::new(move |panel| {
                change.run((panel, !layout.panels.get_untracked().visible(panel)));
            }),
            show: Callback::new(move |panel| change.run((panel, true))),
        }
    }
}
