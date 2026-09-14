use qol_config::contract::FieldDefault;
use qol_config::object_array::{pretty_label, ItemFieldKind};

use super::object_array_row::{
    chosen_options, format_number, item_is_empty, mod_options, Entry, Item, ObjectArrayState,
};
use crate::text_edit::TextField;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum FormValue {
    Text(TextField),
    Boolean(bool),
    Mods {
        options: Vec<String>,
        selected: Vec<bool>,
        cursor: usize,
    },
    List(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormRole {
    EntryKey,
    Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FormField {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) kind: ItemFieldKind,
    pub(super) role: FormRole,
    pub(super) value: FormValue,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum FormOutput {
    Entry(Entry),
    Value(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormQuestion {
    Save,
    Blocked(usize),
}

#[derive(Debug)]
pub(super) struct EntryForm {
    pub(super) replacing: Option<usize>,
    pub(super) item_label: String,
    pub(super) crumb: String,
    pub(super) fields: Vec<FormField>,
    pub(super) question: Option<FormQuestion>,
    value_form: bool,
    initial: FormOutput,
}

impl EntryForm {
    pub(super) fn for_entry(
        state: &ObjectArrayState,
        replacing: Option<usize>,
        item_label: &str,
    ) -> Self {
        let seed = replacing.and_then(|index| state.entries.get(index));
        let mut fields = Vec::new();
        if let Some(label) = state.key_label.as_deref() {
            let current = seed.and_then(|entry| entry.key.clone()).unwrap_or_default();
            fields.push(FormField {
                key: label.to_string(),
                label: label.to_string(),
                kind: ItemFieldKind::Text,
                role: FormRole::EntryKey,
                value: FormValue::Text(TextField::with_text(current)),
            });
        }
        fields.extend(state.schema.iter().map(|(key, kind)| FormField {
            key: key.clone(),
            label: pretty_label(key),
            kind: *kind,
            role: FormRole::Value,
            value: seeded_value(*kind, seed.and_then(|entry| entry.fields.get(key))),
        }));
        let crumb = match replacing {
            None => "add".to_string(),
            Some(_) => seed
                .and_then(|entry| entry.key.as_deref())
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
                .or_else(|| first_text(&fields))
                .unwrap_or_else(|| "entry".to_string()),
        };
        Self::build(fields, replacing, item_label, crumb, false)
    }

    pub(super) fn for_value(values: &[String], replacing: Option<usize>, item_label: &str) -> Self {
        let current = replacing
            .and_then(|index| values.get(index))
            .cloned()
            .unwrap_or_default();
        let fields = vec![FormField {
            key: "value".to_string(),
            label: pretty_label(item_label),
            kind: ItemFieldKind::Text,
            role: FormRole::Value,
            value: FormValue::Text(TextField::with_text(current)),
        }];
        let crumb = match replacing {
            None => "add".to_string(),
            Some(_) => "edit".to_string(),
        };
        Self::build(fields, replacing, item_label, crumb, true)
    }

    fn build(
        fields: Vec<FormField>,
        replacing: Option<usize>,
        item_label: &str,
        crumb: String,
        value_form: bool,
    ) -> Self {
        let initial = form_output(&fields, value_form);
        Self {
            replacing,
            item_label: item_label.to_string(),
            crumb,
            fields,
            question: None,
            value_form,
            initial,
        }
    }

    pub(super) fn output(&self) -> FormOutput {
        form_output(&self.fields, self.value_form)
    }

    pub(super) fn changed(&self) -> bool {
        self.output() != self.initial
    }

    pub(super) fn empty_required(&self) -> Option<usize> {
        if self.value_form {
            let empty = self
                .fields
                .first()
                .and_then(FormField::text)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true);
            return empty.then_some(0);
        }
        if self.fields.first().map(|field| field.role) == Some(FormRole::EntryKey) {
            let empty = self
                .fields
                .first()
                .and_then(FormField::text)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true);
            return empty.then_some(0);
        }
        let FormOutput::Entry(entry) = self.output() else {
            return Some(0);
        };
        if !item_is_empty(&entry.fields) {
            return None;
        }
        Some(
            self.fields
                .iter()
                .position(|field| {
                    field.role == FormRole::Value
                        && matches!(field.kind, ItemFieldKind::Text | ItemFieldKind::Number)
                })
                .or_else(|| {
                    self.fields.iter().position(|field| {
                        field.role == FormRole::Value && field.kind != ItemFieldKind::Boolean
                    })
                })
                .unwrap_or(0),
        )
    }

    pub(super) fn ask(&self) -> FormQuestion {
        match self.empty_required() {
            Some(index) => FormQuestion::Blocked(index),
            None => FormQuestion::Save,
        }
    }

    pub(super) fn name(&self) -> Option<String> {
        let key = self
            .fields
            .first()
            .filter(|field| field.role == FormRole::EntryKey)
            .and_then(FormField::text)
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if let Some(key) = key {
            return Some(key.to_string());
        }
        self.fields
            .iter()
            .filter(|field| field.role == FormRole::Value && field.kind == ItemFieldKind::Text)
            .filter_map(FormField::text)
            .map(str::trim)
            .find(|text| !text.is_empty())
            .map(str::to_string)
    }

    pub(super) fn sub_header(&self) -> String {
        match self.replacing {
            None => format!("a new {}.", self.item_label),
            Some(_) => format!("{}.", with_article(&self.item_label)),
        }
    }

    pub(super) fn question_text(&self, question: FormQuestion) -> String {
        match question {
            FormQuestion::Save => match self.replacing {
                None => format!("Save the new {}?", self.item_label),
                Some(_) => format!("Save changes to {}?", self.crumb),
            },
            FormQuestion::Blocked(index) => {
                let label = self
                    .fields
                    .get(index)
                    .map(|field| field.label.as_str())
                    .unwrap_or_default();
                format!(
                    "{label} is empty, so this {} cannot be saved yet.",
                    self.item_label
                )
            }
        }
    }

    pub(super) fn text_mut(&mut self, index: usize) -> Option<&mut TextField> {
        match &mut self.fields.get_mut(index)?.value {
            FormValue::Text(field) => Some(field),
            _ => None,
        }
    }

    pub(super) fn flip(&mut self, index: usize) -> bool {
        let Some(field) = self.fields.get_mut(index) else {
            return false;
        };
        match &mut field.value {
            FormValue::Boolean(value) => {
                *value = !*value;
                true
            }
            _ => false,
        }
    }

    pub(super) fn step_mods(&mut self, index: usize, direction: isize) -> bool {
        let Some(field) = self.fields.get_mut(index) else {
            return false;
        };
        let FormValue::Mods {
            options, cursor, ..
        } = &mut field.value
        else {
            return false;
        };
        let last = options.len().saturating_sub(1);
        *cursor = cursor.saturating_add_signed(direction).min(last);
        true
    }

    pub(super) fn toggle_mod(&mut self, index: usize, option: Option<usize>) -> bool {
        let Some(field) = self.fields.get_mut(index) else {
            return false;
        };
        let FormValue::Mods {
            selected, cursor, ..
        } = &mut field.value
        else {
            return false;
        };
        if let Some(target) = option {
            if target >= selected.len() {
                return false;
            }
            *cursor = target;
        }
        let Some(flag) = selected.get_mut(*cursor) else {
            return false;
        };
        *flag = !*flag;
        true
    }

    pub(super) fn set_list(&mut self, index: usize, values: Vec<String>) -> bool {
        let Some(field) = self.fields.get_mut(index) else {
            return false;
        };
        match &mut field.value {
            FormValue::List(current) => {
                *current = values;
                true
            }
            _ => false,
        }
    }
}

impl FormField {
    fn text(&self) -> Option<&str> {
        match &self.value {
            FormValue::Text(field) => Some(field.text()),
            _ => None,
        }
    }

    fn stored(&self) -> Option<FieldDefault> {
        match (&self.value, self.kind) {
            (FormValue::Boolean(value), _) => Some(FieldDefault::Boolean(*value)),
            (
                FormValue::Mods {
                    options, selected, ..
                },
                _,
            ) => {
                let chosen = chosen_options(options, selected);
                (!chosen.is_empty()).then(|| FieldDefault::StringArray(chosen))
            }
            (FormValue::List(values), _) => {
                (!values.is_empty()).then(|| FieldDefault::StringArray(values.clone()))
            }
            (FormValue::Text(field), ItemFieldKind::Number) => field
                .text()
                .trim()
                .parse::<f64>()
                .ok()
                .map(FieldDefault::Number),
            (FormValue::Text(field), _) => {
                let trimmed = field.text().trim();
                (!trimmed.is_empty()).then(|| FieldDefault::String(trimmed.to_string()))
            }
        }
    }
}

fn seeded_value(kind: ItemFieldKind, current: Option<&FieldDefault>) -> FormValue {
    match kind {
        ItemFieldKind::Boolean => {
            FormValue::Boolean(matches!(current, Some(FieldDefault::Boolean(true))))
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
            FormValue::Mods {
                options,
                selected,
                cursor: 0,
            }
        }
        ItemFieldKind::StringArray => FormValue::List(match current {
            Some(FieldDefault::StringArray(values)) => values.clone(),
            _ => Vec::new(),
        }),
        ItemFieldKind::Number => FormValue::Text(TextField::with_text(match current {
            Some(FieldDefault::Number(value)) => format_number(*value),
            _ => String::new(),
        })),
        ItemFieldKind::Text => FormValue::Text(TextField::with_text(match current {
            Some(FieldDefault::String(value)) => value.clone(),
            _ => String::new(),
        })),
    }
}

fn first_text(fields: &[FormField]) -> Option<String> {
    fields
        .iter()
        .filter(|field| field.role == FormRole::Value && field.kind == ItemFieldKind::Text)
        .filter_map(FormField::text)
        .map(str::trim)
        .find(|text| !text.is_empty())
        .map(str::to_string)
}

fn form_output(fields: &[FormField], value_form: bool) -> FormOutput {
    if value_form {
        let text = fields
            .first()
            .and_then(FormField::text)
            .map(str::trim)
            .unwrap_or_default();
        return FormOutput::Value(text.to_string());
    }
    let mut key = None;
    let mut item = Item::new();
    for field in fields {
        match field.role {
            FormRole::EntryKey => {
                key = Some(field.text().map(str::trim).unwrap_or_default().to_string());
            }
            FormRole::Value => {
                if let Some(value) = field.stored() {
                    item.insert(field.key.clone(), value);
                }
            }
        }
    }
    FormOutput::Entry(Entry { key, fields: item })
}

pub(super) fn count_label(count: usize, noun: &str) -> String {
    match count {
        0 => "Empty".to_string(),
        1 => format!("1 {noun}"),
        count => format!("{count} {noun}s"),
    }
}

pub(super) fn with_article(noun: &str) -> String {
    let first = noun.chars().next().unwrap_or(' ');
    let article = if "aeiou".contains(first.to_ascii_lowercase()) {
        "an"
    } else {
        "a"
    };
    format!("{article} {noun}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_edit::Span;

    fn switchable_state() -> ObjectArrayState {
        ObjectArrayState::list(
            vec![
                ("app".to_string(), ItemFieldKind::Text),
                ("switchable".to_string(), ItemFieldKind::Boolean),
            ],
            Vec::new(),
        )
    }

    fn scroll_state() -> ObjectArrayState {
        ObjectArrayState::list(
            vec![
                ("from_mods".to_string(), ItemFieldKind::Mods),
                ("to_mods".to_string(), ItemFieldKind::Mods),
                ("global".to_string(), ItemFieldKind::Boolean),
            ],
            vec![Item::from_iter([(
                "global".to_string(),
                FieldDefault::Boolean(true),
            )])],
        )
    }

    fn ide_state() -> ObjectArrayState {
        ObjectArrayState::map(
            "App ID".to_string(),
            vec![
                ("name".to_string(), ItemFieldKind::Text),
                ("paths".to_string(), ItemFieldKind::StringArray),
            ],
            vec![(
                "zed".to_string(),
                Item::from_iter([(
                    "paths".to_string(),
                    FieldDefault::StringArray(vec![
                        "/opt/homebrew/bin/zed".to_string(),
                        "/usr/bin/zed".to_string(),
                        "~/.local/bin/zed".to_string(),
                    ]),
                )]),
            )],
        )
    }

    #[test]
    fn a_new_panel_form_only_changes_against_its_opening_output() {
        let state = switchable_state();
        let mut form = EntryForm::for_entry(&state, None, "switchable panel");

        assert!(!form.changed());
        assert_eq!(form.crumb, "add");
        assert_eq!(form.sub_header(), "a new switchable panel.");

        form.text_mut(0).unwrap().insert_str("firefox");
        assert!(form.changed());

        for _ in 0..7 {
            form.text_mut(0).unwrap().backspace(Span::Char);
        }
        assert!(!form.changed());

        assert!(form.flip(1));
        assert_eq!(form.ask(), FormQuestion::Blocked(0));
        assert_eq!(
            form.question_text(FormQuestion::Blocked(0)),
            "App is empty, so this switchable panel cannot be saved yet."
        );

        form.text_mut(0).unwrap().insert_str("firefox");
        assert_eq!(form.ask(), FormQuestion::Save);
        assert_eq!(
            form.question_text(FormQuestion::Save),
            "Save the new switchable panel?"
        );
    }

    #[test]
    fn a_keyed_entry_form_round_trips_its_entry() {
        let state = ide_state();
        let mut form = EntryForm::for_entry(&state, Some(0), "application");

        let labels: Vec<&str> = form
            .fields
            .iter()
            .map(|field| field.label.as_str())
            .collect();
        assert_eq!(labels, vec!["App ID", "Name", "Paths"]);
        assert_eq!(form.crumb, "zed");
        assert_eq!(form.sub_header(), "an application.");
        assert_eq!(form.output(), FormOutput::Entry(state.entries[0].clone()));
        assert_eq!(
            form.question_text(FormQuestion::Save),
            "Save changes to zed?"
        );

        assert!(form.set_list(2, vec!["/opt/zed".to_string()]));
        assert!(form.changed());

        form.text_mut(0).unwrap().clear();
        assert_eq!(form.ask(), FormQuestion::Blocked(0));
    }

    #[test]
    fn a_scroll_rule_form_with_only_global_on_is_blocked() {
        let state = scroll_state();
        let form = EntryForm::for_entry(&state, Some(0), "scroll rule");

        assert_eq!(form.name(), None);
        assert_eq!(form.ask(), FormQuestion::Blocked(0));
        assert_eq!(
            form.question_text(FormQuestion::Blocked(0)),
            "From Mods is empty, so this scroll rule cannot be saved yet."
        );
    }

    #[test]
    fn a_value_form_edits_one_trimmed_string() {
        let values = vec!["/usr/bin/zed".to_string()];
        let mut form = EntryForm::for_value(&values, Some(0), "path");

        assert_eq!(form.crumb, "edit");
        assert_eq!(form.fields[0].label, "Path");
        assert_eq!(form.output(), FormOutput::Value("/usr/bin/zed".to_string()));

        form.text_mut(0).unwrap().set_text(" ~/.local/bin/zed ");
        assert_eq!(
            form.output(),
            FormOutput::Value("~/.local/bin/zed".to_string())
        );

        form.text_mut(0).unwrap().clear();
        assert_eq!(form.ask(), FormQuestion::Blocked(0));

        let add = EntryForm::for_value(&values, None, "path");
        assert_eq!(add.crumb, "add");
        assert_eq!(add.output(), FormOutput::Value(String::new()));
    }

    #[test]
    fn a_stored_modifier_outside_the_known_set_stays_editable() {
        let state = ObjectArrayState::list(
            vec![("from_mods".to_string(), ItemFieldKind::Mods)],
            vec![Item::from_iter([(
                "from_mods".to_string(),
                FieldDefault::StringArray(vec!["hyper".to_string()]),
            )])],
        );
        let form = EntryForm::for_entry(&state, Some(0), "key rule");

        let FormValue::Mods {
            options, selected, ..
        } = &form.fields[0].value
        else {
            panic!("modifier field");
        };
        assert_eq!(options.last(), Some(&"hyper".to_string()));
        assert_eq!(chosen_options(options, selected), vec!["hyper".to_string()]);
    }

    #[test]
    fn modifier_fields_step_and_toggle_within_their_options() {
        let state = scroll_state();
        let mut form = EntryForm::for_entry(&state, Some(0), "scroll rule");

        assert!(form.step_mods(0, -1));
        assert!(form.toggle_mod(0, None));
        assert!(form.step_mods(0, 1));
        assert!(form.toggle_mod(0, Some(2)));

        let FormValue::Mods {
            options, selected, ..
        } = &form.fields[0].value
        else {
            panic!("modifier field");
        };
        assert_eq!(
            chosen_options(options, selected),
            vec!["ctrl".to_string(), "alt".to_string()]
        );

        for _ in 0..20 {
            form.step_mods(0, 1);
        }
        let FormValue::Mods {
            options, cursor, ..
        } = &form.fields[0].value
        else {
            panic!("modifier field");
        };
        assert_eq!(*cursor, options.len() - 1);

        assert!(!form.toggle_mod(0, Some(99)));
        assert!(!form.flip(0));
        assert!(!form.set_list(0, Vec::new()));
        assert!(!form.step_mods(2, 1));
        assert!(!form.toggle_mod(3, None));
    }

    #[test]
    fn labels_count_items_and_choose_their_article() {
        assert_eq!(count_label(0, "path"), "Empty");
        assert_eq!(count_label(1, "path"), "1 path");
        assert_eq!(count_label(3, "path"), "3 paths");
        assert_eq!(with_article("application"), "an application");
        assert_eq!(with_article("key rule"), "a key rule");
        assert_eq!(with_article("Item"), "an Item");
    }
}
