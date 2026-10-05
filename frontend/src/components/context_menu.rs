//! Shared pointer/touch/keyboard gesture handling for existing action menus.
use leptos::prelude::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, closure::Closure};

type Listener = (&'static str, Closure<dyn FnMut(web_sys::Event)>);
struct Binding {
    target: web_sys::HtmlElement,
    listeners: Vec<Listener>,
    state: Rc<RefCell<Press>>,
}
#[derive(Default)]
struct Press {
    origin: Option<(f64, f64)>,
    consumed: bool,
    timer: Option<leptos::leptos_dom::helpers::TimeoutHandle>,
}
impl Press {
    fn cancel(&mut self) {
        self.origin = None;
        if let Some(timer) = self.timer.take() {
            timer.clear();
        }
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        self.state.borrow_mut().cancel();
        for (name, callback) in &self.listeners {
            let _ = self.target.remove_event_listener_with_callback_and_bool(
                name,
                callback.as_ref().unchecked_ref(),
                true,
            );
        }
    }
}
fn ignored(event: &web_sys::Event, target: &web_sys::HtmlElement) -> bool {
    let Some(element) = event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
    else {
        return true;
    };
    if element
        .closest(
            "input, textarea, [contenteditable=true], .ui-dropdown-menu, .ui-dropdown-backdrop",
        )
        .ok()
        .flatten()
        .is_some()
    {
        return true;
    }
    element
        .closest("[data-context-menu]")
        .ok()
        .flatten()
        .is_some_and(|closest| !closest.is_same_node(Some(target.as_ref())))
}

/// Bind to a reactive DOM target and remove listeners/timers when it changes.
/// Native capture listeners prevent a completed long press from also selecting,
/// expanding or submitting the row through its ordinary click handler.
pub fn context_menu_target(
    target: impl Fn() -> Option<web_sys::HtmlElement> + Send + Sync + 'static,
    show: Callback<Option<(f64, f64)>>,
) {
    let binding = StoredValue::new_local(None::<Binding>);
    Effect::new(move |_| {
        let target = target();
        binding.set_value(None);
        let Some(target) = target else {
            return;
        };
        let state = Rc::new(RefCell::new(Press::default()));
        let mut listeners = Vec::<Listener>::new();
        for name in [
            "contextmenu",
            "pointerdown",
            "pointermove",
            "pointerup",
            "pointercancel",
            "click",
            "keydown",
        ] {
            let target_clone = target.clone();
            let state_clone = state.clone();
            let callback = Closure::new(move |event: web_sys::Event| {
                if ignored(&event, &target_clone) {
                    return;
                }
                match name {
                    "contextmenu" => {
                        event.prevent_default();
                        event.stop_propagation();
                        state_clone.borrow_mut().cancel();
                        if let Some(event) = event.dyn_ref::<web_sys::MouseEvent>() {
                            show.run(Some((event.client_x(), event.client_y())));
                        }
                    }
                    "pointerdown" => {
                        let mut state = state_clone.borrow_mut();
                        state.cancel();
                        state.consumed = false;
                        let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>() else {
                            return;
                        };
                        if !matches!(pointer.pointer_type().as_str(), "touch" | "pen") {
                            return;
                        }
                        if event
                            .target()
                            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                            .is_some_and(|target| {
                                target
                                    .closest(".ui-dropdown-trigger")
                                    .ok()
                                    .flatten()
                                    .is_some()
                            })
                        {
                            return;
                        }
                        let point = (pointer.client_x(), pointer.client_y());
                        state.origin = Some(point);
                        let pressed = state_clone.clone();
                        state.timer = leptos::leptos_dom::helpers::set_timeout_with_handle(
                            move || {
                                pressed.borrow_mut().consumed = true;
                                show.run(Some(point));
                            },
                            std::time::Duration::from_millis(500),
                        )
                        .ok();
                    }
                    "pointermove" => {
                        let mut state = state_clone.borrow_mut();
                        if let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>()
                            && let Some((x, y)) = state.origin
                            && ((pointer.client_x() - x).abs() > 8.0
                                || (pointer.client_y() - y).abs() > 8.0)
                        {
                            state.cancel();
                        }
                    }
                    "pointerup" | "pointercancel" => state_clone.borrow_mut().cancel(),
                    "click" if state_clone.borrow().consumed => {
                        state_clone.borrow_mut().consumed = false;
                        event.prevent_default();
                        event.stop_immediate_propagation();
                    }
                    "keydown" => {
                        let Some(key) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
                            return;
                        };
                        if key.key() == "ContextMenu" || (key.key() == "F10" && key.shift_key()) {
                            event.prevent_default();
                            event.stop_immediate_propagation();
                            show.run(None);
                        }
                    }
                    _ => {}
                }
            });
            if target
                .add_event_listener_with_callback_and_bool(
                    name,
                    callback.as_ref().unchecked_ref(),
                    true,
                )
                .is_ok()
            {
                listeners.push((name, callback));
            }
        }
        binding.set_value(Some(Binding {
            target,
            listeners,
            state,
        }));
    });
    on_cleanup(move || {
        binding.try_update_value(|binding| *binding = None);
    });
}
