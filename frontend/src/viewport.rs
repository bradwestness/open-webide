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
    const chrome = [...pane.children].filter(el => el !== composer && !el.matches('.messages, .tui-empty-state, .chat-welcome')).reduce((sum, el) => sum + el.getBoundingClientRect().height, 0);
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

#[wasm_bindgen(inline_js = r#"
export function observe_editor_viewport(input, overlay, onLayout) {
    const pane = input.parentElement;
    let frame = 0, fontChanged = false, active = true;
    const update = () => {
        frame = 0;
        if (!active || !input.isConnected || !overlay.isConnected) return;
        overlay.style.setProperty('--editor-viewport-height', `${input.clientHeight}px`);
        overlay.style.setProperty('--editor-text-width', `${input.clientWidth}px`);
        const rows = overlay.querySelectorAll('.editor-source-line');
        const grips = pane.querySelectorAll('.editor-fold-row');
        rows.forEach((row, index) => grips[index]?.style.setProperty('--editor-row-height', `${row.getBoundingClientRect().height}px`));
        const changed = fontChanged; fontChanged = false;
        onLayout(changed);
    };
    const schedule = (changed = false) => { fontChanged ||= changed; if (active && !frame) frame = requestAnimationFrame(update); };
    const observer = new ResizeObserver(() => schedule());
    observer.observe(input);
    const mutation = new MutationObserver(() => schedule());
    mutation.observe(overlay, {childList: true, subtree: true});
    const preferences = new MutationObserver(() => schedule());
    preferences.observe(pane, {attributes: true, attributeFilter: ['class', 'style']});
    preferences.observe(document.documentElement, {attributes: true});
    const fontsChanged = () => schedule(true);
    document.fonts?.addEventListener('loadingdone', fontsChanged);
    document.fonts?.ready.then(() => { if (active) fontsChanged(); });
    update();
    return () => { active = false; cancelAnimationFrame(frame); observer.disconnect(); mutation.disconnect(); preferences.disconnect(); document.fonts?.removeEventListener('loadingdone', fontsChanged); };
}
"#)]
extern "C" {
    pub fn observe_editor_viewport(
        input: &web_sys::HtmlTextAreaElement,
        overlay: &web_sys::HtmlElement,
        on_layout: &js_sys::Function,
    ) -> JsValue;
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = r#"
export function editor_point(input, x, y) {
    const paint = input.parentElement?.querySelector('.editor-highlight-content');
    const overlay = paint?.parentElement;
    if (!paint || !overlay) return undefined;
    const inputEvents = input.style.pointerEvents, paintEvents = overlay.style.pointerEvents;
    try {
        input.style.pointerEvents = 'none'; overlay.style.pointerEvents = 'auto';
        const position = document.caretPositionFromPoint?.(x, y);
        const caret = position ? {startContainer:position.offsetNode, startOffset:position.offset} : document.caretRangeFromPoint?.(x, y);
        if (!caret || !paint.contains(caret.startContainer)) return undefined;
        return [caret.startContainer, caret.startOffset];
    } finally {
        input.style.pointerEvents = inputEvents; overlay.style.pointerEvents = paintEvents;
    }
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    fn editor_point(
        input: &web_sys::HtmlTextAreaElement,
        x: f64,
        y: f64,
    ) -> Result<JsValue, JsValue>;
}

#[cfg(target_arch = "wasm32")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "DOM point offsets are checked for integer value and the u32 range before conversion"
)]
pub fn editor_caret_from_point(
    input: &web_sys::HtmlTextAreaElement,
    x: f64,
    y: f64,
) -> Option<u32> {
    let paint = input
        .parent_element()?
        .query_selector(".editor-highlight-content")
        .ok()??;
    if paint.get_attribute("data-editor-scope") != input.get_attribute("data-editor-scope") {
        return None;
    }
    let point = editor_point(input, x, y).ok()?;
    if !js_sys::Array::is_array(&point) {
        return None;
    }
    let point = point.unchecked_into::<js_sys::Array>();
    if point.length() != 2 {
        return None;
    }
    let node = point.get(0).dyn_into::<web_sys::Node>().ok()?;
    let offset = point.get(1).as_f64()?;
    if !offset.is_finite()
        || offset.fract() != 0.0
        || !(0.0..=f64::from(u32::MAX)).contains(&offset)
    {
        return None;
    }
    crate::components::editor_paint::native_offset(&paint, &node, offset as u32)
}
