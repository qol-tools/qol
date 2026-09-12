use std::cell::Cell;
use std::rc::Rc;

use gpui::*;
use qol_config::object_array::pretty_label;

use super::super::components::SettingsRow;
use super::super::object_array_row::{
    shared_key_chip, Chip, ChipTone, DraftField, DraftValue, ItemChips, ObjectArrayOutcome,
    ObjectArrayState,
};
use super::super::rows::{Row, RowControl, RowSection};
use super::super::SettingsDestination;
use super::{initial_card_selection, Level, LevelHeader, SettingsPanelView};

impl SettingsPanelView {
    pub(super) fn on_object_array_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.on_object_array_card_key(key, key_char, cx)
    }

    fn on_object_array_card_key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let selected = self.level().selected;
        let outcome = match self.level_mut().object_array.as_mut() {
            Some(state) if state.draft.is_some() => state.handle_key(key, key_char),
            Some(state) => {
                if matches!(key, "up" | "down") {
                    return false;
                }
                let count = state.entry_count();
                state.list.selected = selected.min(count - 1);
                state.list.sync(count);
                state.handle_key(key, key_char)
            }
            None => return false,
        };
        match outcome {
            ObjectArrayOutcome::Ignored => return false,
            ObjectArrayOutcome::Handled => {}
            ObjectArrayOutcome::Persist => {
                self.sync_object_array_to_root();
                self.persist();
            }
            ObjectArrayOutcome::Close => {
                self.pop_card(cx);
                return true;
            }
        }
        self.sync_scroll();
        cx.notify();
        true
    }

    pub(super) fn open_object_array_card(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::ObjectArray(state) = &row.control else {
            return;
        };
        let label = row.label.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let child = ObjectArrayState::from_entries(
            state.key_label.clone(),
            state.schema.clone(),
            state.entries.clone(),
        );
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let level = object_array_card_level(
            &label,
            &config_key,
            source,
            child,
            row_index,
            destination.clone(),
        );
        self.push_card(destination, level);
        self.sync_scroll();
    }

    pub(super) fn sync_object_array_to_root(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let (root, front) = self.stack.split_at_mut(1);
        object_array_card_sync(&mut root[0].rows, front.last_mut().expect("front level"));
        self.height_revision += 1;
    }

    pub(super) fn object_array_card_chips(&self, index: usize) -> Option<ItemChips> {
        let state = self.level().object_array.as_ref()?;
        if index == 0 || index > state.entries.len() {
            return None;
        }
        Some(state.chips(index - 1))
    }

    pub(super) fn render_chip_row(&self, parts: Vec<ChipRowPart>) -> Div {
        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_TIGHT))
            .min_w_0()
            .overflow_hidden();
        for part in parts {
            strip = strip.child(match part {
                ChipRowPart::Arrow => div()
                    .flex_none()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.status_muted))
                    .child("\u{2192}"),
                ChipRowPart::Chip(chip) => match chip.tone {
                    ChipTone::Modifier | ChipTone::Key => self.kit.keycap(chip.label),
                    ChipTone::Plain => self.kit.chip(chip.label, self.palette.label_text),
                },
            });
        }
        strip
    }

    pub(super) fn render_object_array_card_draft(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.stack.len() <= 1 {
            return None;
        }
        let state = self.level().object_array.as_ref()?;
        let draft = state.draft.as_ref()?;
        let mut container = div().flex().flex_col().gap(px(qol_theme::SPACE_TIGHT));
        let index = self.level().selected;
        for (field_index, field) in draft.fields.iter().enumerate() {
            container = container.child(self.render_draft_field(index, field_index, field, cx));
        }
        container = container.child(self.render_draft_save(index, draft.save_entry_selected(), cx));
        Some(container)
    }

    fn object_array_line(&self, index: usize, entry: usize, selected: bool) -> SettingsRow {
        SettingsRow::rule(
            ("settings-object-entry", entry_id(index, entry)),
            self.palette,
        )
        .selected(selected, self.body_has_focus())
    }

    fn object_array_state(&self, index: usize) -> Option<&ObjectArrayState> {
        if let Some(state) = self.level().object_array.as_ref() {
            return Some(state);
        }
        match &self.level().rows[index].control {
            RowControl::ObjectArray(state) => Some(state),
            _ => None,
        }
    }

    fn object_array_state_mut(&mut self, index: usize) -> Option<&mut ObjectArrayState> {
        let level = self.level_mut();
        if level.object_array.is_some() {
            return level.object_array.as_mut();
        }
        match &mut level.rows[index].control {
            RowControl::ObjectArray(state) => Some(state),
            _ => None,
        }
    }

    fn render_draft_field(
        &self,
        index: usize,
        field_index: usize,
        field: &DraftField,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.object_array_state(index) else {
            return div()
                .id(("settings-draft-empty", entry_id(index, field_index)))
                .into_any_element();
        };
        let selected = state
            .draft
            .as_ref()
            .is_some_and(|draft| draft.selected == field_index);
        self.object_array_line(index, field_index, selected)
            .child(
                div()
                    .flex_none()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child(pretty_label(&field.key)),
            )
            .child(self.render_draft_value(index, field_index, field, selected, cx))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.select_draft_field(index, field_index);
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_draft_value(
        &self,
        index: usize,
        field_index: usize,
        field: &DraftField,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut value = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(qol_theme::SPACE_TIGHT))
            .flex_1()
            .min_w_0()
            .overflow_hidden();
        if let DraftValue::Mods {
            options,
            selected: flags,
            cursor,
        } = &field.value
        {
            for (option_index, option) in options.iter().enumerate() {
                let on = flags.get(option_index).copied().unwrap_or(false);
                let focused = selected && option_index == *cursor;
                let slot = ModChipSlot {
                    row: index,
                    field: field_index,
                    option: option_index,
                };
                value = value.child(self.render_mod_chip(slot, option.clone(), on, focused, cx));
            }
            return value;
        }
        let text = match (&field.value, selected) {
            (DraftValue::Text(text), true) => format!("{text}_"),
            (DraftValue::Text(text), false) if text.is_empty() => {
                pretty_label(&field.key).to_lowercase()
            }
            _ => field.display(),
        };
        value.child(
            div()
                .truncate()
                .text_size(px(qol_theme::TEXT_BODY))
                .text_color(rgb(match &field.value {
                    DraftValue::Text(text) if text.is_empty() && !selected => {
                        self.palette.status_muted
                    }
                    DraftValue::Boolean(true) => self.palette.state_on,
                    _ => self.palette.label_text,
                }))
                .child(text),
        )
    }

    fn render_mod_chip(
        &self,
        slot: ModChipSlot,
        label: String,
        on: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("settings-mod-chip", slot.id()))
            .flex_none()
            .px(px(qol_theme::SPACE_SNUG))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .shadow(vec![BoxShadow {
                color: rgba(qol_color::with_alpha(
                    if focused {
                        self.palette.row_border_selected
                    } else {
                        self.palette.panel_border
                    },
                    0xff,
                ))
                .into(),
                offset: point(px(0.0), px(1.5)),
                blur_radius: px(0.0),
                spread_radius: px(0.0),
            }])
            .bg(if on {
                rgb(self.palette.dropdown_bg)
            } else {
                rgba(self.palette.transparent_rgba)
            })
            .text_size(px(qol_theme::TEXT_CAPTION))
            .text_color(rgb(if on {
                self.palette.state_on
            } else {
                self.palette.status_muted
            }))
            .cursor(CursorStyle::PointingHand)
            .child(label)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                cx.stop_propagation();
                this.toggle_draft_mod(slot.row, slot.field, slot.option);
                cx.notify();
            }))
    }

    fn render_draft_save(
        &self,
        index: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = usize::MAX;
        self.object_array_line(index, entry, selected)
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(self.palette.state_on))
                    .child("Save"),
            )
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child("enter"),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.commit_object_array_draft(index);
                cx.notify();
            }))
            .into_any_element()
    }

    fn toggle_draft_mod(&mut self, index: usize, field_index: usize, option_index: usize) {
        let Some(state) = self.object_array_state_mut(index) else {
            return;
        };
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        draft.toggle_mod_at(field_index, option_index);
    }

    fn select_draft_field(&mut self, index: usize, field_index: usize) {
        let Some(state) = self.object_array_state_mut(index) else {
            return;
        };
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        draft.selected = field_index;
    }

    fn commit_object_array_draft(&mut self, index: usize) {
        let Some(state) = self.object_array_state_mut(index) else {
            return;
        };
        if state.commit_draft() {
            self.sync_object_array_to_root();
            self.persist();
        }
        self.sync_scroll();
    }
}

#[derive(Clone, Copy)]
struct ModChipSlot {
    row: usize,
    field: usize,
    option: usize,
}

impl ModChipSlot {
    fn id(self) -> u64 {
        ((self.row as u64) << 32) | ((self.field as u16 as u64) << 16) | self.option as u16 as u64
    }
}

fn entry_id(index: usize, entry: usize) -> u64 {
    ((index as u64) << 32) | (entry as u32) as u64
}

fn object_array_child_rows(
    _label: &str,
    key: &str,
    source: usize,
    state: &ObjectArrayState,
) -> Vec<Row> {
    let mut rows = vec![Row {
        id: "object_array_add".into(),
        section_id: None,
        section_label: None,
        label: "+ Add".into(),
        description: None,
        placeholder: None,
        variant: None,
        config_key: key.to_string(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control: RowControl::Text(String::new()),
    }];
    rows.extend((0..state.entries.len()).map(|index| {
        let summary = state.summary(index);
        Row {
            id: format!("object_array_item_{index}"),
            section_id: None,
            section_label: None,
            label: summary.clone(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: key.to_string(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source,
            control: RowControl::Text(summary),
        }
    }));
    rows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChipRowPart {
    Chip(Chip),
    Arrow,
}

pub(super) fn chip_row_parts(chips: &ItemChips) -> Vec<ChipRowPart> {
    let mut parts = Vec::new();
    if chips.is_directional() {
        parts.extend(chips.from.iter().cloned().map(ChipRowPart::Chip));
        if let Some(shared) = shared_key_chip(&chips.rest) {
            parts.push(ChipRowPart::Chip(shared));
        }
        parts.push(ChipRowPart::Arrow);
        parts.extend(chips.to.iter().cloned().map(ChipRowPart::Chip));
    } else {
        parts.extend(
            chips
                .from
                .iter()
                .chain(&chips.rest)
                .chain(&chips.to)
                .cloned()
                .map(ChipRowPart::Chip),
        );
    }
    parts.extend(chips.flags.iter().map(|flag| {
        ChipRowPart::Chip(Chip {
            label: flag.clone(),
            tone: ChipTone::Plain,
        })
    }));
    parts
}

fn object_array_card_level(
    label: &str,
    config_key: &str,
    source: usize,
    state: ObjectArrayState,
    origin_row: usize,
    destination: SettingsDestination,
) -> Level {
    let selected = initial_card_selection(state.entries.len());
    let mut state = state;
    state.list.selected = selected;
    state.list.sync(state.entry_count());
    let rows = object_array_child_rows(label, config_key, source, &state);
    let section = RowSection {
        label: label.to_string(),
        description: None,
        rows: (0..rows.len()).collect(),
        source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    Level {
        rows,
        sections: vec![section],
        selected,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        header: LevelHeader::Card(destination),
        origin_row: Some(origin_row),
        object_array: Some(state),
        display_layout: None,
        list_card: false,
        live_card: false,
    }
}

fn sync_object_array_level(level: &mut Level, label: &str, config_key: &str, source: usize) {
    let Some(state) = level.object_array.as_ref() else {
        return;
    };
    let selected = state.list.selected;
    let rows = object_array_child_rows(label, config_key, source, state);
    let section = RowSection {
        label: label.to_string(),
        description: None,
        rows: (0..rows.len()).collect(),
        source,
    };
    level.rows = rows;
    level.sections = vec![section];
    level.row_bounds = (0..level.rows.len())
        .map(|_| Rc::new(Cell::new(None)))
        .collect();
    level.selected = selected.min(level.rows.len() - 1);
}

fn object_array_card_sync(root_rows: &mut [Row], level: &mut Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let Some(RowControl::ObjectArray(stored)) =
        root_rows.get_mut(origin_row).map(|row| &mut row.control)
    else {
        return;
    };
    let Some(child) = level.object_array.as_ref() else {
        return;
    };
    *stored = ObjectArrayState::from_entries(
        child.key_label.clone(),
        child.schema.clone(),
        child.entries.clone(),
    );
    let Some(row) = root_rows.get(origin_row) else {
        return;
    };
    let label = row.label.clone();
    let config_key = row.config_key.clone();
    let source = row.source;
    sync_object_array_level(level, &label, &config_key, source);
}

#[cfg(test)]
mod tests {
    use super::super::super::object_array_row::{
        Chip, ChipTone, Entry, Item, ItemChips, ObjectArrayState,
    };
    use super::super::super::rows::RowControl;
    use super::super::super::SettingsDestination;
    use super::super::tests::{level, rows};
    use super::ChipRowPart;

    fn object_array_state() -> ObjectArrayState {
        ObjectArrayState::from_entries(
            None,
            vec![(
                "app".to_string(),
                qol_config::object_array::ItemFieldKind::Text,
            )],
            vec![
                Entry {
                    key: None,
                    fields: Item::from_iter([(
                        "app".to_string(),
                        qol_config::contract::FieldDefault::String("idea".into()),
                    )]),
                },
                Entry {
                    key: None,
                    fields: Item::from_iter([(
                        "app".to_string(),
                        qol_config::contract::FieldDefault::String("zed".into()),
                    )]),
                },
            ],
        )
    }

    #[test]
    fn object_array_card_rows_build_the_add_row_first_then_one_row_per_entry() {
        let state = object_array_state();
        let rows = super::object_array_child_rows("Rules", "rules", 1, &state);
        assert_eq!(rows.len(), 3);
        let add = &rows[0];
        assert_eq!(add.label, "+ Add");
        assert!(matches!(&add.control, RowControl::Text(stored) if stored.is_empty()));
        for (index, row) in rows.iter().skip(1).enumerate() {
            assert_eq!(row.label, state.summary(index));
            assert!(matches!(
                &row.control,
                RowControl::Text(stored) if stored == &state.summary(index)
            ));
            assert_eq!(row.config_key, "rules");
            assert_eq!(row.source, 1);
        }
    }

    #[test]
    fn an_empty_object_array_card_selects_the_add_row() {
        let empty = ObjectArrayState::from_entries(
            None,
            vec![(
                "app".to_string(),
                qol_config::object_array::ItemFieldKind::Text,
            )],
            Vec::new(),
        );
        let level = super::object_array_card_level(
            "Rules",
            "rules",
            0,
            empty,
            3,
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(level.selected, 0);
        assert_eq!(level.object_array.as_ref().unwrap().list.selected, 0);

        let level = super::object_array_card_level(
            "Rules",
            "rules",
            0,
            object_array_state(),
            3,
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(level.selected, 1);
        assert_eq!(level.object_array.as_ref().unwrap().list.selected, 1);
    }

    #[test]
    fn removing_an_entry_rebuilds_the_card_rows_and_clamps_selection() {
        let state = object_array_state();
        let mut level = super::object_array_card_level(
            "Rules",
            "rules",
            0,
            state,
            3,
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(level.selected, 1);
        level.selected = 2;
        let child = level.object_array.as_mut().unwrap();
        child.list.selected = 2;
        assert!(child.remove_selected());
        super::sync_object_array_level(&mut level, "Rules", "rules", 0);
        let child = level.object_array.as_ref().unwrap();
        assert_eq!(level.rows.len(), 2);
        assert_eq!(level.rows[0].label, "+ Add");
        assert_eq!(level.rows[1].label, child.summary(0));
        assert_eq!(level.selected, 1);
    }

    #[test]
    fn a_synced_root_row_holds_the_child_entries_in_order() {
        let mut root = level(0);
        root.rows = rows(&[false, false]);
        root.rows[1].control = RowControl::ObjectArray(ObjectArrayState::from_entries(
            None,
            vec![(
                "app".to_string(),
                qol_config::object_array::ItemFieldKind::Text,
            )],
            Vec::new(),
        ));
        let mut child = super::object_array_card_level(
            "Rules",
            "rules",
            0,
            object_array_state(),
            1,
            SettingsDestination::from_static("Rules"),
        );
        super::object_array_card_sync(&mut root.rows, &mut child);
        let RowControl::ObjectArray(stored) = &root.rows[1].control else {
            panic!("root row holds an object array");
        };
        let entries = child.object_array.as_ref().unwrap().entries.clone();
        assert_eq!(stored.entries, entries);
    }

    #[test]
    fn a_directional_chip_row_renders_the_same_chips_item_chips_produces() {
        let state = ObjectArrayState::from_entries(
            None,
            vec![
                (
                    "from_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "to_mods".to_string(),
                    qol_config::object_array::ItemFieldKind::Mods,
                ),
                (
                    "keys".to_string(),
                    qol_config::object_array::ItemFieldKind::StringArray,
                ),
                (
                    "global".to_string(),
                    qol_config::object_array::ItemFieldKind::Boolean,
                ),
            ],
            vec![Entry {
                key: None,
                fields: Item::from_iter([
                    (
                        "from_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["ctrl".into()]),
                    ),
                    (
                        "to_mods".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec!["cmd".into()]),
                    ),
                    (
                        "keys".to_string(),
                        qol_config::contract::FieldDefault::StringArray(vec![
                            "c".into(),
                            "v".into(),
                        ]),
                    ),
                    (
                        "global".to_string(),
                        qol_config::contract::FieldDefault::Boolean(true),
                    ),
                ]),
            }],
        );
        let chips = state.chips(0);
        assert!(chips.is_directional());
        assert_eq!(
            super::chip_row_parts(&chips),
            vec![
                ChipRowPart::Chip(Chip {
                    label: "ctrl".into(),
                    tone: ChipTone::Modifier
                }),
                ChipRowPart::Chip(Chip {
                    label: "2 keys".into(),
                    tone: ChipTone::Key
                }),
                ChipRowPart::Arrow,
                ChipRowPart::Chip(Chip {
                    label: "cmd".into(),
                    tone: ChipTone::Modifier
                }),
                ChipRowPart::Chip(Chip {
                    label: "global".into(),
                    tone: ChipTone::Plain
                }),
            ]
        );
    }

    #[test]
    fn a_non_directional_chip_row_renders_in_order_without_an_arrow() {
        let chips = ItemChips {
            from: vec![Chip {
                label: "idea".into(),
                tone: ChipTone::Key,
            }],
            rest: Vec::new(),
            to: Vec::new(),
            flags: vec!["wired".into()],
        };
        assert_eq!(
            super::chip_row_parts(&chips),
            vec![
                ChipRowPart::Chip(Chip {
                    label: "idea".into(),
                    tone: ChipTone::Key
                }),
                ChipRowPart::Chip(Chip {
                    label: "wired".into(),
                    tone: ChipTone::Plain
                }),
            ]
        );
    }
}
