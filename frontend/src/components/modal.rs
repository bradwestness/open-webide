use super::ui::DialogSize;
use std::cell::RefCell;

use leptos::prelude::*;
use wasm_bindgen::{JsCast, closure::Closure};

struct ActiveModal {
    panel: web_sys::HtmlElement,
    opener: Option<web_sys::HtmlElement>,
    close: Callback<()>,
    observer: Option<web_sys::MutationObserver>,
    _mutation_listener: Closure<dyn Fn()>,
}

#[derive(Default)]
struct ModalStack {
    next_id: u64,
    entries: Vec<ActiveModal>,
    key_listener: Option<Closure<dyn Fn(web_sys::Event)>>,
    focus_listener: Option<Closure<dyn Fn(web_sys::Event)>>,
}

thread_local! {
    static MODALS: RefCell<ModalStack> = RefCell::new(ModalStack::default());
}

pub(crate) fn modal_is_open() -> bool {
    MODALS.with(|stack| !stack.borrow().entries.is_empty())
}

fn tabbable(panel: &web_sys::HtmlElement) -> Vec<web_sys::HtmlElement> {
    fn visit(parent: &web_sys::Element, elements: &mut Vec<web_sys::HtmlElement>) {
        let mut child = parent.first_element_child();
        while let Some(element) = child {
            if let Some(html) = element.dyn_ref::<web_sys::HtmlElement>()
                && html.tab_index() >= 0
                && !element.matches(":disabled, [hidden]").unwrap_or(true)
                && html.offset_width() + html.offset_height() > 0
                && web_sys::window()
                    .and_then(|window| window.get_computed_style(html).ok().flatten())
                    .is_some_and(|style| {
                        style.get_property_value("visibility").unwrap_or_default() != "hidden"
                    })
            {
                elements.push(html.clone());
            }
            visit(&element, elements);
            child = element.next_element_sibling();
        }
    }
    let mut elements = Vec::new();
    visit(panel, &mut elements);
    elements
}

fn focus_initial(panel: &web_sys::HtmlElement) {
    let elements = tabbable(panel);
    let target = elements
        .iter()
        .find(|element| element.matches("input, select, textarea").unwrap_or(false))
        .or_else(|| {
            elements.iter().find(|element| {
                element
                    .matches(".modal-footer .send, .modal-footer .danger")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(panel);
    let _ = target.focus();
}

fn top_panel() -> Option<web_sys::HtmlElement> {
    MODALS.with(|stack| {
        stack
            .borrow()
            .entries
            .last()
            .map(|entry| entry.panel.clone())
    })
}

fn handle_key(event: &web_sys::Event) {
    let Some(key) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
        return;
    };
    let Some(panel) = top_panel() else { return };
    // Capture prevents IDE and underlying modal controls from seeing this keystroke.
    if key.key() == "Escape" {
        event.stop_propagation();
        event.prevent_default();
        let close = MODALS.with(|stack| stack.borrow().entries.last().map(|entry| entry.close));
        if let Some(close) = close {
            close.run(());
        }
    } else if key.key() == "Tab" {
        event.stop_propagation();
        event.prevent_default();
        let elements = tabbable(&panel);
        let active = document().active_element();
        let index = elements.iter().position(|element| {
            active
                .as_ref()
                .is_some_and(|active| element.is_same_node(Some(active)))
        });
        let target = if key.shift_key() {
            index
                .and_then(|index| index.checked_sub(1))
                .or_else(|| elements.len().checked_sub(1))
        } else {
            index.map(|index| (index + 1) % elements.len()).or(Some(0))
        };
        let _ = target
            .and_then(|index| elements.get(index))
            .unwrap_or(&panel)
            .focus();
    }
}

fn update_layers(entries: &[ActiveModal]) {
    for (index, entry) in entries.iter().enumerate() {
        if let Some(overlay) = entry
            .panel
            .parent_element()
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = overlay
                .style()
                .set_property("z-index", &(100 + index).to_string());
        }
    }
}

fn register(panel: web_sys::HtmlElement, close: Callback<()>) {
    let opener = document()
        .active_element()
        .and_then(|element| element.dyn_into().ok());
    let observed_panel = panel.clone();
    // Removing a focused control can move focus to BODY without a focusin event.
    let mutation_listener = Closure::wrap(Box::new(move || {
        if observed_panel.is_connected()
            && top_panel().is_some_and(|top| top.is_same_node(Some(&observed_panel)))
            && document()
                .active_element()
                .is_none_or(|active| !observed_panel.contains(Some(&active)))
        {
            focus_initial(&observed_panel);
        }
    }) as Box<dyn Fn()>);
    let observer = web_sys::MutationObserver::new(mutation_listener.as_ref().unchecked_ref()).ok();
    if let Some(observer) = &observer {
        let options = web_sys::MutationObserverInit::new();
        options.set_child_list(true);
        options.set_subtree(true);
        let _ = observer.observe_with_options(&panel, &options);
    }
    MODALS.with(|stack| {
        let mut stack = stack.borrow_mut();
        if stack.entries.is_empty() {
            let key = Closure::wrap(
                Box::new(|event: web_sys::Event| handle_key(&event)) as Box<dyn Fn(_)>
            );
            let focus = Closure::wrap(Box::new(|_: web_sys::Event| {
                if let Some(panel) = top_panel()
                    && document()
                        .active_element()
                        .is_none_or(|active| !panel.contains(Some(&active)))
                {
                    focus_initial(&panel);
                }
            }) as Box<dyn Fn(_)>);
            let _ = document().add_event_listener_with_callback_and_bool(
                "keydown",
                key.as_ref().unchecked_ref(),
                true,
            );
            let _ = document().add_event_listener_with_callback_and_bool(
                "focusin",
                focus.as_ref().unchecked_ref(),
                true,
            );
            stack.key_listener = Some(key);
            stack.focus_listener = Some(focus);
        }
        stack.entries.push(ActiveModal {
            panel: panel.clone(),
            opener,
            close,
            observer,
            _mutation_listener: mutation_listener,
        });
        update_layers(&stack.entries);
    });
    focus_initial(&panel);
}

fn unregister(panel: &web_sys::HtmlElement) {
    let restore = MODALS.with(|stack| {
        let mut stack = stack.borrow_mut();
        let index = stack
            .entries
            .iter()
            .position(|entry| entry.panel.is_same_node(Some(panel)))?;
        let was_top = index + 1 == stack.entries.len();
        let removed = stack.entries.remove(index);
        if let Some(observer) = &removed.observer {
            observer.disconnect();
        }
        update_layers(&stack.entries);
        if stack.entries.is_empty() {
            for (name, listener) in [
                ("keydown", stack.key_listener.take()),
                ("focusin", stack.focus_listener.take()),
            ] {
                if let Some(listener) = listener {
                    let _ = document().remove_event_listener_with_callback_and_bool(
                        name,
                        listener.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
        }
        was_top.then_some(removed.opener)
    });
    let Some(restore) = restore else { return };
    if let Some(panel) = top_panel() {
        if let Some(opener) =
            restore.filter(|opener| opener.is_connected() && panel.contains(Some(opener)))
        {
            let _ = opener.focus();
        } else {
            focus_initial(&panel);
        }
    } else if let Some(opener) = restore.filter(|opener| opener.is_connected()) {
        let _ = opener.focus();
    }
}

#[component]
pub fn Modal(
    title: Signal<String>,
    on_close: Callback<()>,
    #[prop(optional)] description: Option<Signal<String>>,
    #[prop(optional)] describedby: Option<&'static str>,
    #[prop(default = "modal")] class: &'static str,
    #[prop(default = DialogSize::Standard)] size: DialogSize,
    children: Children,
) -> impl IntoView {
    let id = MODALS.with(|stack| {
        let mut stack = stack.borrow_mut();
        stack.next_id += 1;
        stack.next_id
    });
    let title_id = format!("modal-title-{id}");
    let description_id = format!("modal-description-{id}");
    let panel = NodeRef::<leptos::html::Div>::new();
    Effect::new(move || {
        if let Some(panel) = panel.get() {
            register((*panel).clone(), on_close);
        }
    });
    on_cleanup(move || {
        if let Some(panel) = panel.get_untracked() {
            unregister(&panel);
        }
    });
    view! {
        <div class="modal-overlay" on:click=move |_| {
            if panel.get().is_some_and(|panel| top_panel().is_some_and(|top| top.is_same_node(Some(&panel)))) { on_close.run(()); }
        }>
            <div class=format!("{class} {}", size.class_name()) node_ref=panel tabindex="-1" role="dialog" aria-modal="true"
                aria-labelledby=title_id.clone() aria-describedby=describedby.map(str::to_string).or_else(|| description.map(|_| description_id.clone()))
                on:click=move |event: web_sys::MouseEvent| event.stop_propagation()>
                <div class="modal-header">
                    <div>
                        <h2 id=title_id.clone()>{move || title.get()}</h2>
                        {description.map(|description| view! { <p class="modal-description" id=description_id.clone()>{move || description.get()}</p> })}
                    </div>
                    <button type="button" class="icon-btn" title="Close" aria-label="Close dialog" on:click=move |_| on_close.run(())><crate::components::ui::Icon name=crate::components::ui::IconName::X /></button>
                </div>
                {children()}
            </div>
        </div>
    }
}
