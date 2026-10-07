//! Reusable UI primitive components and design system for Open WebIDE.

use leptos::prelude::*;

/// The shared brand mark, with its color and size supplied by theme classes.
#[component]
pub fn LogoMark(#[prop(default = "")] class: &'static str) -> impl IntoView {
    view! {
        <span class=format!("logo-mark {class}") aria-hidden="true" inner_html=include_str!("../../pwa/logo.svg") />
    }
}

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
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
}

impl ButtonVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Primary => "send",
            Self::Success => "approve",
            Self::Danger => "danger",
            Self::Ghost => "ghost",
        }
    }
}

impl ButtonSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Sm => "sm",
            Self::Md => "md",
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
    #[prop(default = "button")] button_type: &'static str,
    children: Children,
) -> impl IntoView {
    let extra_class = class.unwrap_or_default();
    let is_disabled = move || disabled.map(|d| d.get()).unwrap_or(false);

    view! {
        <button
            type=button_type
            class=format!("btn {} {} {}", variant.class_name(), size.class_name(), extra_class)
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
    pub disabled: Option<Signal<bool>>,
}

impl<T: Clone + PartialEq + Send + Sync + 'static> SegmentOption<T> {
    pub fn new(label: impl Into<String>, value: T) -> Self {
        Self {
            label: label.into(),
            value,
            glyph: None,
            disabled: None,
        }
    }
    pub fn disabled_when(mut self, disabled: Signal<bool>) -> Self {
        self.disabled = Some(disabled);
        self
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
        <div class=format!("ui-segmented-control {extra_class}") role="group">
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
                        disabled=move || opt.disabled.is_some_and(|disabled| disabled.get())
                        aria-pressed=move || (value.get() == opt_val).to_string()
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

/// Scrollable dialog content; the header and actions remain visible.
#[component]
pub fn DialogBody(#[prop(default = "")] class: &'static str, children: Children) -> impl IntoView {
    view! { <div class=format!("modal-body {class}")>{children()}</div> }
}

/// The dialog's final actions, ordered secondary first and primary last.
#[component]
pub fn DialogActions(children: Children) -> impl IntoView {
    view! { <div class="modal-footer ui-actions">{children()}</div> }
}

/// A named group of related fields with optional supporting guidance.
#[component]
pub fn FormSection(
    title: &'static str,
    #[prop(default = "")] description: &'static str,
    #[prop(default = "")] class: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <section class=format!("ui-section {class}") aria-label=title>
            <header class="ui-section-heading">
                <h3>{title}</h3>
                {(!description.is_empty()).then(|| view! { <p class="form-hint">{description}</p> })}
            </header>
            <div class="ui-section-content">{children()}</div>
        </section>
    }
}

/// Label one native control, or use a fieldset for a group of radio choices.
/// Keep buttons outside a single-control field to avoid nested labelable elements.
#[component]
pub fn FormField(
    label: &'static str,
    #[prop(default = false)] group: bool,
    children: Children,
) -> impl IntoView {
    if group {
        view! {
            <fieldset class="setting-row ui-field ui-field-group">
                <legend class="setting-label">{label}</legend>
                {children()}
            </fieldset>
        }
        .into_any()
    } else {
        view! {
            <label class="setting-row ui-field">
                <span class="setting-label">{label}</span>
                {children()}
            </label>
        }
        .into_any()
    }
}

/// Related inline actions, distinct from a dialog's final commit action.
#[component]
pub fn InlineActions(children: Children) -> impl IntoView {
    view! { <div class="ui-inline-actions">{children()}</div> }
}

#[derive(Clone, Copy, Default)]
pub enum NoticeTone {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// Supporting feedback with shared theme colors and announcement semantics.
#[component]
pub fn FormNotice(
    #[prop(default = NoticeTone::Info)] tone: NoticeTone,
    children: Children,
) -> impl IntoView {
    let (class, role) = match tone {
        NoticeTone::Info => ("ui-notice", None),
        NoticeTone::Success => ("ui-notice ui-notice-success", Some("status")),
        NoticeTone::Warning => ("ui-notice ui-notice-warning", None),
        NoticeTone::Error => ("ui-notice form-error", Some("alert")),
    };
    view! { <div class=class role=role>{children()}</div> }
}

/// A native checkbox with a descriptive label and shared control spacing.
#[component]
pub fn CheckboxField(
    label: &'static str,
    checked: Signal<bool>,
    on_change: Callback<bool>,
    #[prop(into, optional)] disabled: Option<Signal<bool>>,
) -> impl IntoView {
    view! {
        <label class="ui-check">
            <input type="checkbox" prop:checked=move || checked.get() disabled=move || disabled.is_some_and(|disabled| disabled.get())
                on:change=move |event| on_change.run(event_target_checked(&event)) />
            <span>{label}</span>
        </label>
    }
}

#[derive(Clone, Copy, Default)]
pub enum DialogSize {
    Small,
    #[default]
    Standard,
    Wide,
}
impl DialogSize {
    pub fn class_name(self) -> &'static str {
        match self {
            Self::Small => "modal-sm",
            Self::Standard => "",
            Self::Wide => "modal-wide",
        }
    }
}

/// A compact labeled action. The glyph is decorative; its accessible name is explicit.
#[component]
pub fn IconButton(
    #[prop(into)] label: Signal<String>,
    on_click: Callback<web_sys::MouseEvent>,
    #[prop(default = "")] class: &'static str,
    #[prop(into, optional)] disabled: Option<Signal<bool>>,
    children: Children,
) -> impl IntoView {
    view! {
        <button type="button" class=format!("icon-btn ui-icon {class}")
            title=move || label.get() aria-label=move || label.get()
            disabled=move || disabled.is_some_and(|disabled| disabled.get())
            on:click=move |event| on_click.run(event)>
            <span aria-hidden="true">{children()}</span>
        </button>
    }
}

static NEXT_DISCLOSURE_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A disclosure that retains mounted content, including live tool state.
/// Approval requests can force it open without changing the user's expansion choice.
#[component]
pub fn DisclosurePanel(
    #[prop(into)] summary: ViewFn,
    #[prop(default = "")] class: &'static str,
    #[prop(into, optional)] force_open: Option<Signal<bool>>,
    #[prop(default = "")] toggle_class: &'static str,
    #[prop(default = "")] title: &'static str,
    #[prop(into, optional)] active: Option<Signal<bool>>,
    children: Children,
) -> impl IntoView {
    let content_id = format!(
        "disclosure-{}",
        NEXT_DISCLOSURE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let expanded = RwSignal::new(false);
    let open = move || expanded.get() || force_open.is_some_and(|force| force.get());
    view! {
        <div class=format!("ui-disclosure-panel {class}")>
            <button type="button" class=format!("ui-disclosure-toggle {toggle_class}") title=title class:active=move || active.is_some_and(|value| value.get()) aria-expanded=move || open().to_string() aria-controls=content_id.clone()
                on:click=move |_| { if !force_open.is_some_and(|force| force.get_untracked()) { expanded.update(|expanded| *expanded = !*expanded); } }>
                <span class="ui-disclosure-caret" aria-hidden="true"><Icon name=Signal::derive(move || if open() { IconName::ChevronUp } else { IconName::ChevronDown }) /></span>
                {summary.run()}
            </button>
            <div class="ui-disclosure-content" id=content_id hidden=move || !open()>{children()}</div>
        </div>
    }
}

/// One SVG family, color and size contract for application controls.
pub use lepticons::LucideGlyph as IconName;

#[component]
pub fn Icon(#[prop(into)] name: Signal<IconName>) -> impl IntoView {
    view! { <span class="ui-icon-glyph" aria-hidden="true"><lepticons::Icon glyph=name size="20" stroke_width="2" /></span> }
}

/// Actions and contextual information below a panel's single shared heading.
#[component]
pub fn PanelToolbar(
    #[prop(default = "")] class: &'static str,
    children: Children,
) -> impl IntoView {
    view! { <div class=format!("panel-toolbar {class}") data-context-menu="">{children()}</div> }
}

/// Consistent search input and inline actions for tool panels.
#[component]
pub fn PanelSearchRow(
    #[prop(default = "")] class: &'static str,
    children: Children,
) -> impl IntoView {
    view! { <div class=format!("panel-search-row {class}")><Icon name=IconName::Search />{children()}</div> }
}
