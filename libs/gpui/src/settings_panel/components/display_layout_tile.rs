use gpui::prelude::*;
use gpui::{div, px, rgb, SharedString};

use crate::kit::kit;
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
    pub border_width: f32,
    pub background: u32,
}

pub fn display_layout_tile_style(
    tile: &DisplayLayoutTile,
    palette: SettingsPanelPalette,
) -> DisplayLayoutTileStyle {
    let (border_color, border_width) = if tile.conflicted {
        (palette.state_off, 2.0)
    } else if tile.selected {
        (palette.row_border_selected, 2.0)
    } else {
        (palette.panel_border, 1.0)
    };
    DisplayLayoutTileStyle {
        border_color,
        border_width,
        background: if tile.selected {
            palette.row_bg_selected
        } else {
            palette.dropdown_bg
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
        .bg(rgb(palette.dropdown_bg))
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
        .border(px(style.border_width))
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
                        .truncate()
                        .text_size(px(qol_theme::TEXT_MICRO))
                        .text_color(rgb(palette.label_text))
                        .child(SharedString::from(tile.connector.clone())),
                )
                .children(
                    tile.selected
                        .then(|| kit().status_pill("selected", palette.status_accent)),
                ),
        )
        .child(
            div()
                .truncate()
                .px(px(qol_theme::SPACE_STACK))
                .text_size(px(qol_theme::TEXT_NANO))
                .text_color(rgb(palette.status_muted))
                .child(SharedString::from(tile.resolution.clone())),
        );
    if tile.primary {
        cell = cell.child(
            div()
                .px(px(qol_theme::SPACE_STACK))
                .child(kit().status_pill("primary", palette.status_success)),
        );
    }
    cell
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
    fn selected_tile_takes_the_accent_border_at_two_pixels() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(true, false), palette);
        assert_eq!(style.border_color, palette.row_border_selected);
        assert_eq!(style.border_width, 2.0);
        assert_ne!(style.border_color, palette.panel_border);
    }

    #[test]
    fn unselected_tile_keeps_the_panel_border_at_one_pixel() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(style.border_color, palette.panel_border);
        assert_eq!(style.border_width, 1.0);
    }

    #[test]
    fn conflicted_selected_tile_stays_on_danger_at_two_pixels() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(true, true), palette);
        assert_eq!(style.border_color, palette.state_off);
        assert_eq!(style.border_width, 2.0);
        assert_ne!(style.border_color, palette.row_border_selected);
    }

    #[test]
    fn conflicted_unselected_tile_stays_on_danger_at_two_pixels() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, true), palette);
        assert_eq!(style.border_color, palette.state_off);
        assert_eq!(style.border_width, 2.0);
        assert_eq!(style.background, palette.dropdown_bg);
    }

    #[test]
    fn background_follows_selection() {
        let palette = palette();
        let selected = display_layout_tile_style(&tile(true, false), palette);
        let unselected = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(selected.background, palette.row_bg_selected);
        assert_eq!(unselected.background, palette.dropdown_bg);
    }

    #[test]
    fn a_plain_tile_is_unchanged_from_todays_colors() {
        let palette = palette();
        let style = display_layout_tile_style(&tile(false, false), palette);
        assert_eq!(style.border_color, palette.panel_border);
        assert_eq!(style.border_width, 1.0);
        assert_eq!(style.background, palette.dropdown_bg);
    }
}
