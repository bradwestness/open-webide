use super::{
    modal::Modal,
    model_setup::ModelSetupPanel,
    ui::{Button, DialogActions, DialogBody, DialogSize, FormField, FormSection, InlineActions},
};
use crate::state::settings::{SettingsState, Theme};
use leptos::prelude::*;
use web_sys::wasm_bindgen::JsCast;

/// User preferences and the model / system prompt defaults for new sessions.
#[component]
pub fn Settings(
    on_set_theme: Callback<Theme>,
    on_set_notifications: Callback<bool>,
    on_set_default_prompt: Callback<Option<i64>>,
    on_set_bridge_url: Callback<String>,
) -> impl IntoView {
    let layout = expect_context::<crate::state::layout::LayoutState>();
    let layout_actions = use_context::<crate::state_actions::layout::LayoutActions>();
    let settings = expect_context::<SettingsState>();
    let theme = settings.theme.read_only();
    let notifications = crate::notifications::RunNotifications::from_context();
    let default_prompt = settings.default_prompt.read_only();
    let system_prompts = settings.system_prompts.read_only();
    let bridge_url = settings.bridge_url.read_only();
    let on_close = Callback::new(move |()| settings.show_settings.set(false));
    let (pairing_token, set_pairing_token) = leptos::prelude::signal(String::new());
    Effect::new(move || {
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(Some(pt)) = crate::idb::get_bridge_pairing_token().await {
                set_pairing_token.set(pt);
            }
        });
    });

    view! {
        <Modal title=Signal::derive(|| "Settings".to_string()) on_close=on_close size=DialogSize::Wide>
            <DialogBody>
                <FormSection title="Appearance" description="Changes apply immediately." class="ui-form-grid">
                <FormField label="Layout" group=true>
                    <super::ui::SegmentedControl options=vec![super::ui::SegmentOption::new("Automatic", crate::state::responsive::LayoutMode::Automatic), super::ui::SegmentOption::new("Desktop", crate::state::responsive::LayoutMode::Desktop), super::ui::SegmentOption::new("Phone", crate::state::responsive::LayoutMode::Phone)] value=Signal::derive(move || layout.preferences.with(|prefs| prefs.mode)) on_change=Callback::new(move |mode| { if let Some(actions) = layout_actions { actions.set_mode.run(mode); } }) />
                </FormField>
                <FormField label="Theme" group=true>
                    <super::ui::SegmentedControl options=vec![super::ui::SegmentOption::new("System", Theme::System), super::ui::SegmentOption::new("Dark", Theme::Dark), super::ui::SegmentOption::new("Light", Theme::Light)] value=Signal::derive(move || theme.get()) on_change=on_set_theme />
                </FormField>

                </FormSection>
                <FormSection title="Notifications">
                <FormField label="Browser notifications" group=true>
                    <InlineActions><button class="btn ghost notification-toggle" disabled=move || notifications.configuring.get()
                        on:click=move |_| on_set_notifications.run(!(settings.browser_notifications.get_untracked() && notifications.permission.get_untracked() != crate::notifications::NotificationPermission::Default))>
                        {move || if notifications.configuring.get() { "Saving…" } else if settings.browser_notifications.get() && notifications.permission.get() != crate::notifications::NotificationPermission::Default { "Disable" } else { "Enable" }}
                    </button>
                    </InlineActions>
                </FormField>
                <p class="form-hint notification-status">{move || {
                    use crate::notifications::NotificationPermission;
                    match notifications.permission.get() {
                        NotificationPermission::Unsupported => "Notifications need a supported browser on HTTPS or localhost. Chat approvals remain available in the app.",
                        NotificationPermission::Denied => "Notifications are blocked in this browser's site settings.",
                        NotificationPermission::Default if settings.browser_notifications.get() => "Enable on this browser to receive run and approval notifications.",
                        _ if settings.browser_notifications.get() => "Notify when a run finishes or needs approval while its chat is out of focus. Keep this app open to receive notifications.",
                        _ => "Enable notifications for finished runs and approval requests. Browser permission is required on each device.",
                    }
                }}</p>
                </FormSection>
                <FormSection title="Model defaults" description="Choose the models and system prompt used for new chats.">
                <ModelSetupPanel defaults_only=true />
                <FormField label="Default system prompt">
                    <super::dropdown::DropdownSelect label="Default system prompt" value=Signal::derive(move || default_prompt.get().map(|id| id.to_string()).unwrap_or_default()) options=Signal::derive(move || {
                        let mut options = vec![super::dropdown::SelectOption::new("", "(none)")];
                        options.extend(system_prompts.get().into_iter().map(|prompt| super::dropdown::SelectOption::new(prompt.id.to_string(), prompt.name))); options
                    }) on_change=Callback::new(move |value: String| on_set_default_prompt.run(value.parse().ok())) />
                </FormField>

                </FormSection>
                <FormSection title="Installation">
                    <super::install_app::InstallApp />
                </FormSection>
                <FormSection title="Execution bridge" description="Connect terminal and agent tools to your execution host.">
                <FormField label="Bridge URL">
                    <input
                        type="text"
                        class="form-input"
                        value=bridge_url
                        on:change=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                            {
                                on_set_bridge_url.run(input.value());
                            }
                        }
                    />
                </FormField>

                <FormField label="Bridge pairing token" group=true>
                    <div class="ui-input-group">
                        <input
                            type="password"
                            aria-label="Bridge pairing token"
                            class="form-input"
                            value=pairing_token
                            on:change=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    let val = input.value();
                                    set_pairing_token.set(val.clone());
                                    wasm_bindgen_futures::spawn_local(async move {
                                        let _ = crate::idb::set_bridge_pairing_token(&val).await;
                                    });
                                }
                            }
                        />
                        <button
                            class="btn"
                            on:click=move |_| {
                                set_pairing_token.set(String::new());
                                wasm_bindgen_futures::spawn_local(async move {
                                    let _ = crate::idb::delete_bridge_pairing_token().await;
                                });
                            }
                        >
                            "Clear"
                        </button>
                    </div>
                    <div class="form-hint">
                        "Required only if your bridge daemon was started with OPENWEBIDE_BRIDGE_TOKEN to allow local execution (the laptop-companion case)."
                    </div>
                </FormField>

                </FormSection>
            </DialogBody>
            <DialogActions>
                <Button on_click=Callback::new(move |_| on_close.run(()))>"Done"</Button>
            </DialogActions>
        </Modal>
    }
}
