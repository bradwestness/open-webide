//! Browser adapter for per-run response formatting defaults.
use openwebide_core::BrowserPreferences;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
export function browserPreferences() {
    const result = {};
    try {
        // Request an hour so resolvedOptions includes the default hour cycle.
        const options = new Intl.DateTimeFormat(undefined, {hour: 'numeric'}).resolvedOptions();
        result.timezone = options.timeZone;
        result.locale = options.locale;
        result.hour_cycle = options.hourCycle;
    } catch (_) {}
    try {
        const offset = new Date().getTimezoneOffset();
        if (Number.isFinite(offset)) result.utc_offset_minutes = -offset;
    } catch (_) {}
    return JSON.stringify(result);
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = browserPreferences, catch)]
    fn browser_preferences() -> Result<String, JsValue>;
}

pub fn capture() -> Option<BrowserPreferences> {
    let preferences: BrowserPreferences =
        serde_json::from_str(&browser_preferences().ok()?).ok()?;
    (preferences != BrowserPreferences::default()).then_some(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn snapshot_has_browser_formatting_defaults_and_current_offset() {
        let preferences = capture().expect("browser defaults are available");
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"hour".into(), &"numeric".into()).unwrap();
        let resolved =
            js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &options).resolved_options();
        assert_eq!(
            preferences.timezone,
            js_sys::Reflect::get(&resolved, &"timeZone".into())
                .unwrap()
                .as_string()
        );
        assert_eq!(
            preferences.locale,
            js_sys::Reflect::get(&resolved, &"locale".into())
                .unwrap()
                .as_string()
        );
        assert_eq!(
            preferences.hour_cycle,
            js_sys::Reflect::get(&resolved, &"hourCycle".into())
                .unwrap()
                .as_string()
        );
        assert!(
            (f64::from(preferences.utc_offset_minutes.unwrap())
                + js_sys::Date::new_0().get_timezone_offset())
            .abs()
                < f64::EPSILON
        );
    }
}
