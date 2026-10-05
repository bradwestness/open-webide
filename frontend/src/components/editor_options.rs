//! Shared indentation controls for user defaults and the active document.
use leptos::prelude::*;
use openwebide_core::editor::{IndentStyle, Indentation};

use super::dropdown::{DropdownSelect, SelectOption};
use super::ui::{SegmentOption, SegmentedControl};

#[component]
pub fn IndentationControls(
    value: Signal<Indentation>,
    on_change: Callback<Indentation>,
    #[prop(default = false)] above: bool,
    #[prop(into)] disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="editor-indentation-options" role="group" aria-label="Indentation settings">
            <SegmentedControl options=vec![SegmentOption::new("Spaces", IndentStyle::Spaces).disabled_when(disabled), SegmentOption::new("Tabs", IndentStyle::Tabs).disabled_when(disabled)] value=Signal::derive(move || value.get().style) on_change=Callback::new(move |style| {
                let mut indentation = value.get_untracked(); indentation.style = style; on_change.run(indentation);
            }) />
            <DropdownSelect label="Indentation width" above=above trigger_class="btn ghost sm" disabled=disabled value=Signal::derive(move || value.get().width().to_string()) options=Signal::derive(|| (1..=16).map(|width| SelectOption::new(width.to_string(), format!("Indent: {width}"))).collect()) on_change=Callback::new(move |width: String| {
                if let Ok(width) = width.parse() { let mut indentation = value.get_untracked(); indentation.width = width; on_change.run(indentation); }
            }) />
            <DropdownSelect label="Tab width" above=above trigger_class="btn ghost sm" disabled=disabled value=Signal::derive(move || value.get().tab_width().to_string()) options=Signal::derive(|| (1..=16).map(|width| SelectOption::new(width.to_string(), format!("Tab: {width}"))).collect()) on_change=Callback::new(move |width: String| {
                if let Ok(width) = width.parse() { let mut indentation = value.get_untracked(); indentation.tab_width = width; on_change.run(indentation); }
            }) />
        </div>
    }
}
