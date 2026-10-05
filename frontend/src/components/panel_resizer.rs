use leptos::prelude::*;
use web_sys::PointerEvent;

use crate::state::auth::AuthState;
use crate::state::layout::{ActiveResizer, LayoutState, Panel};
use crate::state_actions::layout::LayoutActions;

#[component]
pub fn PanelResizer(
    kind: ActiveResizer,
    #[prop(optional)] panel: Option<Panel>,
    #[prop(default = false)] invert: bool,
    #[prop(default = None)] partner: Option<ActiveResizer>,
) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    let actions = expect_context::<LayoutActions>();
    let auth = expect_context::<AuthState>();
    let handle = NodeRef::<leptos::html::Div>::new();
    let epoch = RwSignal::new(0_u64);
    let dragging = RwSignal::new(false);
    let panel = panel.unwrap_or(match kind {
        ActiveResizer::Sidebar => Panel::Sessions,
        ActiveResizer::Tree => Panel::Files,
        ActiveResizer::Chat => Panel::Chat,
        ActiveResizer::Terminal => Panel::Terminal,
        ActiveResizer::None => Panel::Editor,
    });
    let horizontal = kind == ActiveResizer::Terminal;
    let direction = if invert || horizontal { -1.0 } else { 1.0 };
    let start_x = RwSignal::new(0.0f64);
    let start_total = RwSignal::new(0.0);
    let start_width = RwSignal::new(0.0f64);
    let width = if kind == ActiveResizer::None {
        RwSignal::new(0.0)
    } else {
        layout.width(kind)
    };

    let set_size = move |requested: f64, total: f64| {
        let value = if let Some(partner) = partner {
            requested.clamp(
                kind.min().max(total - partner.max()),
                kind.max().min(total - partner.min()),
            )
        } else if horizontal {
            let available = handle
                .get_untracked()
                .and_then(|element| element.parent_element())
                .and_then(|panel| panel.parent_element())
                .map_or_else(
                    || {
                        window()
                            .inner_height()
                            .ok()
                            .and_then(|value| value.as_f64())
                            .unwrap_or(900.0)
                            - 260.0
                    },
                    |body| body.get_bounding_client_rect().height() - 180.0,
                );
            requested.clamp(kind.min(), kind.max().min(available.max(kind.min())))
        } else {
            layout.clamp_visible(kind, requested, layout.viewport_width.get_untracked())
        };
        width.set(value);
        if let Some(partner) = partner {
            layout.width(partner).set(total - value);
        }
    };
    let save_sizes = move || {
        actions.save_width.run(kind);
        if let Some(partner) = partner {
            actions.save_width.run(partner);
        }
    };

    let pointer_move = window_event_listener(leptos::ev::pointermove, move |ev: PointerEvent| {
        if !dragging.get_untracked()
            || layout.active_resizer.get() != kind
            || kind == ActiveResizer::None
            || auth.generation.get_untracked() != epoch.get_untracked()
        {
            return;
        }

        let current_x = if horizontal {
            ev.client_y()
        } else {
            ev.client_x()
        };
        let delta = (current_x - start_x.get()) * direction;
        let requested = start_width.get() + delta;
        set_size(requested, start_total.get());
    });

    let pointer_up = window_event_listener(leptos::ev::pointerup, move |_| {
        if !dragging.get_untracked()
            || layout.active_resizer.get() != kind
            || kind == ActiveResizer::None
            || auth.generation.get_untracked() != epoch.get_untracked()
        {
            return;
        }

        dragging.set(false);
        layout.active_resizer.set(ActiveResizer::None);
        save_sizes();
    });

    let pointer_cancel = window_event_listener(leptos::ev::pointercancel, move |_| {
        if dragging.get_untracked() && layout.active_resizer.get_untracked() == kind {
            dragging.set(false);
            layout.active_resizer.set(ActiveResizer::None);
            if auth.generation.get_untracked() == epoch.get_untracked() {
                save_sizes();
            }
        }
    });
    on_cleanup(move || {
        pointer_move.remove();
        pointer_up.remove();
        pointer_cancel.remove();
        if dragging.try_get_untracked() == Some(true)
            && layout.active_resizer.try_get_untracked() == Some(kind)
        {
            layout.active_resizer.set(ActiveResizer::None);
        }
    });

    let title = if invert {
        "Drag to resize pane, double-click to reset adjacent panel"
    } else {
        match kind {
            ActiveResizer::Sidebar => "Drag to resize sidebar, double-click to reset",
            ActiveResizer::Tree => "Drag to resize file tree / diff viewer, double-click to reset",
            ActiveResizer::Chat => "Drag to resize diff viewer / chat pane, double-click to reset",
            ActiveResizer::Terminal => "Drag to resize terminal, double-click to reset",
            ActiveResizer::None => "",
        }
    };

    view! {
        <div
            class=move || {
                if dragging.get() && layout.active_resizer.get() == kind {
                    "panel-resizer is-active"
                } else {
                    "panel-resizer"
                }
            }
            title=title node_ref=handle
            role="separator" aria-orientation=if horizontal { "horizontal" } else { "vertical" } tabindex="0"
            aria-label=format!("Resize {}", panel.label())
            aria-valuemin=kind.min() aria-valuemax=kind.max() aria-valuenow=move || width.get()
            on:keydown=move |event: web_sys::KeyboardEvent| {
                let delta = match (horizontal, event.key().as_str()) {
                    (false, "ArrowLeft") | (true, "ArrowUp") => -20.0,
                    (false, "ArrowRight") | (true, "ArrowDown") => 20.0,
                    _ => return,
                };
                event.prevent_default();
                layout.width_revision.update(|revision| *revision += 1);
                let requested = width.get_untracked() + delta * direction;
                let total = width.get_untracked() + partner.map_or(0.0, |partner| layout.width(partner).get_untracked());
                set_size(requested, total);
                save_sizes();
            }
            on:pointerdown=move |ev: PointerEvent| {
                if kind == ActiveResizer::None {
                    return;
                }
                ev.prevent_default();
                dragging.set(true);
                epoch.set(auth.generation.get_untracked());
                layout.width_revision.update(|revision| *revision += 1);
                start_x.set(if horizontal { ev.client_y() } else { ev.client_x() });
                start_width.set(width.get());
                start_total.set(width.get_untracked() + partner.map_or(0.0, |partner| layout.width(partner).get_untracked()));
                layout.active_resizer.set(kind);
            }
            on:dblclick=move |_| {
                if kind == ActiveResizer::None {
                    return;
                }
                layout.width_revision.update(|revision| *revision += 1);
                let default_width = kind.default();
                let total = width.get_untracked() + partner.map_or(0.0, |partner| layout.width(partner).get_untracked());
                set_size(default_width, total);
                let viewport = window().inner_width().ok().and_then(|value| value.as_f64()).unwrap_or(1200.0);
                layout.fit(viewport);
                save_sizes();
            }
        />
    }
}
