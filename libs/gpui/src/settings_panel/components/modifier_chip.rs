use gpui::prelude::*;
use gpui::{
    px, rgb, rgba, App, ClickEvent, CursorStyle, ElementId, IntoElement, RenderOnce, Rgba,
    SharedString, Window,
};

use crate::kit::Chip;
use crate::kit::Kit;
use crate::text::TextStyled;
use crate::theme::Ground;
use qol_theme::TextStyle;

use super::{RowGround, SETTINGS_ROW_GROUP};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SettingsModifierChip {
    id: ElementId,
    label: SharedString,
    on: bool,
    cursor: bool,
    row: RowGround,
    kit: Kit,
    on_click: Option<ClickHandler>,
}

impl SettingsModifierChip {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        on: bool,
        cursor: bool,
        row: RowGround,
        kit: Kit,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            on,
            cursor,
            row,
            kit,
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

fn chip_fill(on: bool, ground: Ground) -> Rgba {
    if on {
        rgba(qol_theme::translucent(ground.mark, qol_theme::Alpha::Edge))
    } else {
        rgba(0)
    }
}

fn chip_text(on: bool, ground: Ground) -> Rgba {
    rgb(if on { ground.ink } else { ground.faint })
}

fn chip_border(cursor: bool, ground: Ground) -> Rgba {
    if cursor {
        rgb(ground.soft)
    } else {
        rgba(0)
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
            kit,
            on_click,
        } = self;
        let ground = row.rest(kit);
        let hover = row.hover(kit);
        let lit = hover.unwrap_or(ground);
        let chip = kit
            .chip(Chip::Tag(label), ground)
            .id(id)
            .text(TextStyle::Key)
            .border(px(qol_theme::LINE))
            .px(px(qol_theme::SPACE_INSET + qol_theme::LINE))
            .rounded_full()
            .bg(chip_fill(on, ground))
            .text_color(chip_text(on, ground))
            .border_color(chip_border(cursor, ground))
            .when_some(hover, |chip, hover| {
                chip.group_hover(SETTINGS_ROW_GROUP, move |style| {
                    style
                        .bg(chip_fill(on, hover))
                        .text_color(chip_text(on, hover))
                        .border_color(chip_border(cursor, hover))
                })
            })
            .when(!on, |chip| {
                chip.hover(move |style| style.bg(rgba(lit.well.packed())).text_color(rgb(lit.soft)))
            });
        match on_click {
            Some(handler) => chip
                .cursor(CursorStyle::PointingHand)
                .on_click(move |event, window, cx| handler(event, window, cx)),
            None => chip,
        }
    }
}
