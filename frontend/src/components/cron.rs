use super::ui::{FormField, TextInput};
use leptos::prelude::*;

fn segments(expression: &str) -> [String; 5] {
    let mut parts = expression.split_whitespace();
    std::array::from_fn(|_| parts.next().unwrap_or_default().to_string())
}

/// A guided five-field cron editor that retains cron ranges, lists and steps.
#[component]
pub fn CronInput(
    #[prop(into)] value: Signal<String>,
    on_change: Callback<String>,
    #[prop(into)] disabled: Signal<bool>,
) -> impl IntoView {
    let fields = RwSignal::new(segments(&value.get_untracked()));
    Effect::new(move |_| {
        let expression = value.get();
        // Keep blank intermediate fields in their original positions while editing.
        if fields.with_untracked(|parts| parts.join(" ")) != expression {
            fields.set(segments(&expression));
        }
    });
    view! {
        <FormField label="Cron expression" group=true>
            <div class="cron-fields">
                {[ ("Minute", "Cron minute", "0–59"),
                   ("Hour", "Cron hour", "0–23"),
                   ("Day of month", "Cron day of month", "1–31"),
                   ("Month", "Cron month", "1–12 or JAN–DEC"),
                   ("Day of week", "Cron day of week", "0–7 or SUN–SAT") ]
                    .into_iter().enumerate().map(|(index, (label, input_label, hint))| view! {
                        <FormField label=label>
                            <TextInput label=input_label placeholder="*" maxlength=128
                                value=Signal::derive(move || fields.with(|parts| parts[index].clone()))
                                disabled=disabled
                                on_change=Callback::new(move |text: String| {
                                    if text.split_whitespace().count() == 5 {
                                        fields.set(segments(&text));
                                    } else {
                                        fields.update(|parts| parts[index] = text.trim().to_string());
                                    }
                                    on_change.run(fields.with_untracked(|parts| parts.join(" ")));
                                })/>
                            <span class="form-hint">{hint}</span>
                        </FormField>
                    }).collect::<Vec<_>>()}
            </div>
            <code class="cron-preview" aria-label="Cron expression preview">{move || value.get()}</code>
            <p class="form-hint">"* = any value · / = step · - = range · , = list. Sunday is 0 or 7. Paste a full expression into any field."</p>
        </FormField>
    }
}
