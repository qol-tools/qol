use gpui::prelude::*;
use gpui::{div, px, rgba, App, RenderOnce, SharedString, Window};

use crate::kit::Kit;

use super::choice_value::SettingsChevron;
use super::{ground_text, RowGround, SettingsToggle};

#[derive(IntoElement)]
pub struct SettingsFeature {
    on: bool,
    count: SharedString,
    row: RowGround,
    kit: Kit,
}

impl SettingsFeature {
    pub fn new(on: bool, count: impl Into<SharedString>, row: RowGround, kit: Kit) -> Self {
        Self {
            on,
            count: count.into(),
            row,
            kit,
        }
    }
}

impl RenderOnce for SettingsFeature {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ground = self.row.rest(self.kit);
        let hover = self.row.hover(self.kit);
        let (line, word, word_hover) = match self.row {
            RowGround::Pane => (self.kit.washes.hairline_strong.packed(), ground.faint, None),
            RowGround::Band => (
                ground.edge.packed(),
                ground.ink,
                hover.map(|hover| hover.ink),
            ),
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_CELL))
            .child(SettingsToggle::new(self.on, self.row, self.kit))
            .child(
                div()
                    .flex_none()
                    .mx(px(qol_theme::SPACE_TIGHT))
                    .w(px(qol_theme::LINE))
                    .h(px(qol_theme::HEIGHT_INLINE))
                    .bg(rgba(line)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(qol_theme::SPACE_INSET))
                    .child(ground_text(
                        self.kit.value(self.count),
                        gpui::rgb(word),
                        word_hover.map(gpui::rgb),
                    ))
                    .child(SettingsChevron::new(self.row, self.kit)),
            )
    }
}
