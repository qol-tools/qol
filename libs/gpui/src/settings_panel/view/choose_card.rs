use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, KeyDownEvent};

use super::super::components::{
    choose_hints, choose_step, settings_tile_rows, tile_arts, tile_layout, SettingsGroupHeader,
    SettingsHint, SettingsTile, TileArt,
};
use super::super::rows::{
    row_query_names, Row, RowControl, RowQueryState, RowSection, SelectOption,
};
use super::super::SettingsDestination;
use super::{Level, LevelHeader, SettingsPanelView};
use crate::pictures::PictureContext;

const LABEL_DETAIL_SEPARATOR: &str = " \u{00b7} ";

pub(super) struct ChooseState {
    pub(super) origin_row: usize,
    pub(super) highlighted: Option<String>,
}

pub(super) struct ChooseTile {
    pub(super) value: Option<String>,
    pub(super) name: String,
    pub(super) detail: Option<String>,
    pub(super) art: TileArt,
    pub(super) saved: bool,
}

pub(super) fn option_arts(options: &[SelectOption]) -> Vec<TileArt> {
    let names = options
        .iter()
        .map(|option| {
            option
                .label
                .split_once(LABEL_DETAIL_SEPARATOR)
                .map_or(option.label.as_str(), |(name, _)| name)
        })
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
    saved: usize,
    waiting: Option<&str>,
) -> Vec<ChooseTile> {
    let arts = option_arts(options);
    let mut tiles = Vec::new();
    for (position, option) in options.iter().enumerate() {
        let (name, detail) = match option.label.split_once(LABEL_DETAIL_SEPARATOR) {
            Some((name, detail)) => (name, Some(detail)),
            None => (option.label.as_str(), None),
        };
        tiles.push(ChooseTile {
            value: Some(option.value.clone()),
            name: name.to_string(),
            detail: detail.map(str::to_string),
            art: arts[position].clone(),
            saved: position == saved,
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

fn choose_highlight(tiles: &[ChooseTile], highlighted: Option<&str>, saved: usize) -> usize {
    if let Some(value) = highlighted {
        if let Some(index) = tiles
            .iter()
            .position(|tile| tile.value.as_deref() == Some(value))
        {
            return index;
        }
    }
    saved.min(tiles.len().saturating_sub(1))
}

impl SettingsPanelView {
    fn choose_origin(&self) -> Option<(&Row, &ChooseState)> {
        let top = self.stack.len().checked_sub(1)?;
        let parent = self.stack.len().checked_sub(2)?;
        let choose = self.stack[top].choose.as_ref()?;
        let row = self.stack[parent].rows.get(choose.origin_row)?;
        Some((row, choose))
    }

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
        let (row, choose) = self.choose_origin()?;
        let RowControl::Select { options, index, .. } = &row.control else {
            return None;
        };
        let waiting = self.choose_waiting_label(row);
        let tiles = choose_tiles(options, *index, waiting.as_deref());
        let highlighted = choose_highlight(&tiles, choose.highlighted.as_deref(), *index);
        Some((tiles, highlighted))
    }

    fn choose_tile(&mut self, option: usize, cx: &mut Context<Self>) {
        let Some(parent) = self.stack.len().checked_sub(2) else {
            return;
        };
        let Some(origin) = self
            .stack
            .last()
            .and_then(|level| level.choose.as_ref())
            .map(|choose| choose.origin_row)
        else {
            return;
        };
        let Some(row) = self.stack[parent].rows.get_mut(origin) else {
            return;
        };
        let RowControl::Select { options, index, .. } = &mut row.control else {
            return;
        };
        if option >= options.len() {
            return;
        }
        *index = option;
        self.persist();
        self.pop_card(cx);
    }

    pub(super) fn open_choose_card(&mut self, index: usize) {
        let Some(row) = self.level().rows.get(index) else {
            return;
        };
        if !matches!(row.control, RowControl::Select { .. }) {
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
                origin_row: index,
                highlighted: None,
            }),
            entries: None,
            form: None,
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
        let Some((row, _)) = self.choose_origin() else {
            return Vec::new();
        };
        let label = row.label.clone();
        let description = self
            .sources
            .get(row.source)
            .and_then(|state| state.copy.get(&row.id))
            .and_then(|copy| copy.card_description.clone());
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
        let count = self
            .choose_card_tiles()
            .map(|(tiles, _)| tiles.len())
            .unwrap_or_default();
        choose_hints(count)
    }
}

#[cfg(test)]
mod tests {
    use super::{choose_tiles, option_art};
    use crate::settings_panel::components::TileArt;
    use crate::settings_panel::rows::SelectOption;

    #[test]
    fn choose_tiles_split_labels_and_letter_missing_pictures() {
        let mut themed = SelectOption::plain("bone", "Bone \u{00b7} Light desktop");
        themed.picture = Some("desktop-theme:bone".to_string());
        let unthemed = SelectOption::plain("slate", "Slate");
        let options = [themed, unthemed];
        let tiles = choose_tiles(&options, 1, None);
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
        let tiles = choose_tiles(&options, 0, None);
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
        let tiles = choose_tiles(&options, 0, Some("adapters"));
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[2].value, None);
        assert_eq!(tiles[2].name, "Looking for adapters");
        assert_eq!(tiles[2].art, TileArt::Waiting);
        assert!(!tiles[2].saved);
    }
}
