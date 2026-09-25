use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, SharedString};
use qol_theme::TextStyle;

use crate::kit::{kit, Chip};
use crate::theme::SettingsPanelPalette;

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

pub fn display_layout_tile_style(
    tile: &DisplayLayoutTile,
    palette: SettingsPanelPalette,
) -> DisplayLayoutTileStyle {
    let border_color = if tile.conflicted {
        palette.state_off
    } else if tile.selected {
        palette.row_border_selected
    } else {
        palette.panel_border
    };
    DisplayLayoutTileStyle {
        border_color,
        background: if tile.selected {
            palette.row_bg_selected
        } else {
            palette.surface_raised
        },
    }
}

pub fn display_layout_stage(palette: SettingsPanelPalette, height: f32) -> gpui::Div {
    div()
        .relative()
        .w_full()
        .h(px(height))
        .overflow_hidden()
        .rounded(px(qol_theme::RADIUS_CARD))
        .bg(rgb(palette.surface_raised))
}

pub fn display_layout_ghost(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    palette: SettingsPanelPalette,
) -> gpui::Div {
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .h(px(height))
        .rounded(px(qol_theme::RADIUS_TIGHT))
        .border(px(qol_theme::LINE))
        .border_color(rgb(palette.row_border_selected))
        .bg(rgb(palette.row_bg_selected))
        .opacity(qol_theme::OPACITY_REST)
}

pub fn display_layout_tile(
    index: usize,
    tile: &DisplayLayoutTile,
    palette: SettingsPanelPalette,
) -> gpui::Stateful<gpui::Div> {
    let style = display_layout_tile_style(tile, palette);
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
                        .text_color(rgb(palette.label_text))
                        .child(SharedString::from(tile.connector.clone())),
                )
                .children(
                    tile.selected
                        .then(|| status_chip("selected", palette.status_accent)),
                ),
        )
        .child(
            div()
                .px(px(qol_theme::SPACE_STACK))
                .text(TextStyle::Label)
                .text_color(rgb(palette.status_muted))
                .child(SharedString::from(tile.resolution.clone())),
        );
    if tile.primary {
        cell = cell.child(
            div()
                .px(px(qol_theme::SPACE_STACK))
                .child(status_chip("primary", palette.status_success)),
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

    fn palette() -> SettingsPanelPalette {
        qol_theme::settings_panel_runtime()
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
        let palette = palette();
        let style = display_layout_tile_style(&tile(true, false), palette);
        assert_eq!(style.border_color, palette.row_border_selected);
        assert_ne!(style.border_color, palette.panel_border);
    }

    #[test]
    fn unselected_tile_keeps_the_panel_border() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(style.border_color, palette.panel_border);
    }

    #[test]
    fn conflicted_selected_tile_stays_on_danger() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(true, true), palette);
        assert_eq!(style.border_color, palette.state_off);
        assert_ne!(style.border_color, palette.row_border_selected);
    }

    #[test]
    fn conflicted_unselected_tile_stays_on_danger() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, true), palette);
        assert_eq!(style.border_color, palette.state_off);
        assert_eq!(style.background, palette.surface_raised);
    }

    #[test]
    fn background_follows_selection() {
        let palette = palette();
        let selected = display_layout_tile_style(&tile(true, false), palette);
        let unselected = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(selected.background, palette.row_bg_selected);
        assert_eq!(unselected.background, palette.surface_raised);
    }

    #[test]
    fn a_plain_tile_is_unchanged_from_todays_colors() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(style.border_color, palette.panel_border);
        assert_eq!(style.background, palette.surface_raised);
    }
}
