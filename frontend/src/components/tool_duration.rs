use leptos::prelude::*;
use openwebide_core::{ToolTiming, tool_timing::LiveToolTiming};
use std::time::Duration;

#[component]
pub fn ToolDuration(
    timing: Memo<Option<ToolTiming>>,
    #[prop(default = "Tool execution time, including checkpoints; approval waiting is excluded")]
    title: &'static str,
) -> impl IntoView {
    let clock = RwSignal::new(LiveToolTiming::default());
    let now = RwSignal::new(js_sys::Date::now());
    let timer = StoredValue::<Option<IntervalHandle>>::new(None);
    Effect::new(move |_| {
        let timing = timing.get();
        let timestamp = js_sys::Date::now();
        clock.update(|clock| clock.observe(timing, timestamp));
        now.set(timestamp);
        if timing.is_some_and(|timing| !timing.finished) {
            if timer.get_value().is_none() {
                timer.set_value(
                    set_interval_with_handle(
                        move || now.set(js_sys::Date::now()),
                        Duration::from_millis(100),
                    )
                    .ok(),
                );
            }
        } else {
            timer.update_value(|timer| {
                if let Some(timer) = timer.take() {
                    timer.clear();
                }
            });
        }
    });
    on_cleanup(move || {
        if let Some(timer) = timer.get_value() {
            timer.clear();
        }
    });
    view! { <Show when=move || timing.get().is_some()>
        <span class="tui-tool-duration muted" title=title>
            {move || format!("{:.1}s", clock.get().elapsed_ms(now.get()).unwrap_or_default() / 1000.0)}
        </span>
    </Show> }
}
