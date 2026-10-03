use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Sessions,
    Files,
    Editor,
    Chat,
}
impl Panel {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Files => "files",
            Self::Editor => "editor",
            Self::Chat => "chat",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sessions => "Sessions",
            Self::Files => "Files",
            Self::Editor => "Editor",
            Self::Chat => "Chat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PanelVisibility {
    pub sessions: bool,
    pub files: bool,
    pub editor: bool,
    pub chat: bool,
}
impl Default for PanelVisibility {
    fn default() -> Self {
        Self {
            sessions: true,
            files: true,
            editor: true,
            chat: true,
        }
    }
}
impl PanelVisibility {
    pub const fn visible(self, panel: Panel) -> bool {
        match panel {
            Panel::Sessions => self.sessions,
            Panel::Files => self.files,
            Panel::Editor => self.editor,
            Panel::Chat => self.chat,
        }
    }
    pub fn set(&mut self, panel: Panel, visible: bool) {
        match panel {
            Panel::Sessions => self.sessions = visible,
            Panel::Files => self.files = visible,
            Panel::Editor => self.editor = visible,
            Panel::Chat => self.chat = visible,
        }
    }
}

pub const PANEL_VISIBILITY_KEY: &str = "panel_visibility";
pub const PANEL_RAILS_WIDTH: f64 = 36.0;

/// Fit only open panels, retaining the remembered width of collapsed panels.
pub fn fit_visible_panels(
    viewport: f64,
    widths: [f64; 3],
    visibility: PanelVisibility,
) -> [f64; 3] {
    let panels = [
        ActiveResizer::Sidebar,
        ActiveResizer::Tree,
        ActiveResizer::Chat,
    ];
    let visible = [visibility.sessions, visibility.files, visibility.chat];
    let mut widths = std::array::from_fn(|i| widths[i].clamp(panels[i].min(), panels[i].max()));
    let center = if visibility.editor { CENTER_MIN } else { 0.0 };
    let used = widths
        .iter()
        .enumerate()
        .filter(|(i, _)| visible[*i])
        .map(|(_, width)| width)
        .sum::<f64>();
    let mut excess = (used + center + PANEL_RAILS_WIDTH - viewport).max(0.0);
    for i in [2, 1, 0] {
        if visible[i] {
            let shrink = excess.min(widths[i] - panels[i].min());
            widths[i] -= shrink;
            excess -= shrink;
        }
    }
    widths
}

pub const CENTER_MIN: f64 = 260.0;

pub fn fit_panels(viewport: f64, widths: [f64; 3]) -> [f64; 3] {
    fit_visible_panels(
        viewport + PANEL_RAILS_WIDTH,
        widths,
        PanelVisibility::default(),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ActiveResizer {
    #[default]
    None,
    Sidebar,
    Tree,
    Chat,
}

impl ActiveResizer {
    pub const fn min(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Sidebar => 140.0,
            Self::Tree => 160.0,
            Self::Chat => 260.0,
        }
    }

    pub const fn max(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Sidebar => 480.0,
            Self::Tree => 650.0,
            Self::Chat => 1000.0,
        }
    }

    pub const fn default(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Sidebar => 240.0,
            Self::Tree => 260.0,
            Self::Chat => 420.0,
        }
    }

    pub const fn setting_key(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Sidebar => "panel_sidebar_width",
            Self::Tree => "panel_tree_width",
            Self::Chat => "panel_chat_width",
        }
    }
}

/// User-adjustable widths for the three resizable panels.
#[derive(Clone, Copy)]
pub struct LayoutState {
    pub panels: RwSignal<PanelVisibility>,
    pub panel_revision: RwSignal<u64>,
    pub terminal_cmd: RwSignal<Option<String>>,
    pub sidebar_width: RwSignal<f64>,
    pub tree_width: RwSignal<f64>,
    pub chat_width: RwSignal<f64>,
    pub active_resizer: RwSignal<ActiveResizer>,
}

impl LayoutState {
    pub fn new() -> Self {
        Self {
            panels: RwSignal::new(PanelVisibility::default()),
            panel_revision: RwSignal::new(0),
            terminal_cmd: RwSignal::new(None),
            sidebar_width: RwSignal::new(ActiveResizer::Sidebar.default()),
            tree_width: RwSignal::new(ActiveResizer::Tree.default()),
            chat_width: RwSignal::new(ActiveResizer::Chat.default()),
            active_resizer: RwSignal::new(ActiveResizer::None),
        }
    }

    pub fn fit(&self, viewport: f64) {
        let [sidebar, tree, chat] = fit_visible_panels(
            viewport,
            [
                self.sidebar_width.get_untracked(),
                self.tree_width.get_untracked(),
                self.chat_width.get_untracked(),
            ],
            self.panels.get_untracked(),
        );
        self.sidebar_width.set(sidebar);
        self.tree_width.set(tree);
        self.chat_width.set(chat);
    }

    pub fn restore_panels(&self, value: Option<&String>) {
        if self.panel_revision.get_untracked() == 0 {
            self.panels.set(
                value
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_default(),
            );
        }
    }

    pub fn clamp_visible(&self, resizer: ActiveResizer, requested: f64, viewport: f64) -> f64 {
        let panels = self.panels.get_untracked();
        let sidebar = if panels.sessions {
            self.sidebar_width.get_untracked()
        } else {
            0.0
        };
        let tree = if panels.files {
            self.tree_width.get_untracked()
        } else {
            0.0
        };
        let chat = if panels.chat {
            self.chat_width.get_untracked()
        } else {
            0.0
        };
        let viewport = viewport - PANEL_RAILS_WIDTH + if panels.editor { 0.0 } else { CENTER_MIN };
        Self::clamp(resizer, requested, sidebar, tree, chat, viewport)
    }

    /// Clamp a requested panel width to its bounds while keeping room for the
    /// center editor, if the existing outer panels leave enough room.
    pub fn clamp(
        resizer: ActiveResizer,
        requested: f64,
        sidebar_width: f64,
        tree_width: f64,
        chat_width: f64,
        total_width: f64,
    ) -> f64 {
        if resizer == ActiveResizer::None {
            return requested;
        }

        let other_width = match resizer {
            ActiveResizer::None => 0.0,
            ActiveResizer::Sidebar => tree_width + chat_width,
            ActiveResizer::Tree => sidebar_width + chat_width,
            ActiveResizer::Chat => sidebar_width + tree_width,
        };
        let lower = resizer.min();
        let upper = resizer.max();
        let width = requested.clamp(lower, upper);
        let max_for_center = (total_width - other_width - CENTER_MIN).max(lower);

        width.min(max_for_center)
    }
}

impl Default for LayoutState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_widths(actual: [f64; 3], expected: [f64; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn collapsed_panels_keep_their_width_and_release_center_space() {
        let hidden = PanelVisibility {
            sessions: false,
            files: false,
            editor: true,
            chat: true,
        };
        assert_widths(
            fit_visible_panels(900.0, [480.0, 650.0, 800.0], hidden),
            [480.0, 650.0, 604.0],
        );
        let chat_only = PanelVisibility {
            editor: false,
            ..hidden
        };
        assert_widths(
            fit_visible_panels(900.0, [480.0, 650.0, 800.0], chat_only),
            [480.0, 650.0, 800.0],
        );
        assert_widths(
            fit_visible_panels(100.0, [480.0, 650.0, 800.0], hidden),
            [480.0, 650.0, 260.0],
        );
    }

    #[test]
    fn fit_shrinks_chat_then_tree_then_sidebar() {
        let fitted = fit_panels(1280.0, [480.0, 650.0, 1000.0]);
        assert_widths(fitted, [480.0, 280.0, 260.0]);
        assert!(fitted.iter().sum::<f64>() <= 1020.0);
        assert_widths(
            fit_panels(900.0, [240.0, 260.0, 420.0]),
            [220.0, 160.0, 260.0],
        );
    }

    #[test]
    fn wide_viewports_preserve_widths_and_tiny_viewports_use_minimums() {
        assert_widths(
            fit_panels(3000.0, [480.0, 650.0, 1000.0]),
            [480.0, 650.0, 1000.0],
        );
        assert_widths(
            fit_panels(100.0, [480.0, 650.0, 1000.0]),
            [140.0, 160.0, 260.0],
        );
        assert_widths(
            fit_panels(3000.0, [1.0, 900.0, 2000.0]),
            [140.0, 650.0, 1000.0],
        );
    }

    #[test]
    fn resizer_bounds_defaults_and_settings_match_the_panel_table() {
        let cases = [
            (
                ActiveResizer::Sidebar,
                140.0,
                480.0,
                240.0,
                "panel_sidebar_width",
            ),
            (ActiveResizer::Tree, 160.0, 650.0, 260.0, "panel_tree_width"),
            (
                ActiveResizer::Chat,
                260.0,
                1000.0,
                420.0,
                "panel_chat_width",
            ),
        ];

        for (resizer, min, max, default, setting_key) in cases {
            assert!((resizer.min() - min).abs() < f64::EPSILON);
            assert!((resizer.max() - max).abs() < f64::EPSILON);
            assert!((resizer.default() - default).abs() < f64::EPSILON);
            assert_eq!(resizer.setting_key(), setting_key);
        }
    }

    #[test]
    fn clamp_respects_panel_bounds_and_center_minimum() {
        let cases = [
            (
                ActiveResizer::Sidebar,
                10.0,
                240.0,
                260.0,
                420.0,
                1200.0,
                140.0,
            ),
            (
                ActiveResizer::Tree,
                900.0,
                240.0,
                260.0,
                420.0,
                5000.0,
                650.0,
            ),
            (
                ActiveResizer::Chat,
                1100.0,
                240.0,
                260.0,
                420.0,
                5000.0,
                1000.0,
            ),
            (
                ActiveResizer::Chat,
                600.0,
                240.0,
                260.0,
                420.0,
                1200.0,
                440.0,
            ),
        ];

        for (resizer, requested, sidebar, tree, chat, total, expected) in cases {
            assert!(
                (LayoutState::clamp(resizer, requested, sidebar, tree, chat, total) - expected)
                    .abs()
                    < f64::EPSILON
            );
        }
    }
}
