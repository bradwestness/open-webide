use leptos::prelude::*;

pub const CENTER_MIN: f64 = 260.0;

pub fn fit_panels(viewport: f64, widths: [f64; 3]) -> [f64; 3] {
    let panels = [
        ActiveResizer::Sidebar,
        ActiveResizer::Tree,
        ActiveResizer::Chat,
    ];
    let mut widths = std::array::from_fn(|i| widths[i].clamp(panels[i].min(), panels[i].max()));
    let mut excess = (widths.iter().sum::<f64>() + CENTER_MIN - viewport).max(0.0);
    for i in [2, 1, 0] {
        let shrink = excess.min(widths[i] - panels[i].min());
        widths[i] -= shrink;
        excess -= shrink;
    }
    widths
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
    pub terminal_cmd: RwSignal<Option<String>>,
    pub sidebar_width: RwSignal<f64>,
    pub tree_width: RwSignal<f64>,
    pub chat_width: RwSignal<f64>,
    pub active_resizer: RwSignal<ActiveResizer>,
}

impl LayoutState {
    pub fn new() -> Self {
        Self {
            terminal_cmd: RwSignal::new(None),
            sidebar_width: RwSignal::new(ActiveResizer::Sidebar.default()),
            tree_width: RwSignal::new(ActiveResizer::Tree.default()),
            chat_width: RwSignal::new(ActiveResizer::Chat.default()),
            active_resizer: RwSignal::new(ActiveResizer::None),
        }
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

    #[test]
    fn fit_shrinks_chat_then_tree_then_sidebar() {
        let fitted = fit_panels(1280.0, [480.0, 650.0, 1000.0]);
        assert_eq!(fitted, [480.0, 280.0, 260.0]);
        assert!(fitted.iter().sum::<f64>() <= 1020.0);
        assert_eq!(
            fit_panels(900.0, [240.0, 260.0, 420.0]),
            [220.0, 160.0, 260.0]
        );
    }

    #[test]
    fn wide_viewports_preserve_widths_and_tiny_viewports_use_minimums() {
        assert_eq!(
            fit_panels(3000.0, [480.0, 650.0, 1000.0]),
            [480.0, 650.0, 1000.0]
        );
        assert_eq!(
            fit_panels(100.0, [480.0, 650.0, 1000.0]),
            [140.0, 160.0, 260.0]
        );
        assert_eq!(
            fit_panels(3000.0, [1.0, 900.0, 2000.0]),
            [140.0, 650.0, 1000.0]
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
            assert_eq!(resizer.min(), min);
            assert_eq!(resizer.max(), max);
            assert_eq!(resizer.default(), default);
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
            assert_eq!(
                LayoutState::clamp(resizer, requested, sidebar, tree, chat, total),
                expected
            );
        }
    }
}
