use leptos::prelude::*;
use leptos::task::spawn_local;
use web_sys::PointerEvent;

use crate::api::BackendApi;
use openwebide_frontend::state::layout::{ActiveResizer, LayoutState};

#[component]
pub fn PanelResizer(kind: ActiveResizer) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    let api = expect_context::<BackendApi>();
    let start_x = RwSignal::new(0.0f64);
    let start_width = RwSignal::new(0.0f64);
    let width = match kind {
        ActiveResizer::Sidebar => layout.sidebar_width,
        ActiveResizer::Tree => layout.tree_width,
        ActiveResizer::Chat => layout.chat_width,
        ActiveResizer::None => RwSignal::new(0.0f64),
    };

    let _ = window_event_listener(leptos::ev::pointermove, move |ev: PointerEvent| {
        if layout.active_resizer.get() != kind || kind == ActiveResizer::None {
            return;
        }

        let current_x = ev.client_x();
        let delta = if kind == ActiveResizer::Chat {
            start_x.get() - current_x
        } else {
            current_x - start_x.get()
        };
        let requested = start_width.get() + delta;
        let total_width = web_sys::window()
            .and_then(|window| window.inner_width().ok())
            .and_then(|value| value.as_f64())
            .unwrap_or(1200.0);

        width.set(LayoutState::clamp(
            kind,
            requested,
            layout.sidebar_width.get(),
            layout.tree_width.get(),
            layout.chat_width.get(),
            total_width,
        ));
    });

    let _ = window_event_listener(leptos::ev::pointerup, move |_| {
        if layout.active_resizer.get() == ActiveResizer::None {
            return;
        }

        layout.active_resizer.set(ActiveResizer::None);
        let sidebar_width = layout.sidebar_width.get();
        let tree_width = layout.tree_width.get();
        let chat_width = layout.chat_width.get();
        spawn_local(async move {
            let _ = api
                .set_setting(
                    ActiveResizer::Sidebar.setting_key(),
                    &sidebar_width.to_string(),
                )
                .await;
            let _ = api
                .set_setting(ActiveResizer::Tree.setting_key(), &tree_width.to_string())
                .await;
            let _ = api
                .set_setting(ActiveResizer::Chat.setting_key(), &chat_width.to_string())
                .await;
        });
    });

    let title = match kind {
        ActiveResizer::Sidebar => "Drag to resize sidebar, double-click to reset",
        ActiveResizer::Tree => "Drag to resize file tree / diff viewer, double-click to reset",
        ActiveResizer::Chat => "Drag to resize diff viewer / chat pane, double-click to reset",
        ActiveResizer::None => "",
    };

    view! {
        <div
            class=move || {
                if layout.active_resizer.get() == kind {
                    "panel-resizer is-active"
                } else {
                    "panel-resizer"
                }
            }
            title=title
            on:pointerdown=move |ev: PointerEvent| {
                if kind == ActiveResizer::None {
                    return;
                }
                ev.prevent_default();
                start_x.set(ev.client_x());
                start_width.set(width.get());
                layout.active_resizer.set(kind);
            }
            on:dblclick=move |_| {
                if kind == ActiveResizer::None {
                    return;
                }
                let default_width = kind.default();
                width.set(default_width);
                spawn_local(async move {
                    let _ = api
                        .set_setting(kind.setting_key(), &default_width.to_string())
                        .await;
                });
            }
        />
    }
}
