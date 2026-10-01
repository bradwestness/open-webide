use leptos::prelude::*;
use openwebide_core::{Connection, ProviderKind, SystemPrompt};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    Dark,
    Light,
    #[default]
    System,
}

impl Theme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "dark" => Self::Dark,
            "light" => Self::Light,
            "system" => Self::System,
            _ => Self::Dark,
        }
    }
}

/// User preferences and editable connection and prompt configuration.
#[derive(Clone, Copy)]
pub struct SettingsState {
    pub show_settings: RwSignal<bool>,
    pub theme: RwSignal<Theme>,
    pub default_connection: RwSignal<Option<i64>>,
    pub default_prompt: RwSignal<Option<i64>>,
    pub bridge_url: RwSignal<String>,
    pub connections: RwSignal<Vec<Connection>>,
    pub system_prompts: RwSignal<Vec<SystemPrompt>>,
    pub show_prompt_form: RwSignal<bool>,
    pub prompt_edit_id: RwSignal<Option<i64>>,
    pub prompt_name: RwSignal<String>,
    pub prompt_content: RwSignal<String>,
    pub show_conn_form: RwSignal<bool>,
    pub conn_edit_id: RwSignal<Option<i64>>,
    pub conn_name: RwSignal<String>,
    pub conn_kind: RwSignal<ProviderKind>,
    pub conn_base_url: RwSignal<String>,
    pub conn_model: RwSignal<String>,
    pub conn_context_limit: RwSignal<String>,
}

impl SettingsState {
    pub fn new(theme: Theme, bridge_url: String) -> Self {
        Self {
            show_settings: RwSignal::new(false),
            theme: RwSignal::new(theme),
            default_connection: RwSignal::new(None),
            default_prompt: RwSignal::new(None),
            bridge_url: RwSignal::new(bridge_url),
            connections: RwSignal::new(Vec::new()),
            system_prompts: RwSignal::new(Vec::new()),
            show_prompt_form: RwSignal::new(false),
            prompt_edit_id: RwSignal::new(None),
            prompt_name: RwSignal::new(String::new()),
            prompt_content: RwSignal::new(String::new()),
            show_conn_form: RwSignal::new(false),
            conn_edit_id: RwSignal::new(None),
            conn_name: RwSignal::new(String::new()),
            conn_kind: RwSignal::new(ProviderKind::Ollama),
            conn_base_url: RwSignal::new(String::new()),
            conn_model: RwSignal::new(String::new()),
            conn_context_limit: RwSignal::new(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_values_round_trip_and_unknown_values_default_to_dark() {
        Owner::new().with(|| {
            for theme in [Theme::Dark, Theme::Light, Theme::System] {
                assert_eq!(Theme::parse(theme.as_str()), theme);
            }
            assert_eq!(Theme::parse("unknown"), Theme::Dark);
        });
    }
}
