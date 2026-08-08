use qol_config::contract::{FieldDefault, IndexMap};
use qol_config::object_array::{
    group_item_fields, is_mod_array, ItemField, ItemFieldKind, KNOWN_MODS,
};

use crate::scroll_list::ScrollList;

pub(super) const OBJECT_ARRAY_MAX_VISIBLE: usize = 4;

pub(super) type Item = IndexMap<String, FieldDefault>;

#[derive(Debug)]
pub(super) struct ObjectArrayState {
    pub(super) key_label: Option<String>,
    pub(super) schema: Vec<ItemField>,
    pub(super) entries: Vec<Entry>,
    pub(super) list: ScrollList,
    pub(super) draft: Option<ItemDraft>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Entry {
    pub(super) key: Option<String>,
    pub(super) fields: Item,
}

#[derive(Debug)]
pub(super) struct ItemDraft {
    pub(super) replacing: Option<usize>,
    pub(super) fields: Vec<DraftField>,
    pub(super) selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldRole {
    EntryKey,
    Value,
}

#[derive(Debug)]
pub(super) struct DraftField {
    pub(super) key: String,
    pub(super) kind: ItemFieldKind,
    pub(super) role: FieldRole,
    pub(super) value: DraftValue,
}

#[derive(Debug)]
pub(super) enum DraftValue {
    Mods {
        options: Vec<String>,
        selected: Vec<bool>,
        cursor: usize,
    },
    Text(String),
    Boolean(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObjectArrayOutcome {
    Ignored,
    Handled,
    Persist,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemsIntent {
    Up,
    Down,
    Edit,
    Remove,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DraftIntent {
    Up,
    Down,
    CursorLeft,
    CursorRight,
    Toggle,
    Commit,
    Backspace,
    Insert(String),
    Cancel,
}

fn items_intent(key: &str) -> Option<ItemsIntent> {
    match key {
        "up" => Some(ItemsIntent::Up),
        "down" => Some(ItemsIntent::Down),
        "enter" | "return" | "space" | "right" => Some(ItemsIntent::Edit),
        "backspace" | "delete" => Some(ItemsIntent::Remove),
        "escape" | "left" => Some(ItemsIntent::Close),
        _ => None,
    }
}

fn draft_intent(key: &str, key_char: Option<&str>) -> Option<DraftIntent> {
    match key {
        "up" => Some(DraftIntent::Up),
        "down" | "tab" => Some(DraftIntent::Down),
        "left" => Some(DraftIntent::CursorLeft),
        "right" => Some(DraftIntent::CursorRight),
        "space" => Some(DraftIntent::Toggle),
        "enter" | "return" => Some(DraftIntent::Commit),
        "backspace" => Some(DraftIntent::Backspace),
        "escape" => Some(DraftIntent::Cancel),
        _ => key_char.map(|character| DraftIntent::Insert(character.to_string())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChipTone {
    Modifier,
    Key,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Chip {
    pub(super) label: String,
    pub(super) tone: ChipTone,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ItemChips {
    pub(super) from: Vec<Chip>,
    pub(super) rest: Vec<Chip>,
    pub(super) to: Vec<Chip>,
    pub(super) flags: Vec<String>,
}

impl ItemChips {
    pub(super) fn is_directional(&self) -> bool {
        !self.from.is_empty() && !self.to.is_empty()
    }
}

impl ObjectArrayState {
    pub(super) fn list(schema: Vec<ItemField>, items: Vec<Item>) -> Self {
        Self::new(
            None,
            schema,
            items
                .into_iter()
                .map(|fields| Entry { key: None, fields })
                .collect(),
        )
    }

    pub(super) fn map(
        key_label: String,
        schema: Vec<ItemField>,
        entries: Vec<(String, Item)>,
    ) -> Self {
        Self::new(
            Some(key_label),
            schema,
            entries
                .into_iter()
                .map(|(key, fields)| Entry {
                    key: Some(key),
                    fields,
                })
                .collect(),
        )
    }

    fn new(key_label: Option<String>, schema: Vec<ItemField>, entries: Vec<Entry>) -> Self {
        Self {
            key_label,
            schema,
            entries,
            list: ScrollList::new(OBJECT_ARRAY_MAX_VISIBLE),
            draft: None,
        }
    }

    pub(super) fn items(&self) -> Vec<Item> {
        self.entries
            .iter()
            .map(|entry| entry.fields.clone())
            .collect()
    }

    pub(super) fn keyed_items(&self) -> IndexMap<String, Item> {
        self.entries
            .iter()
            .filter_map(|entry| Some((entry.key.clone()?, entry.fields.clone())))
            .collect()
    }

    pub(super) fn entry_count(&self) -> usize {
        self.entries.len() + 1
    }

    pub(super) fn item_window(&self) -> std::ops::Range<usize> {
        let last_offset = self.entries.len().saturating_sub(self.list.max_visible);
        let start = self.list.scroll_offset.min(last_offset);
        start..(start + self.list.max_visible).min(self.entries.len())
    }

    pub(super) fn add_entry_selected(&self) -> bool {
        self.list.selected >= self.entries.len()
    }

    pub(super) fn move_up(&mut self) {
        self.list.move_up();
        self.list.sync(self.entry_count());
    }

    pub(super) fn move_down(&mut self) {
        self.list.move_down(self.entry_count());
        self.list.sync(self.entry_count());
    }

    pub(super) fn remove_selected(&mut self) -> bool {
        if self.add_entry_selected() {
            return false;
        }
        self.entries.remove(self.list.selected);
        self.list.sync(self.entry_count());
        true
    }

    pub(super) fn open_draft(&mut self) {
        let replacing = (!self.add_entry_selected()).then_some(self.list.selected);
        let seed = replacing.and_then(|index| self.entries.get(index));
        let mut fields = Vec::new();
        if let Some(label) = self.key_label.as_deref() {
            fields.push(DraftField::entry_key(
                label,
                seed.and_then(|entry| entry.key.clone()),
            ));
        }
        fields.extend(
            self.schema
                .iter()
                .map(|(key, kind)| DraftField::seeded(key, *kind, seed.map(|entry| &entry.fields))),
        );
        self.draft = Some(ItemDraft {
            replacing,
            fields,
            selected: 0,
        });
    }

    pub(super) fn commit_draft(&mut self) -> bool {
        let Some(draft) = self.draft.as_ref() else {
            return false;
        };
        let entry = draft.to_entry();
        if self.key_label.is_some() {
            if entry.key.as_deref().unwrap_or_default().is_empty() {
                return false;
            }
        } else if item_is_empty(&entry.fields) {
            return false;
        }
        match draft.replacing {
            Some(index) if index < self.entries.len() => self.entries[index] = entry,
            _ => {
                self.entries.push(entry);
                self.list.selected = self.entries.len() - 1;
            }
        }
        self.draft = None;
        self.list.sync(self.entry_count());
        true
    }

    pub(super) fn handle_key(&mut self, key: &str, key_char: Option<&str>) -> ObjectArrayOutcome {
        if self.draft.is_some() {
            return self.handle_draft_key(key, key_char);
        }
        let Some(intent) = items_intent(key) else {
            return ObjectArrayOutcome::Ignored;
        };
        match intent {
            ItemsIntent::Up => self.move_up(),
            ItemsIntent::Down => self.move_down(),
            ItemsIntent::Edit => self.open_draft(),
            ItemsIntent::Remove => {
                return match self.remove_selected() {
                    true => ObjectArrayOutcome::Persist,
                    false => ObjectArrayOutcome::Handled,
                }
            }
            ItemsIntent::Close => return ObjectArrayOutcome::Close,
        }
        ObjectArrayOutcome::Handled
    }

    fn handle_draft_key(&mut self, key: &str, key_char: Option<&str>) -> ObjectArrayOutcome {
        let Some(intent) = draft_intent(key, key_char) else {
            return ObjectArrayOutcome::Ignored;
        };
        match intent {
            DraftIntent::Commit => {
                return match self.commit_draft() {
                    true => ObjectArrayOutcome::Persist,
                    false => ObjectArrayOutcome::Handled,
                }
            }
            DraftIntent::Cancel => {
                self.draft = None;
                return ObjectArrayOutcome::Handled;
            }
            _ => {}
        }
        let Some(draft) = self.draft.as_mut() else {
            return ObjectArrayOutcome::Ignored;
        };
        match intent {
            DraftIntent::Up => draft.move_up(),
            DraftIntent::Down => draft.move_down(),
            DraftIntent::CursorLeft => draft.step_cursor(-1),
            DraftIntent::CursorRight => draft.step_cursor(1),
            DraftIntent::Toggle => {
                if !draft.toggle() {
                    draft.insert(" ");
                }
            }
            DraftIntent::Backspace => {
                draft.backspace();
            }
            DraftIntent::Insert(character) => {
                draft.insert(&character);
            }
            DraftIntent::Commit | DraftIntent::Cancel => {}
        }
        ObjectArrayOutcome::Handled
    }

    pub(super) fn chips(&self, index: usize) -> ItemChips {
        let Some(entry) = self.entries.get(index) else {
            return ItemChips::default();
        };
        let mut chips = item_chips(&self.schema, &entry.fields);
        if let Some(key) = entry.key.as_ref() {
            chips.from.insert(
                0,
                Chip {
                    label: key.clone(),
                    tone: ChipTone::Key,
                },
            );
        }
        chips
    }
}

impl ItemDraft {
    pub(super) fn entry_count(&self) -> usize {
        self.fields.len() + 1
    }

    pub(super) fn save_entry_selected(&self) -> bool {
        self.selected >= self.fields.len()
    }

    pub(super) fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(super) fn move_down(&mut self) {
        self.selected = (self.selected + 1).min(self.entry_count() - 1);
    }

    pub(super) fn step_cursor(&mut self, direction: isize) {
        let Some(DraftValue::Mods {
            options, cursor, ..
        }) = self
            .fields
            .get_mut(self.selected)
            .map(|field| &mut field.value)
        else {
            return;
        };
        let last = options.len().saturating_sub(1);
        *cursor = cursor.saturating_add_signed(direction).min(last);
    }

    pub(super) fn toggle(&mut self) -> bool {
        let Some(field) = self.fields.get_mut(self.selected) else {
            return false;
        };
        match &mut field.value {
            DraftValue::Mods {
                selected, cursor, ..
            } => match selected.get_mut(*cursor) {
                Some(flag) => {
                    *flag = !*flag;
                    true
                }
                None => false,
            },
            DraftValue::Boolean(value) => {
                *value = !*value;
                true
            }
            DraftValue::Text(_) => false,
        }
    }

    pub(super) fn insert(&mut self, text: &str) -> bool {
        let Some(DraftValue::Text(value)) = self
            .fields
            .get_mut(self.selected)
            .map(|field| &mut field.value)
        else {
            return false;
        };
        value.push_str(text);
        true
    }

    pub(super) fn backspace(&mut self) -> bool {
        let Some(DraftValue::Text(value)) = self
            .fields
            .get_mut(self.selected)
            .map(|field| &mut field.value)
        else {
            return false;
        };
        value.pop().is_some()
    }

    fn to_entry(&self) -> Entry {
        let mut fields = Item::new();
        let mut key = None;
        for field in &self.fields {
            if field.role == FieldRole::EntryKey {
                key = Some(field.display().trim().to_string());
                continue;
            }
            if let Some(value) = field.stored_value() {
                fields.insert(field.key.clone(), value);
            }
        }
        Entry { key, fields }
    }
}

impl DraftField {
    fn seeded(key: &str, kind: ItemFieldKind, seed: Option<&Item>) -> Self {
        let current = seed.and_then(|item| item.get(key));
        Self {
            key: key.to_string(),
            kind,
            role: FieldRole::Value,
            value: DraftValue::seeded(kind, current),
        }
    }

    fn entry_key(label: &str, current: Option<String>) -> Self {
        Self {
            key: label.to_string(),
            kind: ItemFieldKind::Text,
            role: FieldRole::EntryKey,
            value: DraftValue::Text(current.unwrap_or_default()),
        }
    }

    fn stored_value(&self) -> Option<FieldDefault> {
        match (&self.value, self.kind) {
            (DraftValue::Boolean(value), _) => Some(FieldDefault::Boolean(*value)),
            (
                DraftValue::Mods {
                    options, selected, ..
                },
                _,
            ) => {
                let chosen = chosen_options(options, selected);
                (!chosen.is_empty()).then(|| FieldDefault::StringArray(chosen))
            }
            (DraftValue::Text(value), ItemFieldKind::StringArray) => {
                let parts = split_list(value);
                (!parts.is_empty()).then(|| FieldDefault::StringArray(parts))
            }
            (DraftValue::Text(value), ItemFieldKind::Number) => {
                value.trim().parse::<f64>().ok().map(FieldDefault::Number)
            }
            (DraftValue::Text(value), _) => {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| FieldDefault::String(trimmed.to_string()))
            }
        }
    }

    pub(super) fn display(&self) -> String {
        match &self.value {
            DraftValue::Boolean(value) => binary_label(*value).to_string(),
            DraftValue::Mods {
                options, selected, ..
            } => {
                let chosen = chosen_options(options, selected);
                if chosen.is_empty() {
                    "none".to_string()
                } else {
                    chosen.join(" + ")
                }
            }
            DraftValue::Text(value) => value.clone(),
        }
    }
}

impl DraftValue {
    fn seeded(kind: ItemFieldKind, current: Option<&FieldDefault>) -> Self {
        match kind {
            ItemFieldKind::Boolean => {
                Self::Boolean(matches!(current, Some(FieldDefault::Boolean(true))))
            }
            ItemFieldKind::Mods => {
                let values = match current {
                    Some(FieldDefault::StringArray(values)) => values.clone(),
                    _ => Vec::new(),
                };
                let options = mod_options(&values);
                let selected = options
                    .iter()
                    .map(|option| values.contains(option))
                    .collect();
                Self::Mods {
                    options,
                    selected,
                    cursor: 0,
                }
            }
            ItemFieldKind::StringArray => Self::Text(match current {
                Some(FieldDefault::StringArray(values)) => values.join(", "),
                _ => String::new(),
            }),
            ItemFieldKind::Number => Self::Text(match current {
                Some(FieldDefault::Number(value)) => format_number(*value),
                _ => String::new(),
            }),
            ItemFieldKind::Text => Self::Text(match current {
                Some(FieldDefault::String(value)) => value.clone(),
                _ => String::new(),
            }),
        }
    }
}

pub(super) fn item_chips(schema: &[ItemField], item: &Item) -> ItemChips {
    let groups = group_item_fields(schema);
    let mut chips = ItemChips::default();
    for (key, _) in &groups.from {
        chips.from.extend(value_chips(key, item.get(key)));
    }
    for (key, _) in &groups.rest {
        chips.rest.extend(value_chips(key, item.get(key)));
    }
    for (key, _) in &groups.to {
        chips.to.extend(value_chips(key, item.get(key)));
    }
    for (key, _) in &groups.booleans {
        if matches!(item.get(key), Some(FieldDefault::Boolean(true))) {
            chips.flags.push(key.clone());
        }
    }
    chips
}

fn value_chips(key: &str, value: Option<&FieldDefault>) -> Vec<Chip> {
    match value {
        Some(FieldDefault::StringArray(values)) => {
            let tone = if is_mod_array(key, values) {
                ChipTone::Modifier
            } else {
                ChipTone::Key
            };
            values
                .iter()
                .map(|value| Chip {
                    label: value.clone(),
                    tone,
                })
                .collect()
        }
        Some(FieldDefault::String(value)) if !value.is_empty() => vec![Chip {
            label: value.clone(),
            tone: ChipTone::Plain,
        }],
        Some(FieldDefault::Number(value)) => vec![Chip {
            label: format_number(*value),
            tone: ChipTone::Plain,
        }],
        _ => Vec::new(),
    }
}

fn mod_options(current: &[String]) -> Vec<String> {
    let mut options: Vec<String> = KNOWN_MODS.iter().map(|mod_| mod_.to_string()).collect();
    for value in current {
        if !options.contains(value) {
            options.push(value.clone());
        }
    }
    options
}

fn chosen_options(options: &[String], selected: &[bool]) -> Vec<String> {
    options
        .iter()
        .zip(selected)
        .filter(|(_, on)| **on)
        .map(|(option, _)| option.clone())
        .collect()
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn item_is_empty(item: &Item) -> bool {
    !item
        .values()
        .any(|value| !matches!(value, FieldDefault::Boolean(_)))
}

fn binary_label(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        return format!("{value:.0}");
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_rule_schema() -> Vec<ItemField> {
        vec![
            ("from_mods".to_string(), ItemFieldKind::Mods),
            ("to_mods".to_string(), ItemFieldKind::Mods),
            ("keys".to_string(), ItemFieldKind::StringArray),
            ("global".to_string(), ItemFieldKind::Boolean),
        ]
    }

    fn ctrl_to_cmd() -> Item {
        Item::from_iter([
            (
                "from_mods".to_string(),
                FieldDefault::StringArray(vec!["ctrl".into()]),
            ),
            (
                "to_mods".to_string(),
                FieldDefault::StringArray(vec!["cmd".into()]),
            ),
            (
                "keys".to_string(),
                FieldDefault::StringArray(vec!["c".into(), "v".into()]),
            ),
        ])
    }

    fn state() -> ObjectArrayState {
        ObjectArrayState::list(key_rule_schema(), vec![ctrl_to_cmd()])
    }

    fn select_field(draft: &mut ItemDraft, key: &str) {
        draft.selected = draft
            .fields
            .iter()
            .position(|field| field.key == key)
            .expect("field exists");
    }

    #[test]
    fn the_entry_after_the_last_item_adds_a_new_one() {
        let mut state = state();
        assert_eq!(state.entry_count(), 2);
        assert!(!state.add_entry_selected());

        state.move_down();

        assert!(state.add_entry_selected());
        state.open_draft();
        assert_eq!(state.draft.as_ref().unwrap().replacing, None);
    }

    #[test]
    fn opening_a_draft_on_an_item_seeds_every_field_from_it() {
        let mut state = state();
        state.open_draft();
        let draft = state.draft.as_ref().unwrap();

        assert_eq!(draft.replacing, Some(0));
        let displays: Vec<String> = draft.fields.iter().map(DraftField::display).collect();
        assert_eq!(displays, vec!["ctrl", "cmd", "c, v", "off"]);
    }

    #[test]
    fn committing_a_seeded_draft_replaces_the_item_in_place() {
        let mut state = state();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "keys");
        assert!(draft.insert(", x"));
        select_field(draft, "global");
        assert!(draft.toggle());

        assert!(state.commit_draft());

        assert_eq!(state.entries.len(), 1);
        assert_eq!(
            state.entries[0].fields.get("keys"),
            Some(&FieldDefault::StringArray(vec![
                "c".into(),
                "v".into(),
                "x".into()
            ]))
        );
        assert_eq!(
            state.entries[0].fields.get("global"),
            Some(&FieldDefault::Boolean(true))
        );
    }

    #[test]
    fn a_new_draft_appends_and_selects_the_added_item() {
        let mut state = state();
        state.move_down();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "from_mods");
        assert!(draft.toggle());
        select_field(draft, "keys");
        assert!(draft.insert("q"));

        assert!(state.commit_draft());

        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.list.selected, 1);
        assert_eq!(
            state.entries[1].fields.get("from_mods"),
            Some(&FieldDefault::StringArray(vec!["ctrl".into()]))
        );
    }

    #[test]
    fn a_draft_with_nothing_but_flags_is_refused_and_stays_open() {
        let mut state = state();
        state.move_down();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "global");
        assert!(draft.toggle());

        assert!(!state.commit_draft());

        assert_eq!(state.entries.len(), 1);
        assert!(state.draft.is_some());
    }

    #[test]
    fn empty_text_fields_stay_out_of_the_stored_item() {
        let mut state = ObjectArrayState::list(
            vec![
                ("from_mods".to_string(), ItemFieldKind::Mods),
                ("from_key".to_string(), ItemFieldKind::Text),
                ("to_char".to_string(), ItemFieldKind::Text),
            ],
            Vec::new(),
        );
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "to_char");
        assert!(draft.insert("@"));

        assert!(state.commit_draft());

        let keys: Vec<&String> = state.entries[0].fields.keys().collect();
        assert_eq!(keys, vec!["to_char"]);
    }

    #[test]
    fn removing_the_add_entry_is_a_no_op() {
        let mut state = state();
        state.move_down();
        assert!(!state.remove_selected());
        assert_eq!(state.entries.len(), 1);

        state.move_up();
        assert!(state.remove_selected());
        assert!(state.entries.is_empty());
        assert_eq!(state.list.selected, 0);
    }

    #[test]
    fn the_modifier_cursor_stays_inside_the_option_list() {
        let mut state = state();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "from_mods");

        draft.step_cursor(-1);
        assert!(draft.toggle());
        for _ in 0..20 {
            draft.step_cursor(1);
        }
        assert!(draft.toggle());

        let DraftValue::Mods {
            options, selected, ..
        } = &draft.fields[0].value
        else {
            panic!("modifier field");
        };
        assert_eq!(chosen_options(options, selected), vec!["altgr".to_string()]);
    }

    #[test]
    fn a_stored_modifier_outside_the_known_set_stays_editable() {
        let item = Item::from_iter([(
            "from_mods".to_string(),
            FieldDefault::StringArray(vec!["hyper".into()]),
        )]);
        let mut state = ObjectArrayState::list(
            vec![("from_mods".to_string(), ItemFieldKind::Mods)],
            vec![item],
        );
        state.open_draft();

        let DraftValue::Mods {
            options, selected, ..
        } = &state.draft.as_ref().unwrap().fields[0].value
        else {
            panic!("modifier field");
        };
        assert_eq!(options.last(), Some(&"hyper".to_string()));
        assert_eq!(chosen_options(options, selected), vec!["hyper".to_string()]);
    }

    #[test]
    fn chips_split_a_rule_into_its_two_sides() {
        let state = state();
        let chips = state.chips(0);

        assert_eq!(
            chips.from,
            vec![Chip {
                label: "ctrl".into(),
                tone: ChipTone::Modifier
            }]
        );
        assert_eq!(
            chips.to,
            vec![Chip {
                label: "cmd".into(),
                tone: ChipTone::Modifier
            }]
        );
        assert_eq!(
            chips.rest,
            vec![
                Chip {
                    label: "c".into(),
                    tone: ChipTone::Key
                },
                Chip {
                    label: "v".into(),
                    tone: ChipTone::Key
                }
            ]
        );
        assert!(chips.flags.is_empty());
        assert!(chips.is_directional());
    }

    #[test]
    fn a_set_flag_shows_as_a_badge() {
        let mut state = state();
        state.entries[0]
            .fields
            .insert("global".to_string(), FieldDefault::Boolean(true));
        assert_eq!(state.chips(0).flags, vec!["global".to_string()]);
    }

    #[test]
    fn draft_navigation_stops_on_the_save_entry() {
        let mut state = state();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        assert_eq!(draft.entry_count(), 5);

        for _ in 0..20 {
            draft.move_down();
        }
        assert!(draft.save_entry_selected());
        assert!(!draft.toggle());
        assert!(!draft.insert("x"));

        for _ in 0..20 {
            draft.move_up();
        }
        assert_eq!(draft.selected, 0);
    }

    #[test]
    fn item_keys_navigate_edit_remove_and_leave_without_touching_config() {
        let mut state = state();
        let cases = [
            ("j", None, ObjectArrayOutcome::Ignored),
            ("down", None, ObjectArrayOutcome::Handled),
            ("up", None, ObjectArrayOutcome::Handled),
            ("backspace", None, ObjectArrayOutcome::Persist),
            ("backspace", None, ObjectArrayOutcome::Handled),
            ("left", None, ObjectArrayOutcome::Close),
        ];
        for (key, key_char, expected) in cases {
            assert_eq!(state.handle_key(key, key_char), expected, "key: {key}");
        }
    }

    #[test]
    fn typing_a_rule_persists_only_once_the_draft_commits() {
        let mut state = ObjectArrayState::list(key_rule_schema(), Vec::new());
        assert_eq!(state.handle_key("enter", None), ObjectArrayOutcome::Handled);
        assert!(state.draft.is_some());

        let typed = [
            ("space", Some(" ")),
            ("down", None),
            ("right", None),
            ("right", None),
            ("right", None),
            ("space", Some(" ")),
            ("down", None),
            ("q", Some("q")),
        ];
        for (key, key_char) in typed {
            assert_eq!(
                state.handle_key(key, key_char),
                ObjectArrayOutcome::Handled,
                "key: {key}"
            );
        }
        assert!(state.entries.is_empty());

        assert_eq!(state.handle_key("enter", None), ObjectArrayOutcome::Persist);

        assert_eq!(state.entries.len(), 1);
        assert_eq!(
            state.entries[0].fields.get("from_mods"),
            Some(&FieldDefault::StringArray(vec!["ctrl".into()]))
        );
        assert_eq!(
            state.entries[0].fields.get("to_mods"),
            Some(&FieldDefault::StringArray(vec!["cmd".into()]))
        );
        assert_eq!(
            state.entries[0].fields.get("keys"),
            Some(&FieldDefault::StringArray(vec!["q".into()]))
        );
    }

    #[test]
    fn space_types_into_a_text_field_instead_of_toggling() {
        let mut state = state();
        state.handle_key("enter", None);
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "keys");

        state.handle_key("space", Some(" "));

        assert_eq!(state.draft.as_ref().unwrap().fields[2].display(), "c, v ");
    }

    #[test]
    fn escape_drops_the_draft_and_then_leaves_the_list() {
        let mut state = state();
        state.handle_key("enter", None);
        assert!(state.draft.is_some());

        assert_eq!(
            state.handle_key("escape", None),
            ObjectArrayOutcome::Handled
        );
        assert!(state.draft.is_none());
        assert_eq!(state.entries.len(), 1);

        assert_eq!(state.handle_key("escape", None), ObjectArrayOutcome::Close);
    }

    fn app_map() -> ObjectArrayState {
        ObjectArrayState::map(
            "App ID".to_string(),
            vec![("paths".to_string(), ItemFieldKind::StringArray)],
            vec![(
                "idea".to_string(),
                Item::from_iter([(
                    "paths".to_string(),
                    FieldDefault::StringArray(vec!["/usr/bin/idea".into()]),
                )]),
            )],
        )
    }

    #[test]
    fn a_keyed_entry_edits_its_key_alongside_its_fields() {
        let mut state = app_map();
        state.open_draft();
        let draft = state.draft.as_ref().unwrap();

        assert_eq!(draft.fields[0].key, "App ID");
        assert_eq!(draft.fields[0].role, FieldRole::EntryKey);
        assert_eq!(draft.fields[0].display(), "idea");
        assert_eq!(draft.fields[1].display(), "/usr/bin/idea");

        state.handle_key("backspace", None);
        assert_eq!(state.handle_key("enter", None), ObjectArrayOutcome::Persist);

        let keyed = state.keyed_items();
        assert_eq!(keyed.keys().collect::<Vec<_>>(), vec!["ide"]);
        assert_eq!(
            keyed["ide"].get("paths"),
            Some(&FieldDefault::StringArray(vec!["/usr/bin/idea".into()]))
        );
    }

    #[test]
    fn a_keyed_entry_without_a_key_is_refused() {
        let mut state = app_map();
        state.move_down();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        draft.selected = 1;
        assert!(draft.insert("/opt/zed"));

        assert!(!state.commit_draft());
        assert_eq!(state.entries.len(), 1);
        assert!(state.draft.is_some());
    }

    #[test]
    fn a_keyed_entry_shows_its_key_first_in_the_summary() {
        let state = app_map();
        let chips = state.chips(0);
        assert_eq!(
            chips.from.first().map(|chip| chip.label.as_str()),
            Some("idea")
        );
    }

    #[test]
    fn backspace_only_edits_text_fields() {
        let mut state = state();
        state.open_draft();
        let draft = state.draft.as_mut().unwrap();
        select_field(draft, "keys");
        assert!(draft.backspace());
        assert_eq!(draft.fields[2].display(), "c, ");

        select_field(draft, "from_mods");
        assert!(!draft.backspace());
    }

    #[test]
    fn the_item_window_never_hides_a_row_to_make_space_for_the_add_entry() {
        let mut state = ObjectArrayState::list(
            vec![("app".to_string(), ItemFieldKind::Text)],
            (0..OBJECT_ARRAY_MAX_VISIBLE)
                .map(|n| {
                    Item::from_iter([("app".to_string(), FieldDefault::String(n.to_string()))])
                })
                .collect(),
        );

        for _ in 0..OBJECT_ARRAY_MAX_VISIBLE {
            state.move_down();
        }

        assert!(
            state.add_entry_selected(),
            "selection reached the add entry"
        );
        assert_eq!(
            state.item_window(),
            0..OBJECT_ARRAY_MAX_VISIBLE,
            "a full page of items stays visible while the add entry is selected"
        );
    }

    #[test]
    fn the_item_window_follows_the_selection_past_a_full_page() {
        let mut state = ObjectArrayState::list(
            vec![("app".to_string(), ItemFieldKind::Text)],
            (0..OBJECT_ARRAY_MAX_VISIBLE + 3)
                .map(|n| {
                    Item::from_iter([("app".to_string(), FieldDefault::String(n.to_string()))])
                })
                .collect(),
        );

        for _ in 0..OBJECT_ARRAY_MAX_VISIBLE + 1 {
            state.move_down();
        }

        let window = state.item_window();
        assert_eq!(window.len(), OBJECT_ARRAY_MAX_VISIBLE);
        assert!(
            window.contains(&state.list.selected),
            "selection stays visible"
        );
    }
}
