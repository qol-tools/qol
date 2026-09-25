use crate::key::Key;
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, KeyDownEvent};

use super::super::components::{
    choose_hints, choose_step, settings_tile_rows, tile_arts, tile_layout, ChoiceArt,
    SettingsGroupHeader, SettingsHint, SettingsTile, TileArt,
};
use super::super::rows::{
    row_query_names, Row, RowControl, RowQueryState, RowSection, SelectOption,
};
use super::super::SettingsDestination;
use super::{Level, LevelHeader, SettingsPanelView};
use crate::pictures::PictureContext;

const LABEL_DETAIL_SEPARATOR: &str = " \u{00b7} ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChooseOrigin {
    Row(usize),
    DisplayModes,
}

pub(super) struct ChooseState {
    pub(super) origin: ChooseOrigin,
    pub(super) highlighted: Option<String>,
}

pub(super) struct ChooseTile {
    pub(super) value: Option<String>,
    pub(super) name: String,
    pub(super) detail: Option<String>,
    pub(super) art: TileArt,
    pub(super) saved: bool,
}

pub(super) fn label_parts(label: &str) -> (&str, Option<&str>) {
    match label.split_once(LABEL_DETAIL_SEPARATOR) {
        Some((name, detail)) => (name, Some(detail)),
        None => (label, None),
    }
}

pub(super) fn option_arts(options: &[SelectOption]) -> Vec<TileArt> {
    let names = options
        .iter()
        .map(|option| label_parts(&option.label).0)
        .collect::<Vec<_>>();
    let pictures = options
        .iter()
        .map(|option| option.picture.as_deref())
        .collect::<Vec<_>>();
    tile_arts(&names, &pictures)
}

pub(super) fn option_art(options: &[SelectOption], index: usize) -> String {
    match option_arts(options).get(index) {
        Some(TileArt::Picture(spec)) => spec.clone(),
        _ => "letters:?".to_string(),
    }
}

pub(super) fn choose_tiles(
    options: &[SelectOption],
    saved: &[bool],
    waiting: Option<&str>,
) -> Vec<ChooseTile> {
    let arts = option_arts(options);
    let mut tiles = Vec::new();
    for (position, option) in options.iter().enumerate() {
        let (name, detail) = label_parts(&option.label);
        tiles.push(ChooseTile {
            value: Some(option.value.clone()),
            name: name.to_string(),
            detail: detail.map(str::to_string),
            art: arts[position].clone(),
            saved: saved.get(position) == Some(&true),
        });
    }
    if let Some(label) = waiting {
        tiles.push(ChooseTile {
            value: None,
            name: format!("Looking for {label}"),
            detail: None,
            art: TileArt::Waiting,
            saved: false,
        });
    }
    tiles
}

pub(super) fn highlighted_at_open(control: &RowControl) -> Option<String> {
    match control {
        RowControl::Select { options, index, .. } => {
            options.get(*index).map(|option| option.value.clone())
        }
        RowControl::MultiSelect {
            options, selected, ..
        } => options
            .iter()
            .zip(selected)
            .find(|(_, selected)| **selected)
            .map(|(option, _)| option.value.clone()),
        _ => None,
    }
}

fn choose_highlight(tiles: &[ChooseTile], highlighted: Option<&str>) -> usize {
    if let Some(value) = highlighted {
        if let Some(index) = tiles
            .iter()
            .position(|tile| tile.value.as_deref() == Some(value))
        {
            return index;
        }
    }
    tiles.iter().position(|tile| tile.saved).unwrap_or(0)
}

pub(super) fn multi_select_word(options: &[SelectOption], selected: &[bool]) -> String {
    let chosen = options
        .iter()
        .zip(selected)
        .filter(|(_, on)| **on)
        .map(|(option, _)| label_parts(&option.label).0)
        .collect::<Vec<_>>();
    if chosen.is_empty() {
        "none".to_string()
    } else {
        chosen.join(", ")
    }
}

pub(super) fn multi_select_art(options: &[SelectOption], selected: &[bool]) -> ChoiceArt {
    let arts = option_arts(options);
    let chosen = arts
        .iter()
        .zip(selected)
        .filter(|(_, on)| **on)
        .filter_map(|(art, _)| match art {
            TileArt::Picture(spec) => Some(spec.as_str()),
            TileArt::Waiting => None,
        })
        .collect::<Vec<_>>();
    match chosen.as_slice() {
        [] => ChoiceArt::Picture("empty".to_string()),
        [one] => ChoiceArt::Picture((*one).to_string()),
        [front, back, ..] => ChoiceArt::Stack {
            front: (*front).to_string(),
            back: (*back).to_string(),
        },
    }
}

pub(super) fn choose_enter_label(multi: bool, highlighted_saved: bool) -> &'static str {
    match (multi, highlighted_saved) {
        (false, _) => "choose",
        (true, true) => "untick",
        (true, false) => "tick",
    }
}

impl SettingsPanelView {
    fn choose_waiting_label(&self, row: &Row) -> Option<String> {
        let lookup_label = self
            .sources
            .get(row.source)?
            .copy
            .get(&row.id)?
            .lookup_label
            .clone()?;
        let pending = row_query_names(row).into_iter().any(|name| {
            matches!(
                self.query_states.get(&(row.source, name.to_string())),
                None | Some(RowQueryState::Idle) | Some(RowQueryState::Loading { .. })
            )
        });
        pending.then_some(lookup_label)
    }

    fn choose_card_tiles(&self) -> Option<(Vec<ChooseTile>, usize)> {
        let top = self.stack.len().checked_sub(1)?;
        let parent = self.stack.len().checked_sub(2)?;
        let choose = self.stack[top].choose.as_ref()?;
        let tiles = match choose.origin {
            ChooseOrigin::Row(origin_row) => {
                let row = self.stack[parent].rows.get(origin_row)?;
                let waiting = self.choose_waiting_label(row);
                match &row.control {
                    RowControl::Select { options, index, .. } => {
                        let mut saved = vec![false; options.len()];
                        if let Some(flag) = saved.get_mut(*index) {
                            *flag = true;
                        }
                        choose_tiles(options, &saved, waiting.as_deref())
                    }
                    RowControl::MultiSelect {
                        options, selected, ..
                    } => choose_tiles(options, selected, waiting.as_deref()),
                    _ => return None,
                }
            }
            ChooseOrigin::DisplayModes => {
                let state = self.stack[parent].display_layout.as_ref()?;
                super::display_layout_card::display_mode_tiles(state)
            }
        };
        let highlighted = choose_highlight(&tiles, choose.highlighted.as_deref());
        Some((tiles, highlighted))
    }

    fn choose_is_multi(&self) -> bool {
        let Some(top) = self.stack.len().checked_sub(1) else {
            return false;
        };
        let Some(parent) = self.stack.len().checked_sub(2) else {
            return false;
        };
        let Some(ChooseOrigin::Row(origin_row)) =
            self.stack[top].choose.as_ref().map(|choose| choose.origin)
        else {
            return false;
        };
        matches!(
            self.stack[parent]
                .rows
                .get(origin_row)
                .map(|row| &row.control),
            Some(RowControl::MultiSelect { .. })
        )
    }

    fn choose_tile(&mut self, option: usize, cx: &mut Context<Self>) {
        let Some(parent) = self.stack.len().checked_sub(2) else {
            return;
        };
        let Some(origin) = self
            .stack
            .last()
            .and_then(|level| level.choose.as_ref())
            .map(|choose| choose.origin)
        else {
            return;
        };
        match origin {
            ChooseOrigin::DisplayModes => self.choose_display_mode(option, cx),
            ChooseOrigin::Row(origin_row) => {
                let Some(row) = self.stack[parent].rows.get_mut(origin_row) else {
                    return;
                };
                match &mut row.control {
                    RowControl::Select {
                        options,
                        index,
                        live,
                        ..
                    } => {
                        if option >= options.len() {
                            return;
                        }
                        *index = option;
                        if let Some(live) = live {
                            live.saved = options[option].value.clone();
                        }
                        self.persist();
                        self.pop_card(cx);
                    }
                    RowControl::MultiSelect {
                        options, selected, ..
                    } => {
                        let Some(value) = options.get(option).map(|entry| entry.value.clone())
                        else {
                            return;
                        };
                        let Some(flag) = selected.get_mut(option) else {
                            return;
                        };
                        *flag = !*flag;
                        if let Some(choose) = self
                            .stack
                            .last_mut()
                            .and_then(|level| level.choose.as_mut())
                        {
                            choose.highlighted = Some(value);
                        }
                        self.persist();
                        cx.notify();
                    }
                    _ => {}
                }
            }
        }
    }

    pub(super) fn open_choose_card(&mut self, index: usize) {
        let Some(row) = self.level().rows.get(index) else {
            return;
        };
        if !matches!(
            row.control,
            RowControl::Select { .. } | RowControl::MultiSelect { .. }
        ) {
            return;
        }
        let label = row.label.clone();
        let source = row.source;
        let description = self
            .sources
            .get(source)
            .and_then(|state| state.copy.get(&row.id))
            .and_then(|copy| copy.card_description.clone());
        let destination = match SettingsDestination::new(label.as_str()) {
            Ok(destination) => destination,
            Err(error) => {
                self.save_error = Some(format!("{error}"));
                return;
            }
        };
        let section = RowSection {
            label: label.clone(),
            description,
            rows: Vec::new(),
            source,
        };
        let child = Level {
            rows: Vec::new(),
            sections: vec![section],
            selected: 0,
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds: Vec::new(),
            header: LevelHeader::Card(destination.clone()),
            origin_row: Some(index),
            object_array: None,
            display_layout: None,
            list_card: false,
            live_card: false,
            choose: Some(ChooseState {
                origin: ChooseOrigin::Row(index),
                highlighted: highlighted_at_open(&row.control),
            }),
            entries: None,
            form: None,
            list_item: None,
        };
        self.push_card(destination, child);
        self.sync_scroll();
    }

    pub(super) fn on_choose_card_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let Some((tiles, highlighted)) = self.choose_card_tiles() else {
            return false;
        };
        if tiles.is_empty() {
            return false;
        }
        let per_row = tile_layout(tiles.len()).per_row;
        match key {
            "left" | "right" | "up" | "down" => {
                if let Some(next) = choose_step(highlighted, tiles.len(), per_row, key) {
                    let value = tiles[next].value.clone();
                    let top = self.stack.len() - 1;
                    if let Some(state) = self.stack[top].choose.as_mut() {
                        state.highlighted = value;
                    }
                    self.sync_scroll();
                    cx.notify();
                }
                true
            }
            "enter" | "return" | "space" => {
                let tile = &tiles[highlighted];
                if !matches!(tile.art, TileArt::Waiting) && tile.value.is_some() {
                    self.choose_tile(highlighted, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(super) fn render_choose_card(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some((tiles, highlighted)) = self.choose_card_tiles() else {
            return Vec::new();
        };
        let Some(section) = self.level().sections.first() else {
            return Vec::new();
        };
        let label = section.label.clone();
        let description = section.description.clone();
        let layout = tile_layout(tiles.len());
        let context = PictureContext::for_accent(
            qol_theme::runtime_theme().mode,
            qol_theme::runtime_accent_key(),
        );
        let current = self.body_has_focus();
        let mut items: Vec<AnyElement> = Vec::new();
        items.push(
            SettingsGroupHeader::new(label, description.map(Into::into), self.palette)
                .current(current)
                .into_any_element(),
        );
        let mut tile_elements: Vec<AnyElement> = Vec::with_capacity(tiles.len());
        for (index, tile) in tiles.into_iter().enumerate() {
            let waiting = matches!(tile.art, TileArt::Waiting);
            let element = SettingsTile::new(
                ("settings-choose-tile", index),
                tile.name,
                tile.art,
                layout,
                context,
                self.palette,
            )
            .detail(tile.detail.map(Into::into))
            .highlighted(index == highlighted && current)
            .ticked(tile.saved);
            let element = if waiting {
                element
            } else {
                element.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.choose_tile(index, cx);
                }))
            };
            tile_elements.push(element.into_any_element());
        }
        for tile_row in settings_tile_rows(layout.per_row, tile_elements) {
            items.push(tile_row.into_any_element());
        }
        items
    }

    pub(super) fn choose_child_index(&self) -> Option<usize> {
        let (tiles, highlighted) = self.choose_card_tiles()?;
        let per_row = tile_layout(tiles.len()).per_row;
        Some(1 + highlighted / per_row)
    }

    pub(super) fn choose_hints(&self) -> Vec<SettingsHint> {
        let tiles = self.choose_card_tiles();
        let count = tiles.as_ref().map_or(0, |(tiles, _)| tiles.len());
        let multi = self.choose_is_multi();
        let saved = tiles
            .as_ref()
            .and_then(|(tiles, highlighted)| tiles.get(*highlighted))
            .is_some_and(|tile| tile.saved);
        let mut hints = choose_hints(count);
        if let Some(first) = hints.first_mut() {
            *first = SettingsHint::new(Key::ENTER, choose_enter_label(multi, saved));
        }
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::{
        choose_enter_label, choose_highlight, choose_tiles, highlighted_at_open, multi_select_art,
        multi_select_word, option_art,
    };
    use crate::settings_panel::components::{ChoiceArt, TileArt};
    use crate::settings_panel::rows::{LiveQuery, RowControl, SelectLive, SelectOption};

    fn managed_devices() -> Vec<SelectOption> {
        vec![
            SelectOption::plain("a", "WH-1000XM4 \u{00b7} AA:BB:CC:DD:EE:01"),
            SelectOption::plain("b", "Pixel Buds Pro \u{00b7} AA:BB:CC:DD:EE:02"),
            SelectOption::plain("c", "MX Master 3 \u{00b7} AA:BB:CC:DD:EE:03"),
        ]
    }

    #[test]
    fn choose_tiles_split_labels_and_letter_missing_pictures() {
        let mut themed = SelectOption::plain("bone", "Bone \u{00b7} Light desktop");
        themed.picture = Some("desktop-theme:bone".to_string());
        let unthemed = SelectOption::plain("slate", "Slate");
        let options = [themed, unthemed];
        let tiles = choose_tiles(&options, &[false, true], None);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].value.as_deref(), Some("bone"));
        assert_eq!(tiles[0].name, "Bone");
        assert_eq!(tiles[0].detail.as_deref(), Some("Light desktop"));
        assert_eq!(
            tiles[0].art,
            TileArt::Picture("desktop-theme:bone".to_string())
        );
        assert!(!tiles[0].saved);
        assert_eq!(tiles[1].name, "Slate");
        assert_eq!(tiles[1].detail, None);
        assert_eq!(tiles[1].art, TileArt::Picture("letters:S".to_string()));
        assert!(tiles[1].saved);
    }

    #[test]
    fn option_art_matches_the_card_tile() {
        let mut themed = SelectOption::plain("bone", "Bone \u{00b7} Light desktop");
        themed.picture = Some("desktop-theme:bone".to_string());
        let unthemed = SelectOption::plain("slate", "Slate");
        let options = [themed, unthemed];
        let tiles = choose_tiles(&options, &[true, false], None);
        for (index, tile) in tiles.iter().enumerate() {
            let TileArt::Picture(spec) = &tile.art else {
                panic!("choose tile art is not a picture");
            };
            assert_eq!(&option_art(&options, index), spec, "index: {index}");
        }
    }

    #[test]
    fn choose_tiles_append_the_waiting_tile() {
        let options = [
            SelectOption::plain("auto", "Automatic"),
            SelectOption::plain("hci0", "hci0 \u{00b7} adapter"),
        ];
        let tiles = choose_tiles(&options, &[true, false], Some("adapters"));
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[2].value, None);
        assert_eq!(tiles[2].name, "Looking for adapters");
        assert_eq!(tiles[2].art, TileArt::Waiting);
        assert!(!tiles[2].saved);
    }

    #[test]
    fn multi_select_word_names_the_chosen_options() {
        let options = managed_devices();
        assert_eq!(
            multi_select_word(&options, &[true, true, false]),
            "WH-1000XM4, Pixel Buds Pro"
        );
        assert_eq!(multi_select_word(&options, &[false, false, false]), "none");
    }

    #[test]
    fn multi_select_art_follows_how_many_are_chosen() {
        let options = managed_devices();
        assert_eq!(
            multi_select_art(&options, &[false, false, false]),
            ChoiceArt::Picture("empty".to_string())
        );
        assert_eq!(
            multi_select_art(&options, &[true, false, false]),
            ChoiceArt::Picture("letters:WH".to_string())
        );
        assert_eq!(
            multi_select_art(&options, &[true, true, false]),
            ChoiceArt::Stack {
                front: "letters:WH".to_string(),
                back: "letters:PB".to_string(),
            }
        );
        assert_eq!(
            multi_select_art(&options, &[true, true, true]),
            ChoiceArt::Stack {
                front: "letters:WH".to_string(),
                back: "letters:PB".to_string(),
            }
        );
        let mut pictured = managed_devices();
        pictured[0].picture = Some("mic-default".to_string());
        assert_eq!(
            multi_select_art(&pictured, &[true, false, false]),
            ChoiceArt::Picture("mic-default".to_string())
        );
    }

    #[test]
    fn choose_tiles_tick_every_saved_option() {
        let options = managed_devices();
        let tiles = choose_tiles(&options, &[true, false, true], None);
        assert!(tiles[0].saved);
        assert!(!tiles[1].saved);
        assert!(tiles[2].saved);
        let singles = choose_tiles(&options, &[false, true, false], None);
        assert_eq!(singles.iter().filter(|tile| tile.saved).count(), 1);
        assert!(singles[1].saved);
    }

    #[test]
    fn choose_highlight_opens_on_the_first_saved_tile() {
        let options = managed_devices();
        let tiles = choose_tiles(&options, &[false, true, true], None);
        assert_eq!(choose_highlight(&tiles, None), 1);
        assert_eq!(choose_highlight(&tiles, Some("c")), 2);
        let none = choose_tiles(&options, &[false, false, false], None);
        assert_eq!(choose_highlight(&none, None), 0);
    }

    #[test]
    fn a_polled_index_does_not_move_the_frozen_highlight() {
        let options = managed_devices();
        let mut control = RowControl::Select {
            options: options.clone(),
            index: 1,
            dynamic: None,
            live: Some(SelectLive {
                source: LiveQuery {
                    query: "output_status".to_string(),
                    value_from: Some("shown".to_string()),
                },
                saved: "b".to_string(),
            }),
        };
        let highlighted = highlighted_at_open(&control);
        assert_eq!(highlighted.as_deref(), Some("b"));
        let tiles = choose_tiles(&options, &[false, true, false], None);
        let chosen = choose_highlight(&tiles, highlighted.as_deref());
        assert_eq!(tiles[chosen].value.as_deref(), Some("b"));
        if let RowControl::Select { index, .. } = &mut control {
            *index = 2;
        }
        let moved = choose_tiles(&options, &[false, false, true], None);
        let still = choose_highlight(&moved, highlighted.as_deref());
        assert_eq!(still, chosen);
        assert_eq!(moved[still].value.as_deref(), Some("b"));
    }

    #[test]
    fn choose_enter_label_ticks_or_unticks_a_multi_select() {
        assert_eq!(choose_enter_label(false, false), "choose");
        assert_eq!(choose_enter_label(false, true), "choose");
        assert_eq!(choose_enter_label(true, false), "tick");
        assert_eq!(choose_enter_label(true, true), "untick");
    }
}
