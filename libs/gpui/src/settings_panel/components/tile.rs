use crate::key::Key;
use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{
    div, img, px, rgb, rgba, AnyElement, App, ClickEvent, CursorStyle, Div, ElementId, RenderOnce,
    SharedString, Window,
};
use qol_theme::TextStyle;

use qol_config::contract::is_picture_spec;

use crate::kit::kit;
use crate::pictures::{self, PictureContext};
use crate::theme::SettingsPanelPalette;

use super::hint_bar::SettingsHint;
use super::settings_tile_spinner;

pub const TILE_HEIGHT: f32 = 116.0;

const TICK_WIDTH: f32 = 16.0;
const TICK_HEIGHT: f32 = 12.0;

#[derive(Clone, Debug, PartialEq)]
pub enum TileArt {
    Picture(String),
    Waiting,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileLayout {
    pub per_row: usize,
    pub art_width: f32,
    pub art_height: f32,
    pub gap: f32,
    pub name_style: TextStyle,
}

pub fn tile_layout(tile_count: usize) -> TileLayout {
    if tile_count <= 6 {
        TileLayout {
            per_row: 3,
            art_width: 112.0,
            art_height: 70.0,
            gap: qol_theme::SPACE_INSET,
            name_style: TextStyle::ListName,
        }
    } else if tile_count <= 8 {
        TileLayout {
            per_row: 4,
            art_width: 104.0,
            art_height: 65.0,
            gap: qol_theme::SPACE_INSET,
            name_style: TextStyle::ListName,
        }
    } else {
        TileLayout {
            per_row: 5,
            art_width: 88.0,
            art_height: 55.0,
            gap: qol_theme::SPACE_SNUG,
            name_style: TextStyle::Detail,
        }
    }
}

pub fn tile_grid_gap(per_row: usize) -> f32 {
    match per_row {
        3 | 4 => qol_theme::SPACE_CELL,
        _ => qol_theme::SPACE_INSET,
    }
}

pub fn tile_arts(names: &[&str], pictures: &[Option<&str>]) -> Vec<TileArt> {
    let letters = pictures::letters_for(names);
    (0..names.len())
        .map(|index| {
            let picture = pictures
                .get(index)
                .copied()
                .flatten()
                .filter(|picture| is_picture_spec(picture));
            if let Some(picture) = picture {
                return TileArt::Picture(picture.to_string());
            }
            let spec = format!(
                "letters:{}",
                letters.get(index).map(String::as_str).unwrap_or_default()
            );
            if is_picture_spec(&spec) {
                TileArt::Picture(spec)
            } else {
                TileArt::Picture("letters:?".to_string())
            }
        })
        .collect()
}

pub fn choose_step(highlighted: usize, count: usize, per_row: usize, key: &str) -> Option<usize> {
    if count == 0 || per_row == 0 {
        return None;
    }
    match key {
        "left" => highlighted.checked_sub(1),
        "right" => (highlighted + 1 < count).then_some(highlighted + 1),
        "up" => highlighted.checked_sub(per_row),
        "down" => {
            if highlighted + per_row < count {
                Some(highlighted + per_row)
            } else {
                let last = count - 1;
                (last / per_row > highlighted / per_row).then_some(last)
            }
        }
        _ => None,
    }
}

pub fn choose_hints(count: usize) -> Vec<SettingsHint> {
    let movement = if count > tile_layout(count).per_row {
        SettingsHint::new(Key::ARROWS, "move")
    } else {
        SettingsHint::new(Key::LEFT_RIGHT, "move")
    };
    vec![SettingsHint::new(Key::ENTER, "choose"), movement]
}

pub fn settings_tile_rows(per_row: usize, tiles: Vec<AnyElement>) -> Vec<Div> {
    let mut rows = Vec::new();
    let mut tiles = tiles.into_iter();
    loop {
        let mut row = div()
            .flex()
            .flex_row()
            .w_full()
            .gap(px(tile_grid_gap(per_row)));
        let mut filled = 0;
        for tile in tiles.by_ref().take(per_row) {
            row = row.child(div().flex_1().min_w_0().child(tile));
            filled += 1;
        }
        if filled == 0 {
            break;
        }
        for _ in filled..per_row {
            row = row.child(div().flex_1().min_w_0());
        }
        let row = if rows.is_empty() {
            row.pt(px(qol_theme::SPACE_INSET))
        } else {
            row
        };
        rows.push(row);
    }
    rows
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SettingsTile {
    id: ElementId,
    name: SharedString,
    detail: Option<SharedString>,
    art: TileArt,
    layout: TileLayout,
    context: PictureContext,
    palette: SettingsPanelPalette,
    highlighted: bool,
    ticked: bool,
    on_click: Option<ClickHandler>,
}

impl SettingsTile {
    pub fn new(
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        art: TileArt,
        layout: TileLayout,
        context: PictureContext,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            detail: None,
            art,
            layout,
            context,
            palette,
            highlighted: false,
            ticked: false,
            on_click: None,
        }
    }

    pub fn detail(mut self, detail: Option<SharedString>) -> Self {
        self.detail = detail;
        self
    }

    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    pub fn ticked(mut self, ticked: bool) -> Self {
        self.ticked = ticked;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SettingsTile {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared = kit();
        let SettingsTile {
            id,
            name,
            detail,
            art,
            layout,
            context,
            palette,
            highlighted,
            ticked,
            on_click,
        } = self;
        let ground = palette.grounds.pane;
        let band = palette.grounds.band;
        let waiting = matches!(art, TileArt::Waiting);
        let scale = window.scale_factor();

        let mut tile = div()
            .id(id.clone())
            .relative()
            .w_full()
            .h(px(TILE_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(layout.gap))
            .px(px(qol_theme::SPACE_SNUG))
            .rounded(px(qol_theme::RADIUS_CARD));
        let border_color = if highlighted {
            0
        } else if waiting {
            shared.washes.hairline.packed()
        } else {
            shared.washes.hairline_strong.packed()
        };
        tile = tile
            .border(px(1.0))
            .border_color(rgba(border_color))
            .when(highlighted, |tile| tile.bg(rgb(band.bg)));

        let ink = if highlighted { band.ink } else { ground.soft };
        let art_box = div()
            .flex_none()
            .w(px(layout.art_width))
            .h(px(layout.art_height))
            .flex()
            .items_center()
            .justify_center();
        let art_box = match art {
            TileArt::Picture(spec) => {
                let width = (layout.art_width * scale).round() as u32;
                let height = (layout.art_height * scale).round() as u32;
                match pictures::image(&spec, ink, width, height, &context) {
                    Some(image) => {
                        art_box.child(img(image).w(px(layout.art_width)).h(px(layout.art_height)))
                    }
                    None => art_box,
                }
            }
            TileArt::Waiting => art_box.child(settings_tile_spinner(id, palette)),
        };

        let name_color = if waiting {
            ground.faint
        } else if highlighted {
            band.ink
        } else {
            ground.soft
        };
        let name = div()
            .w_full()
            .text_center()
            .text(layout.name_style)
            .line_clamp(2)
            .text_color(rgb(name_color))
            .child(name);

        let detail = detail.map(|detail| {
            let color = if highlighted { band.soft } else { ground.faint };
            div()
                .w_full()
                .text_center()
                .text(TextStyle::Detail)
                .text_color(rgb(color))
                .child(detail)
        });

        let words = div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .min_w_0()
            .child(name)
            .children(detail);

        let tick = if ticked && !waiting {
            let color = if highlighted { band.ink } else { ground.mark };
            let width = (TICK_WIDTH * scale).round() as u32;
            let height = (TICK_HEIGHT * scale).round() as u32;
            pictures::tick(color, width, height).map(|image| {
                img(image)
                    .absolute()
                    .top(px(qol_theme::SPACE_CELL))
                    .right(px(qol_theme::SPACE_INSET))
                    .w(px(TICK_WIDTH))
                    .h(px(TICK_HEIGHT))
            })
        } else {
            None
        };

        let mut tile = tile.child(art_box).child(words).children(tick);
        if let Some(on_click) = on_click {
            tile = tile
                .cursor(CursorStyle::PointingHand)
                .on_click(move |event, window, cx| on_click(event, window, cx));
        }
        tile
    }
}

#[cfg(test)]
mod tests {
    use super::{
        choose_hints, choose_step, tile_arts, tile_grid_gap, tile_layout, TileArt, TileLayout,
    };
    use crate::key::Key;
    use qol_theme::TextStyle;

    #[test]
    fn tile_layout_steps_with_the_option_count() {
        let cases = [
            (
                1usize,
                TileLayout {
                    per_row: 3,
                    art_width: 112.0,
                    art_height: 70.0,
                    gap: qol_theme::SPACE_INSET,
                    name_style: TextStyle::ListName,
                },
            ),
            (
                6,
                TileLayout {
                    per_row: 3,
                    art_width: 112.0,
                    art_height: 70.0,
                    gap: qol_theme::SPACE_INSET,
                    name_style: TextStyle::ListName,
                },
            ),
            (
                7,
                TileLayout {
                    per_row: 4,
                    art_width: 104.0,
                    art_height: 65.0,
                    gap: qol_theme::SPACE_INSET,
                    name_style: TextStyle::ListName,
                },
            ),
            (
                8,
                TileLayout {
                    per_row: 4,
                    art_width: 104.0,
                    art_height: 65.0,
                    gap: qol_theme::SPACE_INSET,
                    name_style: TextStyle::ListName,
                },
            ),
            (
                9,
                TileLayout {
                    per_row: 5,
                    art_width: 88.0,
                    art_height: 55.0,
                    gap: qol_theme::SPACE_SNUG,
                    name_style: TextStyle::Detail,
                },
            ),
            (
                15,
                TileLayout {
                    per_row: 5,
                    art_width: 88.0,
                    art_height: 55.0,
                    gap: qol_theme::SPACE_SNUG,
                    name_style: TextStyle::Detail,
                },
            ),
        ];
        for (count, expected) in cases {
            assert_eq!(tile_layout(count), expected, "count {count}");
        }
    }

    #[test]
    fn tile_grid_gap_tightens_at_five_per_row() {
        assert_eq!(tile_grid_gap(3), qol_theme::SPACE_CELL);
        assert_eq!(tile_grid_gap(4), qol_theme::SPACE_CELL);
        assert_eq!(tile_grid_gap(5), qol_theme::SPACE_INSET);
    }

    #[test]
    fn tile_arts_keep_valid_pictures_and_letter_the_rest() {
        let names = ["Bone", "Slate", "Nope", ""];
        let pictures = [
            Some("desktop-theme:bone"),
            None,
            Some("not-a-picture"),
            None,
        ];
        let arts = tile_arts(&names, &pictures);
        assert_eq!(arts[0], TileArt::Picture("desktop-theme:bone".to_string()));
        assert_eq!(arts[1], TileArt::Picture("letters:S".to_string()));
        assert_eq!(arts[2], TileArt::Picture("letters:N".to_string()));
        assert_eq!(arts[3], TileArt::Picture("letters:?".to_string()));
        let short = tile_arts(&["Bone"], &[]);
        assert_eq!(short[0], TileArt::Picture("letters:B".to_string()));
    }

    #[test]
    fn choose_step_stops_at_every_edge() {
        let cases = [
            (0usize, "left", None),
            (0, "right", Some(1)),
            (0, "up", None),
            (0, "down", Some(3)),
            (1, "left", Some(0)),
            (1, "right", Some(2)),
            (1, "up", None),
            (1, "down", Some(4)),
            (2, "left", Some(1)),
            (2, "right", Some(3)),
            (2, "up", None),
            (2, "down", Some(5)),
            (3, "left", Some(2)),
            (3, "right", Some(4)),
            (3, "up", Some(0)),
            (3, "down", Some(6)),
            (4, "left", Some(3)),
            (4, "right", Some(5)),
            (4, "up", Some(1)),
            (4, "down", Some(6)),
            (5, "left", Some(4)),
            (5, "right", Some(6)),
            (5, "up", Some(2)),
            (5, "down", Some(6)),
            (6, "left", Some(5)),
            (6, "right", None),
            (6, "up", Some(3)),
            (6, "down", None),
        ];
        for (highlighted, key, expected) in cases {
            assert_eq!(
                choose_step(highlighted, 7, 3, key),
                expected,
                "tile {highlighted} on {key}"
            );
        }
        assert_eq!(choose_step(0, 7, 3, "escape"), None);
    }

    #[test]
    fn choose_hints_follow_the_tile_grid() {
        let three = choose_hints(3);
        assert_eq!(three.len(), 2);
        assert_eq!(three[0].key, Some(Key::ENTER));
        assert_eq!(three[0].label, "choose");
        assert_eq!(three[1].key, Some(Key::LEFT_RIGHT));
        assert_eq!(three[1].label, "move");

        let ten = choose_hints(10);
        assert_eq!(ten.len(), 2);
        assert_eq!(ten[0].key, Some(Key::ENTER));
        assert_eq!(ten[0].label, "choose");
        assert_eq!(ten[1].key, Some(Key::ARROWS));
        assert_eq!(ten[1].label, "move");
    }
}
