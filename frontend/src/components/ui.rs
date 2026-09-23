//! Reusable UI primitive components and design system for Open WebIDE.

use leptos::prelude::*;

/// Visual variant for buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    #[default]
    Default,
    Primary,
    Success,
    Danger,
    Ghost,
}

/// Size variant for buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
}

impl ButtonVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Default => "ui-btn-default",
            Self::Primary => "ui-btn-primary",
            Self::Success => "ui-btn-success",
            Self::Danger => "ui-btn-danger",
            Self::Ghost => "ui-btn-ghost",
        }
    }
}

impl ButtonSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Sm => "ui-btn-sm",
            Self::Md => "ui-btn-md",
            Self::Lg => "ui-btn-lg",
        }
    }
}

/// A button primitive following Open WebIDE's developer TUI design tokens.
#[component]
pub fn Button(
    #[prop(default = ButtonVariant::Default)] variant: ButtonVariant,
    #[prop(default = ButtonSize::Md)] size: ButtonSize,
    #[prop(into, optional)] class: Option<String>,
    #[prop(into, optional)] disabled: Option<Signal<bool>>,
    #[prop(into, optional)] on_click: Option<Callback<web_sys::MouseEvent>>,
    children: Children,
) -> impl IntoView {
    let extra_class = class.unwrap_or_default();
    let is_disabled = move || disabled.map(|d| d.get()).unwrap_or(false);

    view! {
        <button
            class=format!("ui-btn {} {} {}", variant.class_name(), size.class_name(), extra_class)
            disabled=is_disabled
            on:click=move |e| {
                if let Some(cb) = &on_click {
                    cb.run(e);
                }
            }
        >
            {children()}
        </button>
    }
}

/// A segmented button item inside a [`SegmentedControl`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentOption<T: Clone + PartialEq + Send + Sync + 'static> {
    pub label: String,
    pub value: T,
    pub glyph: Option<String>,
}

impl<T: Clone + PartialEq + Send + Sync + 'static> SegmentOption<T> {
    pub fn new(label: impl Into<String>, value: T) -> Self {
        Self {
            label: label.into(),
            value,
            glyph: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_glyph(label: impl Into<String>, glyph: impl Into<String>, value: T) -> Self {
        Self {
            label: label.into(),
            value,
            glyph: Some(glyph.into()),
        }
    }
}

/// A compact, monospace segmented toggle control (e.g. [Inline | Split | Content | Preview]).
#[component]
pub fn SegmentedControl<T: Clone + PartialEq + Send + Sync + 'static>(
    options: Vec<SegmentOption<T>>,
    value: Signal<T>,
    on_change: Callback<T>,
    #[prop(into, optional)] class: Option<String>,
) -> impl IntoView {
    let extra_class = class.unwrap_or_default();

    view! {
        <div class=format!("ui-segmented-control {extra_class}")>
            {options.into_iter().map(|opt| {
                let opt_val = opt.value.clone();
                let opt_val_click = opt.value.clone();
                let is_active = {
                    let opt_val = opt_val.clone();
                    move || value.get() == opt_val
                };

                view! {
                    <button
                        type="button"
                        class=move || {
                            if is_active() {
                                "ui-seg-btn active"
                            } else {
                                "ui-seg-btn"
                            }
                        }
                        on:click=move |_| on_change.run(opt_val_click.clone())
                    >
                        {if let Some(g) = &opt.glyph {
                            view! { <span class="ui-seg-glyph">{g.clone()}</span> }.into_any()
                        } else {
                            ().into_any()
                        }}
                        <span class="ui-seg-label">{opt.label}</span>
                    </button>
                }
            }).collect::<Vec<_>>()}
        </div>
    }
}
