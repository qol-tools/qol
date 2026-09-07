use indexmap::IndexMap;

use crate::contract::{FieldDefault, FieldKind};
use crate::normalized::ResolvedItemSpec;

pub const KNOWN_MODS: [&str; 6] = ["ctrl", "shift", "alt", "cmd", "ralt", "altgr"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemFieldKind {
    Mods,
    StringArray,
    Text,
    Number,
    Boolean,
}

pub type ItemField = (String, ItemFieldKind);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ItemFieldGroups {
    pub from: Vec<ItemField>,
    pub to: Vec<ItemField>,
    pub rest: Vec<ItemField>,
    pub booleans: Vec<ItemField>,
}

pub fn item_schema(
    item: Option<&ResolvedItemSpec>,
    values: &[IndexMap<String, FieldDefault>],
) -> Vec<ItemField> {
    match item {
        Some(item) => item
            .fields
            .iter()
            .map(|(key, kind)| (key.clone(), declared_kind(key, *kind)))
            .collect(),
        None => inferred_schema(values),
    }
}

pub fn group_item_fields(schema: &[ItemField]) -> ItemFieldGroups {
    let mut groups = ItemFieldGroups::default();
    for field in schema {
        let (key, kind) = field;
        if *kind == ItemFieldKind::Boolean {
            groups.booleans.push(field.clone());
            continue;
        }
        match key_prefix(key) {
            Some("from") => groups.from.push(field.clone()),
            Some("to") => groups.to.push(field.clone()),
            _ => groups.rest.push(field.clone()),
        }
    }
    groups
}

pub fn pretty_label(key: &str) -> String {
    let mut label = String::with_capacity(key.len());
    let mut start_of_word = true;
    for character in key.chars() {
        if character == '_' || character == '-' {
            label.push(' ');
            start_of_word = true;
            continue;
        }
        if start_of_word {
            label.extend(character.to_uppercase());
            start_of_word = false;
            continue;
        }
        label.push(character);
    }
    label
}

pub fn is_mod_array(key: &str, values: &[String]) -> bool {
    if values.is_empty() {
        return key.ends_with("_mods");
    }
    values
        .iter()
        .all(|value| KNOWN_MODS.contains(&value.as_str()))
}

fn declared_kind(key: &str, kind: FieldKind) -> ItemFieldKind {
    match kind {
        FieldKind::StringArray if key.ends_with("_mods") => ItemFieldKind::Mods,
        FieldKind::StringArray => ItemFieldKind::StringArray,
        FieldKind::Boolean => ItemFieldKind::Boolean,
        FieldKind::Number => ItemFieldKind::Number,
        _ => ItemFieldKind::Text,
    }
}

fn inferred_schema(values: &[IndexMap<String, FieldDefault>]) -> Vec<ItemField> {
    let mut schema: Vec<ItemField> = Vec::new();
    for item in values {
        for (key, value) in item {
            if schema.iter().any(|(known, _)| known == key) {
                continue;
            }
            schema.push((key.clone(), inferred_kind(key, value)));
        }
    }
    schema
}

fn inferred_kind(key: &str, value: &FieldDefault) -> ItemFieldKind {
    match value {
        FieldDefault::Boolean(_) => ItemFieldKind::Boolean,
        FieldDefault::Number(_) => ItemFieldKind::Number,
        FieldDefault::StringArray(values) if is_mod_array(key, values) => ItemFieldKind::Mods,
        FieldDefault::StringArray(_) => ItemFieldKind::StringArray,
        _ => ItemFieldKind::Text,
    }
}

fn key_prefix(key: &str) -> Option<&str> {
    key.split_once('_').map(|(prefix, _)| prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(fields: &[(&str, FieldKind)]) -> ResolvedItemSpec {
        ResolvedItemSpec {
            fields: fields
                .iter()
                .map(|(key, kind)| ((*key).to_string(), *kind))
                .collect(),
        }
    }

    fn item(entries: &[(&str, FieldDefault)]) -> IndexMap<String, FieldDefault> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn declared_string_arrays_named_mods_edit_as_modifier_toggles() {
        let cases = [
            ("from_mods", FieldKind::StringArray, ItemFieldKind::Mods),
            ("to_mods", FieldKind::StringArray, ItemFieldKind::Mods),
            ("keys", FieldKind::StringArray, ItemFieldKind::StringArray),
            ("to_char", FieldKind::String, ItemFieldKind::Text),
            ("global", FieldKind::Boolean, ItemFieldKind::Boolean),
            ("weight", FieldKind::Number, ItemFieldKind::Number),
        ];
        for (key, declared, expected) in cases {
            let schema = item_schema(Some(&spec(&[(key, declared)])), &[]);
            assert_eq!(schema, vec![(key.to_string(), expected)], "key: {key}");
        }
    }

    #[test]
    fn missing_item_spec_infers_the_schema_from_stored_values() {
        let values = [
            item(&[
                ("from_mods", FieldDefault::StringArray(vec!["ctrl".into()])),
                ("button", FieldDefault::String("left".into())),
            ]),
            item(&[("global", FieldDefault::Boolean(true))]),
        ];

        assert_eq!(
            item_schema(None, &values),
            vec![
                ("from_mods".to_string(), ItemFieldKind::Mods),
                ("button".to_string(), ItemFieldKind::Text),
                ("global".to_string(), ItemFieldKind::Boolean),
            ]
        );
    }

    #[test]
    fn an_arbitrary_string_array_infers_as_a_plain_list() {
        let values = [item(&[(
            "keys",
            FieldDefault::StringArray(vec!["c".into(), "v".into()]),
        )])];
        assert_eq!(
            item_schema(None, &values),
            vec![("keys".to_string(), ItemFieldKind::StringArray)]
        );
    }

    #[test]
    fn grouping_splits_a_rule_into_from_to_rest_and_flags() {
        let schema = vec![
            ("from_mods".to_string(), ItemFieldKind::Mods),
            ("to_mods".to_string(), ItemFieldKind::Mods),
            ("button".to_string(), ItemFieldKind::Text),
            ("global".to_string(), ItemFieldKind::Boolean),
        ];

        let groups = group_item_fields(&schema);

        assert_eq!(groups.from, vec![schema[0].clone()]);
        assert_eq!(groups.to, vec![schema[1].clone()]);
        assert_eq!(groups.rest, vec![schema[2].clone()]);
        assert_eq!(groups.booleans, vec![schema[3].clone()]);
    }

    #[test]
    fn empty_mod_lists_stay_modifier_fields_only_when_the_key_says_so() {
        assert!(is_mod_array("from_mods", &[]));
        assert!(!is_mod_array("keys", &[]));
        assert!(is_mod_array("keys", &["ctrl".into()]));
        assert!(!is_mod_array("keys", &["c".into()]));
    }

    #[test]
    fn keys_render_as_capitalized_words() {
        let cases = [
            ("from_mods", "From Mods"),
            ("to-char", "To Char"),
            ("button", "Button"),
            ("", ""),
        ];
        for (key, expected) in cases {
            assert_eq!(pretty_label(key), expected, "key: {key}");
        }
    }
}
