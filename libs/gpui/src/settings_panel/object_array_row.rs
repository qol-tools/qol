use std::collections::BTreeMap;

use qol_config::contract::{FieldDefault, IndexMap, NestedListSpec};
use qol_config::object_array::{
    group_item_fields, is_mod_array, pretty_label, ItemField, ItemFieldKind, KNOWN_MODS,
};

use super::entry_form::count_label;

pub(super) type Item = IndexMap<String, FieldDefault>;

#[derive(Debug)]
pub(super) struct ObjectArrayState {
    pub(super) key_label: Option<String>,
    pub(super) schema: Vec<ItemField>,
    pub(super) entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Entry {
    pub(super) key: Option<String>,
    pub(super) fields: Item,
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
        }
    }

    pub(super) fn from_entries(
        key_label: Option<String>,
        schema: Vec<ItemField>,
        entries: Vec<Entry>,
    ) -> Self {
        Self::new(key_label, schema, entries)
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

    pub(super) fn save(&mut self, replacing: Option<usize>, entry: Entry) -> usize {
        if let Some(index) = replacing.filter(|index| *index < self.entries.len()) {
            self.entries[index] = entry;
            return index;
        }
        self.entries.push(entry);
        self.entries.len() - 1
    }

    pub(super) fn remove(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.entries.remove(index);
        true
    }

    pub(super) fn value_summary(
        &self,
        index: usize,
        lists: &BTreeMap<String, NestedListSpec>,
    ) -> String {
        let Some(entry) = self.entries.get(index) else {
            return String::new();
        };
        let parts: Vec<String> = self
            .schema
            .iter()
            .filter_map(|(key, kind)| summary_part(key, kind, entry.fields.get(key)?, lists))
            .collect();
        parts.join(" \u{b7} ")
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

    pub(super) fn summary(&self, index: usize) -> String {
        let chips = self.chips(index);
        if chips.is_directional() {
            let shared = shared_key_chip(&chips.rest);
            let mut from: Vec<&str> = chips.from.iter().map(|chip| chip.label.as_str()).collect();
            from.extend(shared.iter().map(|chip| chip.label.as_str()));
            let mut to: Vec<&str> = chips.to.iter().map(|chip| chip.label.as_str()).collect();
            to.extend(shared.iter().map(|chip| chip.label.as_str()));
            return format!("{} \u{2192} {}", from.join(" + "), to.join(" + "));
        }
        let mut parts: Vec<&str> = chips
            .from
            .iter()
            .chain(&chips.rest)
            .chain(&chips.to)
            .map(|chip| chip.label.as_str())
            .collect();
        parts.extend(chips.flags.iter().map(String::as_str));
        parts.join(" + ")
    }
}

fn summary_part(
    key: &str,
    kind: &ItemFieldKind,
    value: &FieldDefault,
    lists: &BTreeMap<String, NestedListSpec>,
) -> Option<String> {
    match (kind, value) {
        (ItemFieldKind::Boolean, FieldDefault::Boolean(true)) => {
            Some(pretty_label(key).to_lowercase())
        }
        (ItemFieldKind::Mods, FieldDefault::StringArray(values)) if !values.is_empty() => {
            Some(values.join(" + "))
        }
        (ItemFieldKind::StringArray, FieldDefault::StringArray(values)) if !values.is_empty() => {
            let noun = lists
                .get(key)
                .and_then(|spec| spec.item_label.as_deref())
                .unwrap_or("item");
            Some(count_label(values.len(), noun))
        }
        (ItemFieldKind::Number, FieldDefault::Number(number)) => Some(format_number(*number)),
        (ItemFieldKind::Text, FieldDefault::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

pub(super) fn shared_key_chip(rest: &[Chip]) -> Option<Chip> {
    match rest.len() {
        0 => None,
        1 => rest.first().cloned(),
        count => Some(Chip {
            label: format!("{count} keys"),
            tone: ChipTone::Key,
        }),
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

pub(super) fn mod_options(current: &[String]) -> Vec<String> {
    let mut options: Vec<String> = KNOWN_MODS.iter().map(|mod_| mod_.to_string()).collect();
    for value in current {
        if !options.contains(value) {
            options.push(value.clone());
        }
    }
    options
}

pub(super) fn chosen_options(options: &[String], selected: &[bool]) -> Vec<String> {
    options
        .iter()
        .zip(selected)
        .filter(|(_, on)| **on)
        .map(|(option, _)| option.clone())
        .collect()
}

pub(super) fn item_is_empty(item: &Item) -> bool {
    !item
        .values()
        .any(|value| !matches!(value, FieldDefault::Boolean(_)))
}

pub(super) fn format_number(value: f64) -> String {
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

    #[test]
    fn saving_replaces_in_place_and_appends() {
        let mut state = state();
        let replacement = Entry {
            key: None,
            fields: Item::from_iter([(
                "from_mods".to_string(),
                FieldDefault::StringArray(vec!["alt".into()]),
            )]),
        };

        assert_eq!(state.save(Some(0), replacement.clone()), 0);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0], replacement);

        assert_eq!(state.save(None, replacement.clone()), 1);
        assert_eq!(state.entries.len(), 2);

        assert_eq!(state.save(Some(9), replacement), 2);
        assert_eq!(state.entries.len(), 3);
    }

    #[test]
    fn removing_an_out_of_range_entry_changes_nothing() {
        let mut state = state();

        assert!(!state.remove(1));
        assert_eq!(state.entries.len(), 1);

        assert!(state.remove(0));
        assert!(state.entries.is_empty());
    }

    #[test]
    fn a_map_entry_summarises_its_non_empty_values_in_schema_order() {
        let lists = BTreeMap::from([(
            "paths".to_string(),
            NestedListSpec {
                item_label: Some("path".to_string()),
                card_description: Some("Where to look for {entry}.".to_string()),
            },
        )]);
        let state = ObjectArrayState::map(
            "App ID".to_string(),
            vec![
                ("name".to_string(), ItemFieldKind::Text),
                ("paths".to_string(), ItemFieldKind::StringArray),
            ],
            vec![(
                "zed".to_string(),
                Item::from_iter([(
                    "paths".to_string(),
                    FieldDefault::StringArray(vec!["a".into(), "b".into(), "c".into()]),
                )]),
            )],
        );

        assert_eq!(state.value_summary(0, &lists), "3 paths");
        assert_eq!(state.value_summary(4, &lists), "");
    }

    #[test]
    fn a_single_value_map_entry_summarises_as_its_text() {
        let state = ObjectArrayState::map(
            "Display".to_string(),
            vec![("policy".to_string(), ItemFieldKind::Text)],
            vec![(
                "eDP-1".to_string(),
                Item::from_iter([("policy".to_string(), FieldDefault::String("auto".into()))]),
            )],
        );

        assert_eq!(state.value_summary(0, &BTreeMap::new()), "auto");
    }

    #[test]
    fn a_map_entry_joins_mods_flags_and_values_in_schema_order() {
        let mut state = ObjectArrayState::map(
            "Rule".to_string(),
            vec![
                ("from_mods".to_string(), ItemFieldKind::Mods),
                ("keys".to_string(), ItemFieldKind::StringArray),
                ("global".to_string(), ItemFieldKind::Boolean),
                ("weight".to_string(), ItemFieldKind::Number),
            ],
            vec![(
                "rule".to_string(),
                Item::from_iter([
                    (
                        "from_mods".to_string(),
                        FieldDefault::StringArray(vec!["ctrl".into(), "alt".into()]),
                    ),
                    (
                        "keys".to_string(),
                        FieldDefault::StringArray(vec!["c".into(), "v".into()]),
                    ),
                    ("global".to_string(), FieldDefault::Boolean(false)),
                    ("weight".to_string(), FieldDefault::Number(1.5)),
                ]),
            )],
        );

        assert_eq!(
            state.value_summary(0, &BTreeMap::new()),
            "ctrl + alt \u{b7} 2 items \u{b7} 1.5"
        );

        state.entries[0]
            .fields
            .insert("global".to_string(), FieldDefault::Boolean(true));

        assert_eq!(
            state.value_summary(0, &BTreeMap::new()),
            "ctrl + alt \u{b7} 2 items \u{b7} global \u{b7} 1.5"
        );
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
}
