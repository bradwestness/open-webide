//! Shared layout preferences; viewport and browser drag events are adapter inputs.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const LAYOUT_PREFERENCES_KEY: &str = "workspace_layout";
pub const PHONE_BREAKPOINT: f64 = 800.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    #[default]
    Automatic,
    Desktop,
    Phone,
}
impl LayoutMode {
    pub fn phone(self, width: f64) -> bool {
        match self {
            Self::Automatic => width.is_finite() && (0.0..PHONE_BREAKPOINT).contains(&width),
            Self::Desktop => false,
            Self::Phone => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelSide {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutPreferences {
    pub mode: LayoutMode,
    sides: BTreeMap<String, PanelSide>,
}
impl LayoutPreferences {
    pub fn side(&self, panel: &str) -> PanelSide {
        self.sides
            .get(panel)
            .copied()
            .unwrap_or(if matches!(panel, "chat" | "terminal") {
                PanelSide::Right
            } else {
                PanelSide::Left
            })
    }
    pub fn order(&self, panel: &str) -> u8 {
        let rank = match panel {
            "sessions" => 1,
            "files" => 2,
            "search" => 3,
            "git" => 4,
            "editor" => 5,
            "terminal" => 6,
            _ => 8,
        };
        match self.sides.get(panel) {
            Some(PanelSide::Left) => rank,
            Some(PanelSide::Right) => 80 + rank,
            None => rank * 10,
        }
    }
    pub fn pin(&mut self, panel: &str, side: PanelSide) -> bool {
        if ![
            "sessions", "files", "editor", "chat", "terminal", "git", "search",
        ]
        .contains(&panel)
        {
            return false;
        }
        self.sides.insert(panel.into(), side);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_layout_and_explicit_override_have_the_same_boundary_in_every_workspace() {
        for width in [320.0, 390.0, 799.0] {
            assert!(LayoutMode::Automatic.phone(width));
        }
        for width in [800.0, 1280.0, f64::NAN, f64::INFINITY] {
            assert!(!LayoutMode::Automatic.phone(width));
        }
        assert!(LayoutMode::Phone.phone(1920.0));
        assert!(!LayoutMode::Desktop.phone(320.0));
    }
    #[test]
    fn pin_preferences_keep_defaults_and_roundtrip_without_viewport_state() {
        let mut preferences = LayoutPreferences::default();
        assert_eq!(preferences.side("chat"), PanelSide::Right);
        assert_eq!(preferences.side("files"), PanelSide::Left);
        assert!(preferences.pin("chat", PanelSide::Left));
        assert!(preferences.pin("terminal", PanelSide::Right));
        assert!(!preferences.pin("unknown", PanelSide::Right));
        preferences.mode = LayoutMode::Phone;
        let encoded = serde_json::to_string(&preferences).unwrap();
        assert_eq!(
            serde_json::from_str::<LayoutPreferences>(&encoded).unwrap(),
            preferences
        );
        assert_eq!(
            serde_json::from_str::<LayoutPreferences>("{}").unwrap(),
            LayoutPreferences::default()
        );
    }
}
