use crate::key::Key;
use crate::kit::Chip as KitChip;
use crate::text::TextStyled;
use qol_theme::TextStyle;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::*;
use qol_config::contract::NestedListSpec;
use qol_config::object_array::pretty_label;

use super::super::components::{
    settings_label_group, settings_mono_label, settings_value_group, settings_value_text, HintTone,
    RowGround, SettingsHint, SettingsHintBar, SettingsModifierChip, SettingsRow, SettingsTextField,
    SettingsToggle, SettingsValueTone,
};
use super::super::entry_form::{count_label, EntryForm, FormOutput, FormQuestion, FormValue};
use super::super::object_array_row::{
    shared_key_chip, Chip, ChipTone, ItemChips, ObjectArrayState,
};
use super::super::rows::{Row, RowControl, RowSection};
use super::super::SettingsDestination;
use super::{initial_card_selection, Level, LevelHeader, SettingsPanelView};
use crate::text_edit;

pub(super) struct EntriesCard {
    pub(super) item_label: String,
    pub(super) lists: BTreeMap<String, NestedListSpec>,
    pub(super) values: Option<Vec<String>>,
    pub(super) owner_field: Option<usize>,
}

struct EntriesOrigin {
    object_array: Option<ObjectArrayState>,
    card: EntriesCard,
    description: Option<String>,
    origin_row: Option<usize>,
}

#[derive(Clone, Copy)]
enum FormKey {
    Next,
    Flip,
    Step(isize),
    Toggle,
    Open,
    Edit,
    Consume,
}

#[derive(Clone, Copy)]
enum FormRowClick {
    Select,
    Flip,
    Open,
}

impl SettingsPanelView {
    pub(super) fn open_object_array_card(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::ObjectArray(state) = &row.control else {
            return;
        };
        let label = row.label.clone();
        let row_id = row.id.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let copy = self
            .sources
            .get(source)
            .and_then(|state| state.copy.get(&row_id));
        let description = copy.and_then(|copy| copy.card_description.clone());
        let item_label = copy
            .and_then(|copy| copy.item_label.clone())
            .unwrap_or_else(|| "entry".to_string());
        let lists = copy.map(|copy| copy.lists.clone()).unwrap_or_default();
        let state = ObjectArrayState::from_entries(
            state.key_label.clone(),
            state.schema.clone(),
            state.entries.clone(),
        );
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let level = entries_card_level(
            &label,
            &config_key,
            source,
            EntriesOrigin {
                object_array: Some(state),
                card: EntriesCard {
                    item_label,
                    lists,
                    values: None,
                    owner_field: None,
                },
                description,
                origin_row: Some(row_index),
            },
            destination.clone(),
        );
        self.push_card(destination, level);
        self.sync_scroll();
    }

    pub(super) fn open_text_list_card(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::TextList(values) = &row.control else {
            return;
        };
        let label = row.label.clone();
        let row_id = row.id.clone();
        let config_key = row.config_key.clone();
        let source = row.source;
        let values = values.clone();
        let copy = self
            .sources
            .get(source)
            .and_then(|state| state.copy.get(&row_id));
        let description = copy.and_then(|copy| copy.card_description.clone());
        let item_label = copy
            .and_then(|copy| copy.item_label.clone())
            .unwrap_or_else(|| "entry".to_string());
        let lists = copy.map(|copy| copy.lists.clone()).unwrap_or_default();
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let level = entries_card_level(
            &label,
            &config_key,
            source,
            EntriesOrigin {
                object_array: None,
                card: EntriesCard {
                    item_label,
                    lists,
                    values: Some(values),
                    owner_field: None,
                },
                description,
                origin_row: Some(row_index),
            },
            destination.clone(),
        );
        self.push_card(destination, level);
        self.sync_scroll();
    }

    pub(super) fn on_entries_card_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        if matches!(key, "enter" | "return" | "space") {
            let selected = self.level().selected;
            if selected == 0 {
                self.open_entries_form(None, cx);
            } else {
                self.open_entries_form(Some(selected - 1), cx);
            }
            return true;
        }
        if matches!(key, "backspace" | "delete") {
            self.remove_entry();
            return true;
        }
        if plain_a_key(event) {
            self.open_entries_form(None, cx);
            return true;
        }
        false
    }

    pub(super) fn on_form_card_key(
        &mut self,
        event: &KeyDownEvent,
        walking: Option<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        if let Some(question) = self.level().form.as_ref().and_then(|form| form.question) {
            match key {
                "enter" | "return" => {
                    match question {
                        FormQuestion::Save => {
                            self.back_target = walking;
                            self.save_form(cx);
                        }
                        FormQuestion::Blocked(index) => {
                            if let Some(form) = self.level_mut().form.as_mut() {
                                form.question = None;
                            }
                            self.select_form_row(index);
                        }
                    }
                    return true;
                }
                "escape" => {
                    if let Some(form) = self.level_mut().form.as_mut() {
                        form.question = None;
                    }
                    self.back_target = walking;
                    self.pop_card(cx);
                    return true;
                }
                _ => {
                    if let Some(form) = self.level_mut().form.as_mut() {
                        form.question = None;
                    }
                }
            }
        }
        if key == "escape" {
            if !self.raise_top_form_question() {
                self.pop_card(cx);
            }
            return true;
        }
        if matches!(key, "up" | "down") {
            return false;
        }
        let selected = self.level().selected;
        let action = {
            let Some(field) = self
                .level()
                .form
                .as_ref()
                .and_then(|form| form.fields.get(selected))
            else {
                return true;
            };
            match &field.value {
                FormValue::Text(_) => match key {
                    "enter" | "return" => FormKey::Next,
                    _ => FormKey::Edit,
                },
                FormValue::Boolean(_) => match key {
                    "enter" | "return" | "space" => FormKey::Flip,
                    _ => FormKey::Consume,
                },
                FormValue::Mods { .. } => match key {
                    "left" => FormKey::Step(-1),
                    "right" => FormKey::Step(1),
                    "space" => FormKey::Toggle,
                    "enter" | "return" => FormKey::Next,
                    _ => FormKey::Consume,
                },
                FormValue::List(_) => match key {
                    "enter" | "return" | "space" => FormKey::Open,
                    _ => FormKey::Consume,
                },
            }
        };
        match action {
            FormKey::Next => self.select_form_row(selected + 1),
            FormKey::Flip => {
                if let Some(form) = self.level_mut().form.as_mut() {
                    form.flip(selected);
                }
            }
            FormKey::Step(direction) => {
                if let Some(form) = self.level_mut().form.as_mut() {
                    form.step_mods(selected, direction);
                }
            }
            FormKey::Toggle => {
                if let Some(form) = self.level_mut().form.as_mut() {
                    form.toggle_mod(selected, None);
                }
            }
            FormKey::Open => self.open_nested_list_card(selected, cx),
            FormKey::Edit => {
                if let Some(form) = self.level_mut().form.as_mut() {
                    if let Some(field) = form.text_mut(selected) {
                        text_edit::apply_edit_key(field, &event.keystroke, || {
                            cx.read_from_clipboard().and_then(|item| item.text())
                        });
                    }
                }
            }
            FormKey::Consume => {}
        }
        true
    }

    pub(super) fn render_entries_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let level = self.level();
        let selected = index == level.selected;
        let focused = self.body_has_focus();
        let ground = RowGround::of(selected, focused);
        let mut row = SettingsRow::rule(("settings-entry", index), self.palette)
            .selected(selected, focused)
            .child(self.row_bounds_canvas(index));
        if index == 0 {
            row = row.child(settings_label_group("+ Add", None, ground, self.palette));
            if level_entries_len(level) == 0 {
                row = row.child(settings_value_text(
                    "Empty",
                    SettingsValueTone::Muted,
                    ground,
                    self.palette,
                ));
            }
        } else if let Some(state) = level.object_array.as_ref() {
            let entry = index - 1;
            if state.key_label.is_some() {
                let key = state
                    .entries
                    .get(entry)
                    .and_then(|entry| entry.key.clone())
                    .unwrap_or_default();
                row = row.child(settings_label_group(key, None, ground, self.palette));
                let summary = level
                    .entries
                    .as_ref()
                    .map(|card| state.value_summary(entry, &card.lists))
                    .unwrap_or_default();
                if !summary.is_empty() {
                    row = row.child(settings_value_text(
                        summary,
                        SettingsValueTone::Muted,
                        ground,
                        self.palette,
                    ));
                }
            } else {
                row = row.child(self.render_chip_row(chip_row_parts(&state.chips(entry)), ground));
            }
        } else if let Some(values) = level.entries.as_ref().and_then(|card| card.values.as_ref()) {
            let value = values.get(index - 1).cloned().unwrap_or_default();
            row = row.child(settings_mono_label(value, ground, self.palette));
        }
        row.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.activate_entries_row(index, cx);
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn render_form_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let level = self.level();
        let selected = index == level.selected;
        let focused = self.body_has_focus();
        let ground = RowGround::of(selected, focused);
        let Some(field) = level.form.as_ref().and_then(|form| form.fields.get(index)) else {
            return div().id(("settings-form-field", index)).into_any_element();
        };
        let mut row = SettingsRow::rule(("settings-form-field", index), self.palette)
            .selected(selected, focused)
            .child(self.row_bounds_canvas(index))
            .child(settings_label_group(
                field.label.clone(),
                None,
                ground,
                self.palette,
            ));
        match &field.value {
            FormValue::Text(text) => {
                let control = if selected && focused {
                    SettingsTextField::live(text.clone(), ground, self.palette)
                } else {
                    SettingsTextField::new(
                        text.text().to_owned(),
                        text.is_empty(),
                        false,
                        ground,
                        self.palette,
                    )
                    .placeholder("Empty")
                };
                row = row.child(control);
            }
            FormValue::Boolean(on) => {
                row = row.child(SettingsToggle::new(*on, ground, self.palette));
            }
            FormValue::Mods {
                options,
                selected: flags,
                cursor,
            } => {
                let mut chips = settings_value_group().gap(px(qol_theme::SPACE_TIGHT));
                for (option_index, option) in options.iter().enumerate() {
                    let on = flags.get(option_index).copied().unwrap_or(false);
                    let cursor_here = selected && focused && option_index == *cursor;
                    chips = chips.child(
                        SettingsModifierChip::new(
                            ("settings-form-mod", index * 64 + option_index),
                            option.clone(),
                            on,
                            cursor_here,
                            ground,
                            self.palette,
                        )
                        .on_click(cx.listener(
                            move |this, event: &ClickEvent, _, cx| {
                                if !event.standard_click() {
                                    return;
                                }
                                cx.stop_propagation();
                                this.level_mut().selected = index;
                                if let Some(form) = this.level_mut().form.as_mut() {
                                    form.toggle_mod(index, Some(option_index));
                                }
                                cx.notify();
                            },
                        )),
                    );
                }
                row = row.child(chips);
            }
            FormValue::List(values) => {
                let noun = self
                    .level_index()
                    .checked_sub(1)
                    .and_then(|below| self.stack.get(below))
                    .and_then(|below| below.entries.as_ref())
                    .and_then(|card| card.lists.get(&field.key))
                    .and_then(|spec| spec.item_label.clone())
                    .unwrap_or_else(|| "item".to_string());
                row = row.child(settings_value_text(
                    count_label(values.len(), &noun),
                    SettingsValueTone::Muted,
                    ground,
                    self.palette,
                ));
            }
        }
        row.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            this.level_mut().selected = index;
            let click = {
                let level = this.level();
                match level
                    .form
                    .as_ref()
                    .and_then(|form| form.fields.get(index))
                    .map(|field| &field.value)
                {
                    Some(FormValue::Boolean(_)) => FormRowClick::Flip,
                    Some(FormValue::List(_)) => FormRowClick::Open,
                    _ => FormRowClick::Select,
                }
            };
            match click {
                FormRowClick::Flip => {
                    if let Some(form) = this.level_mut().form.as_mut() {
                        form.flip(index);
                    }
                }
                FormRowClick::Open => this.open_nested_list_card(index, cx),
                FormRowClick::Select => {}
            }
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn card_hint_bar(&self, bar: SettingsHintBar) -> SettingsHintBar {
        let level = self.level();
        if let Some(form) = level.form.as_ref() {
            let (question, left, right) = form_hints(form, level.selected);
            let bar = bar.left(left).right(right);
            return match question {
                Some(text) => bar.question(text),
                None => bar,
            };
        }
        if level.entries.is_some() {
            let (left, right) = entries_hints(level.selected == 0);
            return bar.left(left).right(right);
        }
        bar
    }

    pub(super) fn raise_top_form_question(&mut self) -> bool {
        let Some(form) = self.level_mut().form.as_mut() else {
            return false;
        };
        if form.question.is_some() {
            return true;
        }
        if !form.changed() {
            return false;
        }
        let question = form.ask();
        form.question = Some(question);
        true
    }

    fn open_entries_form(&mut self, entry: Option<usize>, cx: &mut Context<Self>) {
        let (config_key, source, form) = {
            let level = self.level();
            let Some(card) = level.entries.as_ref() else {
                return;
            };
            let form = match level.object_array.as_ref() {
                Some(state) => EntryForm::for_entry(state, entry, &card.item_label),
                None => EntryForm::for_value(
                    card.values.as_deref().unwrap_or(&[]),
                    entry,
                    &card.item_label,
                ),
            };
            let config_key = level
                .rows
                .first()
                .map(|row| row.config_key.clone())
                .unwrap_or_default();
            let source = level.rows.first().map_or(0, |row| row.source);
            (config_key, source, form)
        };
        let crumb = form.crumb.clone();
        let description = form.sub_header();
        let rows: Vec<Row> = form
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| Row {
                id: format!("form_field_{index}"),
                section_id: None,
                section_label: None,
                label: field.label.clone(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: config_key.clone(),
                default: qol_config::contract::FieldDefault::String(String::new()),
                stream: None,
                action: None,
                visibility: None,
                source,
                control: RowControl::Text(String::new()),
            })
            .collect();
        let section = RowSection {
            label: crumb.clone(),
            description: Some(description),
            rows: (0..rows.len()).collect(),
            source,
        };
        let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
        let Some(destination) = self.card_destination(&crumb, cx) else {
            return;
        };
        let level = Level {
            rows,
            sections: vec![section],
            selected: 0,
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds,
            header: LevelHeader::Card(destination.clone()),
            origin_row: None,
            object_array: None,
            display_layout: None,
            list_card: false,
            live_card: false,
            choose: None,
            entries: None,
            form: Some(form),
            list_item: None,
        };
        self.push_card(destination, level);
        self.sync_scroll();
    }

    fn open_nested_list_card(&mut self, field_index: usize, cx: &mut Context<Self>) {
        let (label, key, values, entry_ref, config_key, source) = {
            let level = self.level();
            let Some(form) = level.form.as_ref() else {
                return;
            };
            let Some(field) = form.fields.get(field_index) else {
                return;
            };
            let FormValue::List(values) = &field.value else {
                return;
            };
            let entry_ref = form
                .name()
                .unwrap_or_else(|| format!("this {}", form.item_label));
            let config_key = level
                .rows
                .first()
                .map(|row| row.config_key.clone())
                .unwrap_or_default();
            let source = level.rows.first().map_or(0, |row| row.source);
            (
                pretty_label(&field.key),
                field.key.clone(),
                values.clone(),
                entry_ref,
                config_key,
                source,
            )
        };
        let spec = self
            .stack
            .len()
            .checked_sub(2)
            .and_then(|below| self.stack.get(below))
            .and_then(|level| level.entries.as_ref())
            .and_then(|card| card.lists.get(&key))
            .cloned();
        let item_label = spec
            .as_ref()
            .and_then(|spec| spec.item_label.clone())
            .unwrap_or_else(|| "item".to_string());
        let description = spec
            .as_ref()
            .and_then(|spec| spec.card_description.clone())
            .map(|description| description.replace("{entry}", &entry_ref));
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let level = entries_card_level(
            &label,
            &config_key,
            source,
            EntriesOrigin {
                object_array: None,
                card: EntriesCard {
                    item_label,
                    lists: BTreeMap::new(),
                    values: Some(values),
                    owner_field: Some(field_index),
                },
                description,
                origin_row: None,
            },
            destination.clone(),
        );
        self.push_card(destination, level);
        self.sync_scroll();
    }

    fn activate_entries_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.level_mut().selected = index;
        if index == 0 {
            self.open_entries_form(None, cx);
        } else {
            self.open_entries_form(Some(index - 1), cx);
        }
    }

    fn remove_entry(&mut self) {
        let selected = self.level().selected;
        if selected == 0 {
            return;
        }
        let entry = selected - 1;
        {
            let level = self.level_mut();
            let Some(card) = level.entries.as_mut() else {
                return;
            };
            if let Some(values) = card.values.as_mut() {
                if entry >= values.len() {
                    return;
                }
                values.remove(entry);
            } else if let Some(state) = level.object_array.as_mut() {
                if !state.remove(entry) {
                    return;
                }
            } else {
                return;
            }
        }
        let entries_index = self.level_index();
        sync_entries_level(&mut self.stack[entries_index]);
        self.height_revision += 1;
        self.write_entries_back(entries_index);
        self.sync_scroll();
    }

    fn save_form(&mut self, cx: &mut Context<Self>) {
        let len = self.stack.len();
        if len < 2 {
            return;
        }
        let blocked = match self.level().form.as_ref().map(EntryForm::ask) {
            Some(FormQuestion::Blocked(index)) => Some(index),
            _ => None,
        };
        if let Some(index) = blocked {
            if let Some(form) = self.level_mut().form.as_mut() {
                form.question = Some(FormQuestion::Blocked(index));
            }
            return;
        }
        let (replacing, output) = {
            let Some(form) = self.level().form.as_ref() else {
                return;
            };
            (form.replacing, form.output())
        };
        let entries_index = len - 2;
        let saved = match output {
            FormOutput::Entry(entry) => {
                let Some(state) = self
                    .stack
                    .get_mut(entries_index)
                    .and_then(|level| level.object_array.as_mut())
                else {
                    return;
                };
                state.save(replacing, entry)
            }
            FormOutput::Value(value) => {
                let Some(values) = self
                    .stack
                    .get_mut(entries_index)
                    .and_then(|level| level.entries.as_mut())
                    .and_then(|card| card.values.as_mut())
                else {
                    return;
                };
                match replacing {
                    Some(index) if index < values.len() => {
                        values[index] = value;
                        index
                    }
                    _ => {
                        values.push(value);
                        values.len() - 1
                    }
                }
            }
        };
        if let Some(level) = self.stack.get_mut(entries_index) {
            sync_entries_level(level);
            level.selected = saved + 1;
        }
        self.height_revision += 1;
        if let Some(form) = self.level_mut().form.as_mut() {
            form.question = None;
        }
        self.write_entries_back(entries_index);
        self.pop_card(cx);
    }

    fn write_entries_back(&mut self, entries_index: usize) {
        let owner = self
            .stack
            .get(entries_index)
            .and_then(|level| level.entries.as_ref())
            .and_then(|card| card.owner_field);
        if let Some(field) = owner {
            let values = self
                .stack
                .get(entries_index)
                .and_then(|level| level.entries.as_ref())
                .and_then(|card| card.values.clone())
                .unwrap_or_default();
            if let Some(form) = entries_index
                .checked_sub(1)
                .and_then(|index| self.stack.get_mut(index))
                .and_then(|level| level.form.as_mut())
            {
                form.set_list(field, values);
            }
            return;
        }
        let written = {
            let (root, rest) = self.stack.split_at_mut(1);
            match entries_index
                .checked_sub(1)
                .and_then(|index| rest.get_mut(index))
            {
                Some(level) => write_root_entries(&mut root[0].rows, level),
                None => false,
            }
        };
        if written {
            self.persist();
        }
    }

    fn select_form_row(&mut self, index: usize) {
        let count = self
            .level()
            .form
            .as_ref()
            .map_or(0, |form| form.fields.len());
        if count == 0 {
            return;
        }
        self.level_mut().selected = index.min(count - 1);
        self.sync_scroll();
    }

    fn level_index(&self) -> usize {
        self.render_level.get().min(self.stack.len() - 1)
    }

    pub(super) fn render_chip_row(&self, parts: Vec<ChipRowPart>, row: RowGround) -> Div {
        let ground = row.rest(self.palette);
        let arrow = match row {
            RowGround::Pane => self.palette.status_muted,
            RowGround::Band => ground.soft,
        };
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
                    .text(TextStyle::Detail)
                    .text_color(rgb(arrow))
                    .child("\u{2192}"),
                ChipRowPart::Chip(chip) => match chip.tone {
                    ChipTone::Modifier | ChipTone::Key => {
                        self.kit.chip(KitChip::KeyText(chip.label.into()), ground)
                    }
                    ChipTone::Plain => self.kit.chip(KitChip::Tag(chip.label.into()), ground),
                },
            });
        }
        strip
    }
}

fn entries_card_level(
    label: &str,
    config_key: &str,
    source: usize,
    origin: EntriesOrigin,
    destination: SettingsDestination,
) -> Level {
    let count = entries_len(&origin.object_array, &origin.card);
    let rows = entries_card_rows(
        config_key,
        source,
        origin.object_array.as_ref(),
        &origin.card,
    );
    let section = RowSection {
        label: label.to_string(),
        description: origin.description,
        rows: (0..rows.len()).collect(),
        source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    Level {
        rows,
        sections: vec![section],
        selected: initial_card_selection(count),
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        header: LevelHeader::Card(destination),
        origin_row: origin.origin_row,
        object_array: origin.object_array,
        display_layout: None,
        list_card: false,
        live_card: false,
        choose: None,
        entries: Some(origin.card),
        form: None,
        list_item: None,
    }
}

fn entries_card_rows(
    config_key: &str,
    source: usize,
    object_array: Option<&ObjectArrayState>,
    card: &EntriesCard,
) -> Vec<Row> {
    let add_id = match card.values.is_some() {
        true => "text_list_add",
        false => "object_array_add",
    };
    let mut rows = vec![entries_row(add_id, "+ Add", config_key, source)];
    if let Some(state) = object_array {
        rows.extend((0..state.entries.len()).map(|index| {
            let label = match state.key_label.is_some() {
                true => state
                    .entries
                    .get(index)
                    .and_then(|entry| entry.key.clone())
                    .unwrap_or_default(),
                false => state.summary(index),
            };
            entries_row(
                &format!("object_array_item_{index}"),
                &label,
                config_key,
                source,
            )
        }));
    } else if let Some(values) = card.values.as_ref() {
        rows.extend(values.iter().enumerate().map(|(index, value)| {
            entries_row(
                &format!("text_list_item_{index}"),
                value,
                config_key,
                source,
            )
        }));
    }
    rows
}

fn entries_row(id: &str, label: &str, config_key: &str, source: usize) -> Row {
    Row {
        id: id.to_string(),
        section_id: None,
        section_label: None,
        label: label.to_string(),
        description: None,
        placeholder: None,
        variant: None,
        config_key: config_key.to_string(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control: RowControl::Text(label.to_string()),
    }
}

fn entries_len(object_array: &Option<ObjectArrayState>, card: &EntriesCard) -> usize {
    if let Some(values) = card.values.as_ref() {
        return values.len();
    }
    object_array.as_ref().map_or(0, |state| state.entries.len())
}

fn level_entries_len(level: &Level) -> usize {
    if let Some(values) = level.entries.as_ref().and_then(|card| card.values.as_ref()) {
        return values.len();
    }
    level
        .object_array
        .as_ref()
        .map_or(0, |state| state.entries.len())
}

fn sync_entries_level(level: &mut Level) {
    let Some(card) = level.entries.as_ref() else {
        return;
    };
    let config_key = level
        .rows
        .first()
        .map(|row| row.config_key.clone())
        .unwrap_or_default();
    let source = level.rows.first().map_or(0, |row| row.source);
    let label = level
        .sections
        .first()
        .map(|section| section.label.clone())
        .unwrap_or_default();
    let description = level
        .sections
        .first()
        .and_then(|section| section.description.clone());
    let rows = entries_card_rows(&config_key, source, level.object_array.as_ref(), card);
    let section = RowSection {
        label,
        description,
        rows: (0..rows.len()).collect(),
        source,
    };
    level.rows = rows;
    level.sections = vec![section];
    level.row_bounds = (0..level.rows.len())
        .map(|_| Rc::new(Cell::new(None)))
        .collect();
    level.selected = level.selected.min(level.rows.len().saturating_sub(1));
}

fn write_root_entries(root_rows: &mut [Row], level: &Level) -> bool {
    let Some(origin) = level.origin_row else {
        return false;
    };
    let Some(row) = root_rows.get_mut(origin) else {
        return false;
    };
    let stored = level.object_array.as_ref().map(|state| {
        ObjectArrayState::from_entries(
            state.key_label.clone(),
            state.schema.clone(),
            state.entries.clone(),
        )
    });
    let values = level.entries.as_ref().and_then(|card| card.values.clone());
    match (stored, values) {
        (Some(state), _) => row.control = RowControl::ObjectArray(state),
        (None, Some(values)) => row.control = RowControl::TextList(values),
        (None, None) => return false,
    }
    true
}

pub(super) fn entries_hints(on_add: bool) -> (Vec<SettingsHint>, Vec<SettingsHint>) {
    let left = vec![
        SettingsHint::new(
            Key::ENTER,
            match on_add {
                true => "add",
                false => "open",
            },
        ),
        SettingsHint::new(Key::UP_DOWN, "move"),
        SettingsHint::new(Key::letter('a'), "add"),
    ];
    let right = vec![SettingsHint::new(Key::ESC, "back")];
    (left, right)
}

pub(super) fn form_hints(
    form: &EntryForm,
    selected: usize,
) -> (Option<String>, Vec<SettingsHint>, Vec<SettingsHint>) {
    if let Some(question) = form.question {
        let left = vec![match question {
            FormQuestion::Save => SettingsHint::new(Key::ENTER, "save").tone(HintTone::Save),
            FormQuestion::Blocked(_) => SettingsHint::new(Key::ENTER, "fill it in"),
        }];
        let right = vec![SettingsHint::new(Key::ESC, "discard").tone(HintTone::Discard)];
        return (Some(form.question_text(question)), left, right);
    }
    let mut left = Vec::new();
    if let Some(field) = form.fields.get(selected) {
        left.push(SettingsHint::new(
            Key::ENTER,
            match &field.value {
                FormValue::Text(_) | FormValue::Mods { .. } => "next",
                FormValue::Boolean(_) => "flip",
                FormValue::List(_) => "edit",
            },
        ));
        if matches!(field.value, FormValue::Mods { .. }) {
            left.push(SettingsHint::new(Key::ARROWS, "move"));
            left.push(SettingsHint::new(Key::SPACE, "toggle"));
        } else {
            left.push(SettingsHint::new(Key::UP_DOWN, "move"));
            if matches!(field.value, FormValue::Text(_)) {
                left.push(SettingsHint::new(Key::TYPE, "edit"));
            }
        }
    } else {
        left.push(SettingsHint::new(Key::UP_DOWN, "move"));
    }
    let right = vec![match form.changed() {
        true => SettingsHint::new(Key::ESC, "back, asks to save"),
        false => SettingsHint::new(Key::ESC, "back"),
    }];
    (None, left, right)
}

fn plain_a_key(event: &KeyDownEvent) -> bool {
    let modifiers = &event.keystroke.modifiers;
    event.keystroke.key == "a" && !modifiers.control && !modifiers.alt && !modifiers.platform
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

#[cfg(test)]
mod tests {
    use crate::key::Key;
    use std::collections::BTreeMap;

    use qol_config::object_array::ItemFieldKind;

    use super::super::super::entry_form::{EntryForm, FormQuestion};
    use super::super::super::object_array_row::{
        Chip, ChipTone, Entry, Item, ItemChips, ObjectArrayState,
    };
    use super::super::super::rows::RowControl;
    use super::super::super::SettingsDestination;
    use super::super::tests::{level, rows};
    use super::{
        entries_card_level, entries_hints, form_hints, sync_entries_level, write_root_entries,
        ChipRowPart, EntriesCard, EntriesOrigin,
    };
    use crate::settings_panel::components::{HintTone, SettingsHint};

    fn object_array_state() -> ObjectArrayState {
        ObjectArrayState::from_entries(
            None,
            vec![("app".to_string(), ItemFieldKind::Text)],
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

    fn card(values: Option<Vec<String>>) -> EntriesCard {
        EntriesCard {
            item_label: "entry".to_string(),
            lists: BTreeMap::new(),
            values,
            owner_field: None,
        }
    }

    fn hint_labels(hints: &[SettingsHint]) -> Vec<String> {
        hints.iter().map(|hint| hint.label.to_string()).collect()
    }

    #[test]
    fn object_array_card_rows_build_the_add_row_first_then_one_row_per_entry() {
        let level = entries_card_level(
            "Rules",
            "rules",
            1,
            EntriesOrigin {
                object_array: Some(object_array_state()),
                card: card(None),
                description: None,
                origin_row: Some(3),
            },
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(level.rows.len(), 3);
        let add = &level.rows[0];
        assert_eq!(add.id, "object_array_add");
        assert_eq!(add.label, "+ Add");
        assert!(matches!(&add.control, RowControl::Text(stored) if stored == "+ Add"));
        let state = level.object_array.as_ref().unwrap();
        for (index, row) in level.rows.iter().skip(1).enumerate() {
            assert_eq!(row.id, format!("object_array_item_{index}"));
            assert_eq!(row.label, state.summary(index));
            assert!(matches!(
                &row.control,
                RowControl::Text(stored) if stored == &state.summary(index)
            ));
            assert_eq!(row.config_key, "rules");
            assert_eq!(row.source, 1);
        }
        assert_eq!(level.selected, 1);
    }

    #[test]
    fn an_empty_object_array_card_selects_the_add_row() {
        let empty = entries_card_level(
            "Rules",
            "rules",
            0,
            EntriesOrigin {
                object_array: Some(ObjectArrayState::from_entries(
                    None,
                    vec![("app".to_string(), ItemFieldKind::Text)],
                    Vec::new(),
                )),
                card: card(None),
                description: None,
                origin_row: Some(3),
            },
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(empty.selected, 0);
        assert_eq!(empty.rows.len(), 1);

        let full = entries_card_level(
            "Rules",
            "rules",
            0,
            EntriesOrigin {
                object_array: Some(object_array_state()),
                card: card(None),
                description: None,
                origin_row: Some(3),
            },
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(full.selected, 1);
    }

    #[test]
    fn removing_an_entry_rebuilds_the_card_rows_and_clamps_selection() {
        let mut level = entries_card_level(
            "Rules",
            "rules",
            0,
            EntriesOrigin {
                object_array: Some(object_array_state()),
                card: card(None),
                description: None,
                origin_row: Some(3),
            },
            SettingsDestination::from_static("Rules"),
        );
        assert_eq!(level.selected, 1);
        level.selected = 2;
        assert!(level.object_array.as_mut().unwrap().remove(1));
        sync_entries_level(&mut level);
        let state = level.object_array.as_ref().unwrap();
        assert_eq!(level.rows.len(), 2);
        assert_eq!(level.rows[0].label, "+ Add");
        assert_eq!(level.rows[1].label, state.summary(0));
        assert_eq!(level.selected, 1);
    }

    #[test]
    fn a_synced_root_row_holds_the_child_entries_in_order() {
        let mut root = level(0);
        root.rows = rows(&[false, false]);
        root.rows[1].control = RowControl::ObjectArray(ObjectArrayState::from_entries(
            None,
            vec![("app".to_string(), ItemFieldKind::Text)],
            Vec::new(),
        ));
        let child = entries_card_level(
            "Rules",
            "rules",
            0,
            EntriesOrigin {
                object_array: Some(object_array_state()),
                card: card(None),
                description: None,
                origin_row: Some(1),
            },
            SettingsDestination::from_static("Rules"),
        );
        assert!(write_root_entries(&mut root.rows, &child));
        let RowControl::ObjectArray(stored) = &root.rows[1].control else {
            panic!("root row holds an object array");
        };
        let entries = child.object_array.as_ref().unwrap().entries.clone();
        assert_eq!(stored.entries, entries);
    }

    #[test]
    fn text_list_child_rows_build_the_add_row_first_then_one_row_per_value() {
        let values = vec!["/usr/bin/zed".to_string(), "/opt/zed".to_string()];
        let level = entries_card_level(
            "Paths",
            "paths",
            0,
            EntriesOrigin {
                object_array: None,
                card: card(Some(values.clone())),
                description: None,
                origin_row: Some(4),
            },
            SettingsDestination::from_static("Paths"),
        );
        assert_eq!(level.rows.len(), 3);
        let add = &level.rows[0];
        assert_eq!(add.id, "text_list_add");
        assert_eq!(add.label, "+ Add");
        assert!(matches!(&add.control, RowControl::Text(stored) if stored == "+ Add"));
        for (index, value) in values.iter().enumerate() {
            let row = &level.rows[index + 1];
            assert_eq!(row.id, format!("text_list_item_{index}"));
            assert_eq!(&row.label, value);
            assert!(matches!(&row.control, RowControl::Text(stored) if stored == value));
            assert_eq!(row.config_key, "paths");
            assert_eq!(row.source, 0);
        }
        assert_eq!(level.selected, 1);
    }

    #[test]
    fn deleting_the_first_real_text_list_item_removes_its_value() {
        let mut level = entries_card_level(
            "Paths",
            "paths",
            0,
            EntriesOrigin {
                object_array: None,
                card: card(Some(vec![
                    "/usr/bin/zed".to_string(),
                    "/opt/zed".to_string(),
                    "~/zed".to_string(),
                ])),
                description: None,
                origin_row: Some(4),
            },
            SettingsDestination::from_static("Paths"),
        );
        level.selected = 1;
        level
            .entries
            .as_mut()
            .unwrap()
            .values
            .as_mut()
            .unwrap()
            .remove(0);
        sync_entries_level(&mut level);
        let values = level.entries.as_ref().unwrap().values.as_ref().unwrap();
        assert_eq!(values, &["/opt/zed".to_string(), "~/zed".to_string()]);
        assert_eq!(level.rows.len(), 3);
        assert_eq!(level.rows[1].label, "/opt/zed");
    }

    #[test]
    fn a_directional_chip_row_renders_the_same_chips_item_chips_produces() {
        let state = ObjectArrayState::from_entries(
            None,
            vec![
                ("from_mods".to_string(), ItemFieldKind::Mods),
                ("to_mods".to_string(), ItemFieldKind::Mods),
                ("keys".to_string(), ItemFieldKind::StringArray),
                ("global".to_string(), ItemFieldKind::Boolean),
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

    fn switchable_form() -> EntryForm {
        let state = ObjectArrayState::list(
            vec![
                ("app".to_string(), ItemFieldKind::Text),
                ("switchable".to_string(), ItemFieldKind::Boolean),
            ],
            Vec::new(),
        );
        EntryForm::for_entry(&state, None, "switchable panel")
    }

    #[test]
    fn form_hints_follow_the_selected_field() {
        let form = switchable_form();
        let (question, left, right) = form_hints(&form, 0);
        assert_eq!(question, None);
        assert_eq!(hint_labels(&left), vec!["next", "move", "edit"]);
        assert_eq!(left[0].key, Some(Key::ENTER));
        assert_eq!(hint_labels(&right), vec!["back"]);
    }

    #[test]
    fn form_hints_note_a_changed_form() {
        let mut form = switchable_form();
        form.text_mut(0).unwrap().insert_str("firefox");
        let (question, _, right) = form_hints(&form, 0);
        assert_eq!(question, None);
        assert_eq!(hint_labels(&right), vec!["back, asks to save"]);
    }

    #[test]
    fn form_hints_carry_the_save_question_in_its_tones() {
        let mut form = switchable_form();
        form.text_mut(0).unwrap().insert_str("firefox");
        form.question = Some(FormQuestion::Save);
        let (question, left, right) = form_hints(&form, 0);
        assert_eq!(question.as_deref(), Some("Save the new switchable panel?"));
        assert_eq!(hint_labels(&left), vec!["save"]);
        assert_eq!(left[0].tone, HintTone::Save);
        assert_eq!(hint_labels(&right), vec!["discard"]);
        assert_eq!(right[0].tone, HintTone::Discard);
    }

    #[test]
    fn form_hints_carry_the_blocked_question() {
        let mut form = switchable_form();
        assert!(form.flip(1));
        let question = form.ask();
        form.question = Some(question);
        let (question, left, right) = form_hints(&form, 0);
        assert_eq!(
            question.as_deref(),
            Some("App is empty, so this switchable panel cannot be saved yet.")
        );
        assert_eq!(hint_labels(&left), vec!["fill it in"]);
        assert_eq!(left[0].tone, HintTone::Plain);
        assert_eq!(hint_labels(&right), vec!["discard"]);
    }

    #[test]
    fn entries_hints_name_add_on_the_add_row_and_open_on_an_entry() {
        let (left, right) = entries_hints(true);
        assert_eq!(hint_labels(&left), vec!["add", "move", "add"]);
        assert_eq!(hint_labels(&right), vec!["back"]);

        let (left, right) = entries_hints(false);
        assert_eq!(hint_labels(&left), vec!["open", "move", "add"]);
        assert_eq!(hint_labels(&right), vec!["back"]);
    }
}
