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
