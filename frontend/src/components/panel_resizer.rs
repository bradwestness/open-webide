use leptos::prelude::*;
use web_sys::PointerEvent;

use crate::state::auth::AuthState;
use crate::state::layout::{ActiveResizer, LayoutState, Panel};
use crate::state_actions::layout::LayoutActions;

#[component]
pub fn PanelResizer(kind: ActiveResizer) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    let actions = expect_context::<LayoutActions>();
    let auth = expect_context::<AuthState>();
    let epoch = RwSignal::new(0_u64);
    let panel = match kind {
        ActiveResizer::Sidebar => Panel::Sessions,
        ActiveResizer::Tree => Panel::Files,
        ActiveResizer::Chat => Panel::Chat,
        ActiveResizer::Terminal => Panel::Terminal,
        ActiveResizer::None => Panel::Editor,
    };
    let start_x = RwSignal::new(0.0f64);
    let start_width = RwSignal::new(0.0f64);
    let width = if kind == ActiveResizer::None {
        RwSignal::new(0.0)
    } else {
        layout.width(kind)
    };

    let pointer_move = window_event_listener(leptos::ev::pointermove, move |ev: PointerEvent| {
        if layout.active_resizer.get() != kind
            || kind == ActiveResizer::None
            || auth.generation.get_untracked() != epoch.get_untracked()
        {
            return;
        }

        let current_x = ev.client_x();
        let delta = if layout
            .preferences
            .with_untracked(|prefs| prefs.side(panel.id()))
            == crate::state::responsive::PanelSide::Right
        {
            start_x.get() - current_x
        } else {
            current_x - start_x.get()
        };
        let requested = start_width.get() + delta;
        let total_width = web_sys::window()
            .and_then(|window| window.inner_width().ok())
            .and_then(|value| value.as_f64())
            .unwrap_or(1200.0);

        width.set(layout.clamp_visible(kind, requested, total_width));
    });

    let pointer_up = window_event_listener(leptos::ev::pointerup, move |_| {
        if layout.active_resizer.get() != kind
            || kind == ActiveResizer::None
            || auth.generation.get_untracked() != epoch.get_untracked()
        {
            return;
        }

        layout.active_resizer.set(ActiveResizer::None);
        actions.save_width.run(kind);
    });

    let pointer_cancel = window_event_listener(leptos::ev::pointercancel, move |_| {
        if layout.active_resizer.get_untracked() == kind {
            layout.active_resizer.set(ActiveResizer::None);
            if auth.generation.get_untracked() == epoch.get_untracked() {
                actions.save_width.run(kind);
            }
        }
    });
    on_cleanup(move || {
        pointer_move.remove();
        pointer_up.remove();
        pointer_cancel.remove();
        if layout.active_resizer.try_get_untracked() == Some(kind) {
            layout.active_resizer.set(ActiveResizer::None);
        }
    });

    let title = match kind {
        ActiveResizer::Sidebar => "Drag to resize sidebar, double-click to reset",
        ActiveResizer::Tree => "Drag to resize file tree / diff viewer, double-click to reset",
        ActiveResizer::Chat => "Drag to resize diff viewer / chat pane, double-click to reset",
        ActiveResizer::Terminal => "Drag to resize terminal, double-click to reset",
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
            role="separator" aria-orientation="vertical" tabindex="0"
            aria-label=format!("Resize {}", panel.label())
            aria-valuemin=kind.min() aria-valuemax=kind.max() aria-valuenow=move || width.get()
            on:keydown=move |event: web_sys::KeyboardEvent| {
                let direction = if layout.preferences.with_untracked(|prefs| prefs.side(panel.id())) == crate::state::responsive::PanelSide::Right { -1.0 } else { 1.0 };
                let delta = match event.key().as_str() { "ArrowLeft" => -20.0, "ArrowRight" => 20.0, _ => return };
                event.prevent_default();
                layout.width_revision.update(|revision| *revision += 1);
                width.set(layout.clamp_visible(kind, width.get_untracked() + delta * direction, layout.viewport_width.get_untracked()));
                actions.save_width.run(kind);
            }
            on:pointerdown=move |ev: PointerEvent| {
                if kind == ActiveResizer::None {
                    return;
                }
                ev.prevent_default();
                epoch.set(auth.generation.get_untracked());
                layout.width_revision.update(|revision| *revision += 1);
                start_x.set(ev.client_x());
                start_width.set(width.get());
                layout.active_resizer.set(kind);
            }
            on:dblclick=move |_| {
                if kind == ActiveResizer::None {
                    return;
                }
                layout.width_revision.update(|revision| *revision += 1);
                let default_width = kind.default();
                width.set(default_width);
                let viewport = window().inner_width().ok().and_then(|value| value.as_f64()).unwrap_or(1200.0);
                layout.fit(viewport);
                actions.save_width.run(kind);
            }
        />
    }
}
