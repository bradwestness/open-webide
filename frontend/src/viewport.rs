//! Browser viewport primitives; layout policy remains in the shared layout store.
use leptos::prelude::*;
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(inline_js = r#"
export function observe_visible_height() {
    const viewport = window.visualViewport;
    const update = () => document.documentElement.style.setProperty('--visible-height', `${viewport?.height || window.innerHeight}px`);
    window.addEventListener('resize', update);
    viewport?.addEventListener('resize', update);
    update();
    return () => {
        window.removeEventListener('resize', update);
        viewport?.removeEventListener('resize', update);
        document.documentElement.style.removeProperty('--visible-height');
    };
}
"#)]
extern "C" {
    fn observe_visible_height() -> JsValue;
}

/// Keep the phone composer above on-screen keyboards, including Safari's overlay viewport.
pub fn install() {
    let stop = StoredValue::new_local(observe_visible_height());
    on_cleanup(move || {
        stop.with_value(|callback| {
            let _ = callback
                .unchecked_ref::<js_sys::Function>()
                .call0(&JsValue::NULL);
        });
    });
}

#[wasm_bindgen(inline_js = r#"
export function fit_composer(input) {
    const pane = input.closest('.chat-pane');
    if (!pane || !pane.clientHeight || !input.clientWidth) return;
    const composer = input.closest('.composer');
    const chrome = [...pane.children].filter(el => el !== composer && !el.classList.contains('messages') && !el.classList.contains('tui-empty-state')).reduce((sum, el) => sum + el.getBoundingClientRect().height, 0);
    const maximum = Math.max(44, Math.min(pane.clientHeight * .8, pane.clientHeight - chrome - 100));
    input.style.overflowY = input.value ? 'auto' : 'hidden';
    if (!input.value) { input.style.height = '44px'; return; }
    input.style.height = '0px';
    input.style.height = `${Math.min(maximum, Math.max(44, input.scrollHeight + 2))}px`;
}
export function observe_composer(input) {
    const pane = input.closest('.chat-pane');
    let frame = 0;
    const update = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(() => fit_composer(input)); };
    const observer = new ResizeObserver(update);
    observer.observe(pane);
    let width = 0;
    const inputObserver = new ResizeObserver(() => { if (input.clientWidth !== width) { width = input.clientWidth; update(); } });
    inputObserver.observe(input);
    const mutation = new MutationObserver(update);
    mutation.observe(pane, {childList: true});
    input.addEventListener('input', update);
    update();
    return () => { cancelAnimationFrame(frame); observer.disconnect(); inputObserver.disconnect(); mutation.disconnect(); input.removeEventListener('input', update); };
}
"#)]
extern "C" {
    pub fn fit_composer(input: &web_sys::HtmlTextAreaElement);
    fn observe_composer(input: &web_sys::HtmlTextAreaElement) -> JsValue;
}

pub fn install_composer(input: NodeRef<leptos::html::Textarea>) {
    let stop = StoredValue::new_local(None::<JsValue>);
    Effect::new(move |_| {
        if let Some(input) = input.get()
            && stop.get_value().is_none()
        {
            stop.set_value(Some(observe_composer(&input)));
        }
    });
    on_cleanup(move || {
        stop.with_value(|stop| {
            if let Some(stop) = stop {
                let _ = stop
                    .unchecked_ref::<js_sys::Function>()
                    .call0(&JsValue::NULL);
            }
        });
    });
}

#[wasm_bindgen(inline_js = r#"
export function install_tooltips() {
    const tip = document.createElement('div');
    tip.className = 'ui-tooltip'; tip.id = 'ui-action-tooltip'; tip.setAttribute('role', 'tooltip'); tip.hidden = true;
    document.body.append(tip);
    let current = null, timer = 0, title = null, described = null;
    const hide = () => {
        clearTimeout(timer); tip.hidden = true;
        if (current) {
            if (title !== null) current.setAttribute('title', title);
            if (described === null) current.removeAttribute('aria-describedby'); else current.setAttribute('aria-describedby', described);
        }
        current = null; title = null;
    };
    const enter = event => {
        const target = event.target instanceof Element ? event.target.closest('button[title],button[data-tooltip]') : null;
        if (!target || current === target) return;
        hide(); current = target; title = target.getAttribute('title'); described = target.getAttribute('aria-describedby');
        const text = target.getAttribute('data-tooltip') || title;
        if (!text) return;
        target.removeAttribute('title');
        if (!target.hasAttribute('aria-label') && target.classList.contains('icon-btn')) target.setAttribute('aria-label', text);
        timer = setTimeout(() => {
            if (!target.isConnected) { hide(); return; }
            tip.textContent = text; tip.hidden = false;
            const rect = target.getBoundingClientRect();
            const width = tip.offsetWidth, height = tip.offsetHeight;
            tip.style.left = `${Math.max(8, Math.min(innerWidth - width - 8, rect.left + (rect.width - width) / 2))}px`;
            tip.style.top = `${rect.bottom + height + 8 < innerHeight ? rect.bottom + 6 : Math.max(8, rect.top - height - 6)}px`;
            target.setAttribute('aria-describedby', [described, tip.id].filter(Boolean).join(' '));
        }, event.type === 'focus' ? 0 : 200);
    };
    const leave = event => { if (current && !(event.relatedTarget instanceof Node && current.contains(event.relatedTarget))) hide(); };
    const key = event => { if (event.key === 'Escape') hide(); };
    document.addEventListener('pointerover', enter); document.addEventListener('pointerout', leave);
    document.addEventListener('focus', enter, true); document.addEventListener('blur', leave, true);
    document.addEventListener('pointerdown', hide); document.addEventListener('keydown', key);
    const scroll = () => { const target = current; hide(); if (target && document.activeElement === target) enter({target, type: 'focus'}); };
    window.addEventListener('scroll', scroll, true); window.addEventListener('resize', hide);
    return () => {
        hide(); tip.remove(); document.removeEventListener('pointerover', enter); document.removeEventListener('pointerout', leave);
        document.removeEventListener('focus', enter, true); document.removeEventListener('blur', leave, true);
        document.removeEventListener('pointerdown', hide); document.removeEventListener('keydown', key);
        window.removeEventListener('scroll', scroll, true); window.removeEventListener('resize', hide);
    };
}
"#)]
extern "C" {
    fn install_tooltips() -> JsValue;
}

pub fn install_action_tooltips() {
    let stop = StoredValue::new_local(install_tooltips());
    on_cleanup(move || {
        stop.with_value(|stop| {
            let _ = stop
                .unchecked_ref::<js_sys::Function>()
                .call0(&JsValue::NULL);
        });
    });
}
