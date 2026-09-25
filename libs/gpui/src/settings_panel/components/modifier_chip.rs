use gpui::prelude::*;
use gpui::{
    rgb, rgba, App, ClickEvent, CursorStyle, ElementId, IntoElement, RenderOnce, Rgba,
    SharedString, Window,
};

use crate::kit::kit;
use crate::theme::{Ground, SettingsPanelPalette};

use super::{ground_bg, ground_border, ground_text, RowGround};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SettingsModifierChip {
    id: ElementId,
    label: SharedString,
    on: bool,
    cursor: bool,
    row: RowGround,
    palette: SettingsPanelPalette,
    on_click: Option<ClickHandler>,
}

impl SettingsModifierChip {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        on: bool,
        cursor: bool,
        row: RowGround,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            on,
            cursor,
            row,
            palette,
            on_click: None,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

fn chip_border(on: bool, cursor: bool, ground: Ground) -> Rgba {
    if cursor {
        rgb(ground.ink)
    } else if on {
        rgb(ground.soft)
    } else {
        rgba(ground.edge.packed())
    }
}

impl RenderOnce for SettingsModifierChip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let SettingsModifierChip {
            id,
            label,
            on,
            cursor,
            row,
            palette,
            on_click,
        } = self;
        let ground = row.rest(palette);
        let hover = row.hover(palette);
        let text = |ground: Ground| rgb(if on { ground.ink } else { ground.faint });
        let fill = |ground: Ground| {
            if on {
                rgba(ground.well.packed())
            } else {
                rgba(palette.transparent_rgba)
            }
        };
        let border = |ground: Ground| chip_border(on, cursor, ground);
        let chip = kit().keycap(label).id(id);
        let chip = ground_bg(chip, fill(ground), hover.map(fill));
        let chip = ground_text(chip, text(ground), hover.map(text));
        let chip = ground_border(chip, border(ground), hover.map(border));
        match on_click {
            Some(handler) => chip
                .cursor(CursorStyle::PointingHand)
                .on_click(move |event, window, cx| handler(event, window, cx)),
            None => chip,
        }
    }
}
