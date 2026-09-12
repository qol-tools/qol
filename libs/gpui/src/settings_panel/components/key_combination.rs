use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, IntoElement, RenderOnce, SharedString, Window};

use crate::kit::{alpha, kit};
use crate::theme::SettingsPanelPalette;

#[derive(IntoElement)]
pub struct SettingsKeyCombination {
    text: SharedString,
    focused: bool,
    recording: bool,
    palette: SettingsPanelPalette,
}

impl SettingsKeyCombination {
    pub fn new(
        text: impl Into<SharedString>,
        focused: bool,
        recording: bool,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            focused,
            recording,
            palette,
        }
    }
}

impl RenderOnce for SettingsKeyCombination {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(qol_theme::HEIGHT_INLINE))
            .px(px(qol_theme::SPACE_INSET))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .border(px(1.0))
            .border_color(rgb(if self.focused || self.recording {
                self.palette.row_border_selected
            } else {
                self.palette.panel_border
            }))
            .bg(rgba(if self.recording {
                shared.washes.wash_selected.packed()
            } else {
                alpha(self.palette.dropdown_bg, 0xff)
            }))
            .when(self.focused || self.recording, |combo| {
                combo.shadow(shared.focus_ring())
            })
            .font_family(SharedString::from(qol_theme::font_mono()))
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(if self.text.is_empty() {
                self.palette.status_muted
            } else {
                self.palette.section_text
            }))
            .child(self.text)
    }
}
