use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, SharedString};
use qol_theme::TextStyle;

use crate::kit::Kit;
use crate::kit::{kit, Chip};

pub struct DisplayLayoutTile {
    pub connector: String,
    pub resolution: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub selected: bool,
    pub primary: bool,
    pub conflicted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayLayoutTileStyle {
    pub border_color: u32,
    pub background: u32,
}

pub fn display_layout_tile_style(tile: &DisplayLayoutTile, kit: Kit) -> DisplayLayoutTileStyle {
    let border_color = if tile.conflicted {
        kit.palette.danger
    } else if tile.selected {
        kit.grounds.pane.mark
    } else {
        kit.palette.border_subtle
    };
    DisplayLayoutTileStyle {
        border_color,
        background: if tile.selected {
            kit.grounds.band.bg
        } else {
            kit.grounds.menu.bg
        },
    }
}

pub fn display_layout_stage(kit: Kit, height: f32) -> gpui::Div {
    div()
        .relative()
        .w_full()
        .h(px(height))
        .overflow_hidden()
        .rounded(px(qol_theme::RADIUS_CARD))
        .bg(rgb(kit.grounds.menu.bg))
}

pub fn display_layout_ghost(left: f32, top: f32, width: f32, height: f32, kit: Kit) -> gpui::Div {
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .h(px(height))
        .rounded(px(qol_theme::RADIUS_TIGHT))
        .border(px(qol_theme::LINE))
        .border_color(rgb(kit.grounds.pane.mark))
        .bg(rgb(kit.grounds.band.bg))
        .opacity(qol_theme::OPACITY_REST)
}

pub fn display_layout_tile(
    index: usize,
    tile: &DisplayLayoutTile,
    kit: Kit,
) -> gpui::Stateful<gpui::Div> {
    let style = display_layout_tile_style(tile, kit);
    let mut cell = div()
        .id(("settings-display-layout-tile", index))
        .absolute()
        .left(px(tile.left))
        .top(px(tile.top))
        .w(px(tile.width))
        .h(px(tile.height))
        .rounded(px(qol_theme::RADIUS_TIGHT))
        .border(px(qol_theme::LINE))
        .border_color(rgb(style.border_color))
        .bg(rgb(style.background))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_STACK))
                .px(px(qol_theme::SPACE_STACK))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text(TextStyle::Detail)
                        .text_color(rgb(kit.grounds.pane.soft))
                        .child(SharedString::from(tile.connector.clone())),
                )
                .children(
                    tile.selected
                        .then(|| status_chip("selected", kit.palette.accent_ink)),
                ),
        )
        .child(
            div()
                .px(px(qol_theme::SPACE_STACK))
                .text(TextStyle::Label)
                .text_color(rgb(kit.grounds.pane.faint))
                .child(SharedString::from(tile.resolution.clone())),
        );
    if tile.primary {
        cell = cell.child(
            div()
                .px(px(qol_theme::SPACE_STACK))
                .child(status_chip("primary", kit.palette.success)),
        );
    }
    cell
}

fn status_chip(text: &'static str, tone: u32) -> gpui::Div {
    let kit = kit();
    kit.chip(
        Chip::Status {
            tone,
            halo: qol_theme::translucent(tone, qol_theme::Alpha::Halo),
            text: text.into(),
        },
        kit.grounds.pane,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> Kit {
        crate::kit::kit()
    }

    fn tile(selected: bool, conflicted: bool) -> DisplayLayoutTile {
        DisplayLayoutTile {
            connector: "card0-DP-1".to_string(),
            resolution: "3840x2160 @ 60 Hz".to_string(),
            left: 0.0,
            top: 0.0,
            width: 320.0,
            height: 180.0,
            selected,
            primary: false,
            conflicted,
        }
    }

    #[test]
    fn selected_tile_takes_the_accent_border() {
        let kit = kit();
        let style = display_layout_tile_style(&tile(true, false), kit);
        assert_eq!(style.border_color, kit.grounds.pane.mark);
        assert_ne!(style.border_color, kit.palette.border_subtle);
    }

    #[test]
    fn unselected_tile_keeps_the_panel_border() {
        let kit = kit();
        let style = display_layout_tile_style(&tile(false, false), kit);
        assert_eq!(style.border_color, kit.palette.border_subtle);
    }

    #[test]
    fn conflicted_selected_tile_stays_on_danger() {
        let kit = kit();
        let style = display_layout_tile_style(&tile(true, true), kit);
        assert_eq!(style.border_color, kit.palette.danger);
        assert_ne!(style.border_color, kit.grounds.pane.mark);
    }

    #[test]
    fn conflicted_unselected_tile_stays_on_danger() {
        let kit = kit();
        let style = display_layout_tile_style(&tile(false, true), kit);
        assert_eq!(style.border_color, kit.palette.danger);
        assert_eq!(style.background, kit.grounds.menu.bg);
    }

    #[test]
    fn background_follows_selection() {
        let kit = kit();
        let selected = display_layout_tile_style(&tile(true, false), kit);
        let unselected = display_layout_tile_style(&tile(false, false), kit);
        assert_eq!(selected.background, kit.grounds.band.bg);
        assert_eq!(unselected.background, kit.grounds.menu.bg);
    }

    #[test]
    fn a_plain_tile_is_unchanged_from_todays_colors() {
        let kit = kit();
        let style = display_layout_tile_style(&tile(false, false), kit);
        assert_eq!(style.border_color, kit.palette.border_subtle);
        assert_eq!(style.background, kit.grounds.menu.bg);
    }
}
