//! Bottom-pane overlays: settings, theme picker, approval.

use crate::content_render::Theme;

#[derive(Clone, Debug, Default)]
pub enum Overlay {
    #[default]
    None,
    ThemePicker {
        selected: usize,
    },
    Settings,
    Approval {
        id: String,
        tool_name: String,
        message: String,
    },
    FilePicker,
}

impl Overlay {
    pub fn is_active(&self) -> bool {
        !matches!(self, Overlay::None)
    }
}

pub const THEMES: &[Theme] = &[Theme::Dark, Theme::Light, Theme::Ansi];

pub fn theme_label(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "Dark",
        Theme::Light => "Light",
        Theme::Ansi => "Ansi",
    }
}
