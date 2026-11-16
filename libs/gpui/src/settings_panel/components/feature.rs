use gpui::prelude::*;
use gpui::{div, px, rgba, App, RenderOnce, Window};

use crate::kit::Kit;

use super::choice_value::SettingsChevron;
use super::{ChoiceArt, RowGround, SettingsToggle, SettingsWell};

#[derive(IntoElement)]
pub struct SettingsFeature {
    on: bool,
    art: ChoiceArt,
    row: RowGround,
    kit: Kit,
}

impl SettingsFeature {
    pub fn new(on: bool, art: ChoiceArt, row: RowGround, kit: Kit) -> Self {
        Self { on, art, row, kit }
    }
}

impl RenderOnce for SettingsFeature {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ground = self.row.rest(self.kit);
        let line = match self.row {
            RowGround::Pane => self.kit.washes.hairline_strong.packed(),
            RowGround::Band => ground.edge.packed(),
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
                    .child(SettingsWell::new(self.art, self.row, self.kit))
                    .child(SettingsChevron::new(self.row, self.kit)),
            )
    }
}
