use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, BoxShadow, IntoElement, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::kit::{FOCUS_RING_EDGE, FOCUS_RING_HALO};
use crate::theme::SettingsPanelPalette;

use super::{ground_bg, ground_text, RowGround};

#[derive(IntoElement)]
pub struct SettingsKeyCombination {
    text: SharedString,
    focused: bool,
    recording: bool,
    row: RowGround,
    palette: SettingsPanelPalette,
}

impl SettingsKeyCombination {
    pub fn new(
        text: impl Into<SharedString>,
        focused: bool,
        recording: bool,
        row: RowGround,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            text: text.into(),
            focused,
            recording,
            row,
            palette,
        }
    }
}

impl RenderOnce for SettingsKeyCombination {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ground = self.row.rest(self.palette);
        let hover = self.row.hover(self.palette);
        let editing = self.focused || self.recording;
        let (text, text_hover) = if self.text.is_empty() {
            (ground.faint, hover.map(|hover| hover.faint))
        } else {
            match self.row {
                RowGround::Pane => (ground.soft, None),
                RowGround::Band => (ground.ink, hover.map(|hover| hover.ink)),
            }
        };
        let combo = div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(qol_theme::HEIGHT_INLINE))
            .px(px(qol_theme::SPACE_INSET))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .border(px(if editing { FOCUS_RING_EDGE } else { 1.0 }))
            .border_color(if editing {
                rgb(ground.ink)
            } else {
                rgba(ground.edge.packed())
            })
            .when(editing, |combo| {
                combo.shadow(vec![BoxShadow {
                    color: rgba(qol_theme::css_rgba_milli(ground.ink, 160).packed()).into(),
                    offset: gpui::point(px(0.0), px(0.0)),
                    blur_radius: px(0.0),
                    spread_radius: px(FOCUS_RING_HALO),
                }])
            })
            .text(TextStyle::Key);
        ground_bg(
            combo,
            rgba(ground.well.packed()),
            hover.map(|hover| rgba(hover.well.packed())),
        )
        .child(ground_text(div(), rgb(text), text_hover.map(rgb)).child(self.text))
    }
}
