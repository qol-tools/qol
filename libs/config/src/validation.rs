use crate::contract::{
    is_picture_spec, ConfigSpec, FieldDefault, FieldKind, FieldSpec, CARD_DESCRIPTION_MAX,
};
use crate::platform::validate_platforms;
use std::fmt;

const ITEM_LABEL_MAX: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

pub fn validate_spec(spec: &ConfigSpec) -> Result<(), Vec<ValidationError>> {
    let errors = validate_spec_collect(spec);
    if errors.is_empty() {
        return Ok(());
    }
    Err(errors)
}

pub fn validate_spec_collect(spec: &ConfigSpec) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_schema_version(spec, &mut errors);
    validate_sections(spec, &mut errors);
    validate_devices(spec, &mut errors);
    validate_fields(spec, &mut errors);
    errors
}

pub fn validate_field_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_field_value_type(path, field, value, &mut errors);
    validate_select_value(path, field, value, &mut errors);
    validate_string_array_value(path, field, value, &mut errors);
    validate_number_value(path, field, value, &mut errors);
    validate_placement_value(path, field, value, &mut errors);
    errors
}

fn validate_schema_version(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    if spec.schema_version == 1 {
        return;
    }
    errors.push(ValidationError::new(
        "schema_version",
        format!("unsupported version {}", spec.schema_version),
    ));
}

fn validate_sections(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    for (id, section) in &spec.sections {
        validate_platforms(
            &format!("section.{id}"),
            section.platforms.as_deref(),
            errors,
        );
        if valid_id(id) {
            validate_section_actions(id, &section.actions, errors);
            continue;
        }
        errors.push(ValidationError::new(format!("section.{id}"), "invalid id"));
        validate_section_actions(id, &section.actions, errors);
    }
}

fn validate_devices(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    let kinds = spec.devices.as_ref().map(|devices| &devices.kinds);
    if let Some(devices) = spec.devices.as_ref() {
        if !valid_id(&devices.query) {
            errors.push(ValidationError::new("devices.query", "invalid id"));
        }
        if devices.kinds.is_empty() {
            errors.push(ValidationError::new(
                "devices.kinds",
                "must name at least one kind",
            ));
        }
        for (kind, label) in &devices.kinds {
            if !valid_id(kind) {
                errors.push(ValidationError::new(
                    format!("devices.kinds.{kind}"),
                    "invalid id",
                ));
            }
            if label.trim().is_empty() {
                errors.push(ValidationError::new(
                    format!("devices.kinds.{kind}"),
                    "label must not be empty",
                ));
            }
        }
    }
    for (id, section) in &spec.sections {
        let Some(kind) = section.device_kind.as_deref() else {
            continue;
        };
        let path = format!("section.{id}.device_kind");
        match kinds {
            None => errors.push(ValidationError::new(path, "requires a [devices] table")),
            Some(kinds) if !kinds.contains_key(kind) => errors.push(ValidationError::new(
                path,
                format!("unknown device kind {kind}"),
            )),
            Some(_) => {}
        }
    }
}

fn validate_section_actions(id: &str, actions: &[String], errors: &mut Vec<ValidationError>) {
    let mut seen = std::collections::HashSet::new();
    for (index, action) in actions.iter().enumerate() {
        if !valid_id(action) {
            errors.push(ValidationError::new(
                format!("section.{id}.actions[{index}]"),
                "invalid action id",
            ));
            continue;
        }
        if seen.insert(action) {
            continue;
        }
        errors.push(ValidationError::new(
            format!("section.{id}.actions[{index}]"),
            "duplicate action id",
        ));
    }
}

fn validate_fields(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    for (id, field) in &spec.fields {
        validate_field_id(id, errors);
        validate_field_config_key(id, field, errors);
        validate_field_section(id, field.section.as_deref(), spec, errors);
        validate_field_default(id, field, errors);
        validate_field_options(id, field, errors);
        validate_field_copy(id, field, errors);
        validate_entries_card(id, field, errors);
        validate_option_pictures(id, field, errors);
        validate_lookup_label(id, field, errors);
        validate_lists(id, field, errors);
        validate_entry_fields(id, field, errors);
        validate_number_constraints(id, field, errors);
        validate_row_slider(id, field, errors);
        validate_level_slider(id, field, errors);
        validate_show_when(id, field, spec, errors);
        validate_platforms(&format!("field.{id}"), field.platforms.as_deref(), errors);
    }
    validate_config_key_collisions(spec, errors);
}

fn validate_field_id(id: &str, errors: &mut Vec<ValidationError>) {
    if valid_id(id) {
        return;
    }
    errors.push(ValidationError::new(format!("field.{id}"), "invalid id"));
}

fn validate_field_config_key(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let Some(config_key) = field.config_key.as_deref() else {
        return;
    };
    if valid_config_key(config_key) {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.config_key"),
        "invalid config key",
    ));
}

fn validate_field_section(
    id: &str,
    section: Option<&str>,
    spec: &ConfigSpec,
    errors: &mut Vec<ValidationError>,
) {
    let section = match section {
        Some(section) => section,
        None => return,
    };
    if spec.sections.contains_key(section) {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.section"),
        format!("unknown section {section}"),
    ));
}

fn validate_config_key_collisions(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    let keys: Vec<(&str, String)> = spec
        .fields
        .iter()
        .filter_map(|(id, field)| {
            if field.has_stored_value() {
                Some((
                    id.as_str(),
                    field.config_key.clone().unwrap_or_else(|| id.clone()),
                ))
            } else {
                None
            }
        })
        .collect();

    for (left_index, (left_id, left_key)) in keys.iter().enumerate() {
        for (right_id, right_key) in keys.iter().skip(left_index + 1) {
            if left_key == right_key {
                errors.push(ValidationError::new(
                    format!("field.{right_id}.config_key"),
                    format!("duplicates field.{left_id} config key {right_key}"),
                ));
                continue;
            }
            if config_key_prefix_collision(left_key, right_key) {
                errors.push(ValidationError::new(
                    format!("field.{right_id}.config_key"),
                    format!("collides with field.{left_id} config key {left_key}"),
                ));
            }
        }
    }
}

fn config_key_prefix_collision(left: &str, right: &str) -> bool {
    left.strip_prefix(right)
        .is_some_and(|suffix| suffix.starts_with('.'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn validate_field_default(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let default = match &field.default {
        Some(default) => default,
        None => {
            if field.requires_declared_default() {
                errors.push(ValidationError::new(
                    format!("field.{id}.default"),
                    "missing default",
                ));
            }
            return;
        }
    };
    errors.extend(validate_field_value(
        &format!("field.{id}.default"),
        field,
        default,
    ));
}

fn validate_field_options(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.kind == FieldKind::Select {
        validate_select_options(id, field, errors);
        validate_option_labels(id, field, errors);
        return;
    }
    if field.kind == FieldKind::StringArray && (!field.options.is_empty() || field.query.is_some())
    {
        validate_select_options(id, field, errors);
        validate_option_labels(id, field, errors);
        return;
    }
    if field.options.is_empty() && field.option_labels.is_empty() {
        return;
    }
    if !field.option_labels.is_empty() {
        errors.push(ValidationError::new(
            format!("field.{id}.option_labels"),
            "option_labels only supported for select and string_array fields",
        ));
    }
    if field.options.is_empty() {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.options"),
        "options only supported for select and string_array fields",
    ));
}

fn validate_entry_fields(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.kind == FieldKind::ObjectMap {
        validate_object_map_fields(id, field, errors);
        return;
    }
    if field.key_label.is_none() && field.entry_fields.is_empty() {
        return;
    }
    if field.key_label.is_some() {
        errors.push(ValidationError::new(
            format!("field.{id}.key_label"),
            "key_label only supported for object_map fields",
        ));
    }
    if field.entry_fields.is_empty() {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.entry_fields"),
        "entry_fields only supported for object_map fields",
    ));
}

fn validate_object_map_fields(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    for entry_id in field.entry_fields.keys() {
        if valid_id(entry_id) {
            continue;
        }
        errors.push(ValidationError::new(
            format!("field.{id}.entry_fields.{entry_id}"),
            "invalid entry field id",
        ));
    }
}

fn validate_select_options(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.options.is_empty() {
        if field.kind == FieldKind::Select && field.query.is_none() {
            errors.push(ValidationError::new(
                format!("field.{id}.options"),
                "select field requires options or a query",
            ));
        }
        return;
    }
    for (index, option) in field.options.iter().enumerate() {
        if !option.trim().is_empty() {
            continue;
        }
        errors.push(ValidationError::new(
            format!("field.{id}.options[{index}]"),
            "option cannot be empty",
        ));
    }
}

fn validate_option_labels(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    for (option, label) in &field.option_labels {
        let known =
            field.query.is_some() || field.options.iter().any(|candidate| candidate == option);
        if known {
            if !label.trim().is_empty() {
                continue;
            }
            errors.push(ValidationError::new(
                format!("field.{id}.option_labels.{option}"),
                "option label cannot be empty",
            ));
            continue;
        }
        errors.push(ValidationError::new(
            format!("field.{id}.option_labels.{option}"),
            "option label must reference an existing option",
        ));
    }
}

fn validate_field_copy(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if let Some(description) = field.card_description.as_deref() {
        let path = format!("field.{id}.card_description");
        validate_card_description(&path, description, errors);
    }
    let Some(item_label) = field.item_label.as_deref() else {
        return;
    };
    let list_kinds = [
        FieldKind::ObjectArray,
        FieldKind::ObjectMap,
        FieldKind::StringArray,
    ];
    if !list_kinds.contains(&field.kind) {
        errors.push(ValidationError::new(
            format!("field.{id}.item_label"),
            "item_label only supported for object_array, object_map and string_array fields",
        ));
        return;
    }
    let path = format!("field.{id}.item_label");
    validate_item_label(&path, item_label, errors);
}

fn validate_entries_card(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let entries_card = field.kind == FieldKind::ObjectArray
        || (field.kind == FieldKind::StringArray
            && field.options.is_empty()
            && field.query.is_none());
    for (key, set) in [
        ("reorder", field.reorder.is_some()),
        ("add_label", field.add_label.is_some()),
    ] {
        if set && !entries_card {
            errors.push(ValidationError::new(
                format!("field.{id}.{key}"),
                format!(
                    "{key} only supported for object_array fields and string_array fields without options or query"
                ),
            ));
        }
    }
    if let Some(label) = field.add_label.as_deref().filter(|_| entries_card) {
        validate_item_label(&format!("field.{id}.add_label"), label, errors);
    }
    if field.kind != FieldKind::ObjectArray {
        return;
    }
    let Some(template) = field.row_label.as_deref() else {
        return;
    };
    let names = template_names(template);
    let known = |name: &&str| {
        field
            .item
            .as_ref()
            .is_some_and(|item| item.fields.contains_key(*name))
    };
    if names.is_empty() || !names.iter().all(known) {
        errors.push(ValidationError::new(
            format!("field.{id}.row_label"),
            "row_label must name item fields in braces, like {name}",
        ));
    }
}

fn template_names(template: &str) -> Vec<&str> {
    template
        .split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

fn validate_card_description(path: &str, description: &str, errors: &mut Vec<ValidationError>) {
    if description.trim().is_empty() {
        errors.push(ValidationError::new(
            path,
            "card description cannot be empty",
        ));
        return;
    }
    if description.chars().count() <= CARD_DESCRIPTION_MAX {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("card description cannot exceed {CARD_DESCRIPTION_MAX} characters"),
    ));
}

fn validate_item_label(path: &str, label: &str, errors: &mut Vec<ValidationError>) {
    if label.trim().is_empty() {
        errors.push(ValidationError::new(path, "item label cannot be empty"));
        return;
    }
    if label.chars().count() <= ITEM_LABEL_MAX {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("item label cannot exceed {ITEM_LABEL_MAX} characters"),
    ));
}

fn validate_lookup_label(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let Some(label) = field.lookup_label.as_deref() else {
        return;
    };
    let path = format!("field.{id}.lookup_label");
    if field.query.is_none() {
        errors.push(ValidationError::new(
            path.as_str(),
            "lookup_label only supported for fields with a query",
        ));
        return;
    }
    if label.trim().is_empty() {
        errors.push(ValidationError::new(
            path.as_str(),
            "lookup label cannot be empty",
        ));
        return;
    }
    if label.chars().count() <= ITEM_LABEL_MAX {
        return;
    }
    errors.push(ValidationError::new(
        path.as_str(),
        format!("lookup label cannot exceed {ITEM_LABEL_MAX} characters"),
    ));
}

fn validate_option_pictures(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    for (option, picture) in &field.option_pictures {
        let path = format!("field.{id}.option_pictures.{option}");
        let known = field.options.iter().any(|key| key == option)
            || field.option_labels.contains_key(option);
        if !known {
            errors.push(ValidationError::new(
                path,
                "option picture must reference an existing option",
            ));
            continue;
        }
        if is_picture_spec(picture) {
            continue;
        }
        errors.push(ValidationError::new(path, "invalid picture spec"));
    }
}

fn validate_lists(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    for (key, list) in &field.lists {
        let path = format!("field.{id}.lists.{key}");
        let item_kind = field.item.as_ref().and_then(|item| item.fields.get(key));
        let item_kind = item_kind.or_else(|| field.entry_fields.get(key));
        let valid = item_kind == Some(&FieldKind::StringArray);
        if !valid || key.ends_with("_mods") {
            errors.push(ValidationError::new(
                path.clone(),
                "lists key must name a string_array item or entry field",
            ));
        }
        if let Some(item_label) = list.item_label.as_deref() {
            let item_path = format!("{path}.item_label");
            validate_item_label(&item_path, item_label, errors);
        }
        if let Some(description) = list.card_description.as_deref() {
            let card_path = format!("{path}.card_description");
            validate_card_description(&card_path, description, errors);
        }
    }
}

fn validate_number_constraints(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.kind != FieldKind::Number {
        validate_non_number_constraints(id, field, errors);
        return;
    }
    if field.number.lowest() > field.number.highest() {
        errors.push(ValidationError::new(
            format!("field.{id}"),
            "min cannot be greater than max",
        ));
    }
    validate_number_step(id, field.number.step, errors);
}

fn validate_non_number_constraints(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.number.min.is_none() && field.number.max.is_none() && field.number.step.is_none() {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}"),
        "min, max, and step are only supported for number fields",
    ));
}

fn validate_number_step(id: &str, step: Option<f64>, errors: &mut Vec<ValidationError>) {
    let step = match step {
        Some(step) => step,
        None => return,
    };
    if step > 0.0 {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.step"),
        "step must be greater than 0",
    ));
}

fn validate_row_slider(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let Some(slider) = field.row_slider.as_ref() else {
        return;
    };
    if field.kind != FieldKind::List && field.kind != FieldKind::DisplayLayout {
        errors.push(ValidationError::new(
            format!("field.{id}.row_slider"),
            "row_slider is only supported for list and display_layout fields",
        ));
        return;
    }
    if slider.min >= slider.max {
        errors.push(ValidationError::new(
            format!("field.{id}.row_slider"),
            "min cannot be greater than or equal to max",
        ));
    }
    if slider.step <= 0.0 {
        errors.push(ValidationError::new(
            format!("field.{id}.row_slider.step"),
            "step must be greater than 0",
        ));
    }
}

const LEVEL_SLIDER: &str = "level_slider";

fn validate_level_slider(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.variant.as_deref() != Some(LEVEL_SLIDER) {
        for (key, value) in [
            ("level_query", &field.level_query),
            ("level_value_from", &field.level_value_from),
        ] {
            if value.is_some() {
                errors.push(ValidationError::new(
                    format!("field.{id}.{key}"),
                    format!("{key} is only valid for the level_slider variant"),
                ));
            }
        }
        return;
    }
    if field.kind != FieldKind::Number {
        errors.push(ValidationError::new(
            format!("field.{id}.variant"),
            "level_slider is only supported for number fields",
        ));
        return;
    }
    if field.level_query.as_deref().is_none_or(str::is_empty) {
        errors.push(ValidationError::new(
            format!("field.{id}.level_query"),
            "is required for the level_slider variant",
        ));
    }
    if field.level_value_from.as_deref().is_none_or(str::is_empty) {
        errors.push(ValidationError::new(
            format!("field.{id}.level_value_from"),
            "is required for the level_slider variant",
        ));
    }
}

fn validate_show_when(
    id: &str,
    field: &FieldSpec,
    spec: &ConfigSpec,
    errors: &mut Vec<ValidationError>,
) {
    let show_when = match &field.show_when {
        Some(show_when) => show_when,
        None => return,
    };
    let referenced = match spec.fields.get(show_when.field.as_str()) {
        Some(referenced) => referenced,
        None => {
            errors.push(ValidationError::new(
                format!("field.{id}.show_when.field"),
                format!("unknown field {}", show_when.field),
            ));
            return;
        }
    };
    if default_matches_kind(&show_when.equals, referenced.kind) {
        validate_select_value(
            &format!("field.{id}.show_when.equals"),
            referenced,
            &show_when.equals,
            errors,
        );
        validate_number_value(
            &format!("field.{id}.show_when.equals"),
            referenced,
            &show_when.equals,
            errors,
        );
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.show_when.equals"),
        format!("value does not match field type {}", referenced.kind.name()),
    ));
}

fn validate_field_value_type(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if value_matches_kind_after_widening(value, field.kind) {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value does not match field type {}", field.kind.name()),
    ));
}

fn value_matches_kind_after_widening(value: &FieldDefault, kind: FieldKind) -> bool {
    default_matches_kind(&crate::normalized::widen_to_kind(value.clone(), kind), kind)
}

fn validate_select_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if field.kind != FieldKind::Select {
        return;
    }
    if field.options.is_empty() || field.query.is_some() {
        return;
    }
    let selected = match value {
        FieldDefault::String(selected) => selected,
        _ => return,
    };
    if field.options.iter().any(|option| option == selected) {
        return;
    }
    errors.push(ValidationError::new(
        path,
        "value must match one of the select options",
    ));
}

fn validate_string_array_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if field.kind != FieldKind::StringArray {
        return;
    }
    if field.options.is_empty() || field.query.is_some() {
        return;
    }
    let values = match value {
        FieldDefault::StringArray(values) => values,
        _ => return,
    };
    for (index, entry) in values.iter().enumerate() {
        if field.options.iter().any(|option| option == entry) {
            continue;
        }
        errors.push(ValidationError::new(
            format!("{path}[{index}]"),
            "value must match one of the field options",
        ));
    }
}

fn validate_placement_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if field.kind != FieldKind::Placement {
        return;
    }
    let FieldDefault::String(text) = value else {
        return;
    };
    if crate::placement::Spot::parse(text).is_some() {
        return;
    }
    errors.push(ValidationError::new(
        path,
        "value must name a spot such as top-right, or give its x,y fractions",
    ));
}

fn validate_number_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if field.kind != FieldKind::Number {
        return;
    }
    let number = match value {
        FieldDefault::Number(number) => *number,
        _ => return,
    };
    validate_min(path, field.number.lowest(), number, errors);
    validate_max(path, field.number.highest(), number, errors);
    validate_step_value(path, field, number, errors);
}

fn validate_min(path: &str, min: f64, value: f64, errors: &mut Vec<ValidationError>) {
    if value >= min {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value must be at least {min}"),
    ));
}

fn validate_max(path: &str, max: f64, value: f64, errors: &mut Vec<ValidationError>) {
    if value <= max {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value must be at most {max}"),
    ));
}

fn validate_step_value(
    path: &str,
    field: &FieldSpec,
    value: f64,
    errors: &mut Vec<ValidationError>,
) {
    let step = match field.number.step {
        Some(step) => step,
        None => return,
    };
    let origin = field.number.lowest();
    let steps = (value - origin) / step;
    let rounded = steps.round();
    if (steps - rounded).abs() <= 1e-9 {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value must align to step {step}"),
    ));
}

fn default_matches_kind(default: &FieldDefault, kind: FieldKind) -> bool {
    match kind {
        FieldKind::Boolean => matches!(default, FieldDefault::Boolean(_)),
        FieldKind::String | FieldKind::Select | FieldKind::Color | FieldKind::Placement => {
            matches!(default, FieldDefault::String(_))
        }
        FieldKind::Number => matches!(default, FieldDefault::Number(_)),
        FieldKind::StringArray => matches!(default, FieldDefault::StringArray(_)),
        FieldKind::ObjectArray => match default {
            FieldDefault::ObjectArray(_) => true,
            FieldDefault::StringArray(values) => values.is_empty(),
            FieldDefault::Boolean(_)
            | FieldDefault::String(_)
            | FieldDefault::Number(_)
            | FieldDefault::ObjectMap(_) => false,
        },
        FieldKind::ObjectMap => matches!(default, FieldDefault::ObjectMap(_)),
        FieldKind::Action
        | FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad
        | FieldKind::DisplayLayout => false,
    }
}

fn valid_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    value
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-')
}

fn valid_config_key(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    value.split('.').all(valid_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::parse_spec_str;

    #[test]
    fn row_slider_requires_list_or_display_layout_kind_and_a_valid_range() {
        let valid = validate_contract(
            r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"

[field.volumes.row_slider]
value_from = "volume"
action = "set_volume"
"#,
        );
        assert!(valid.is_empty(), "{valid:?}");

        let inverted = validate_contract(
            r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"

[field.volumes.row_slider]
value_from = "volume"
min = 100
max = 0
action = "set_volume"
"#,
        );
        assert_has_error(&inverted, "field.volumes.row_slider", "min");

        let equal = validate_contract(
            r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"

[field.volumes.row_slider]
value_from = "volume"
min = 50
max = 50
action = "set_volume"
"#,
        );
        assert_has_error(&equal, "field.volumes.row_slider", "min");

        let bad_step = validate_contract(
            r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"

[field.volumes.row_slider]
value_from = "volume"
step = 0
action = "set_volume"
"#,
        );
        assert_has_error(&bad_step, "field.volumes.row_slider.step", "step");

        let display_layout = validate_contract(
            r#"
schema_version = 1

[field.arrangement]
type = "display_layout"
query = "layout"

[field.arrangement.row_slider]
value_from = "brightness"
min = 0
max = 100
step = 5
action = "set_brightness"
"#,
        );
        assert!(display_layout.is_empty(), "{display_layout:?}");

        let non_list = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
default = 1

[field.volume.row_slider]
value_from = "volume"
action = "set_volume"
"#,
        );
        assert_has_error(
            &non_list,
            "field.volume.row_slider",
            "only supported for list and display_layout fields",
        );
    }

    #[test]
    fn config_key_prefix_collision_is_symmetric_and_segment_aware() {
        let cases = [
            ("display", "display.scale", true),
            ("display.scale", "display", true),
            ("display.scale", "display.scale.inner", true),
            ("display", "displayed", false),
            ("display.scale", "display.scaled", false),
            ("audio.inputs", "video.inputs", false),
        ];

        for (left, right, expected) in cases {
            assert_eq!(
                config_key_prefix_collision(left, right),
                expected,
                "{left} vs {right}"
            );
        }
    }

    #[test]
    fn validates_exact_duplicate_config_keys() {
        let errors = validate_contract(
            r##"
schema_version = 1

[field.first]
type = "string"
config_key = "display.color"
default = "red"

[field.second]
type = "color"
config_key = "display.color"
default = "#ffffff"
"##,
        );

        assert_has_error(&errors, "field.second.config_key", "duplicates field.first");
    }

    #[test]
    fn validates_prefix_config_key_collisions_in_both_orders() {
        for contract in [
            r#"
schema_version = 1

[field.parent]
type = "string"
config_key = "display"
default = "red"

[field.child]
type = "number"
config_key = "display.scale"
default = 1
"#,
            r#"
schema_version = 1

[field.child]
type = "number"
config_key = "display.scale"
default = 1

[field.parent]
type = "string"
config_key = "display"
default = "red"
"#,
        ] {
            let errors = validate_contract(contract);
            assert_has_collision_between(&errors, "parent", "child");
        }
    }

    #[test]
    fn allows_config_keys_that_only_share_text_prefixes() {
        let errors = validate_contract(
            r#"
schema_version = 1

[field.display]
type = "string"
config_key = "display_color"
default = "red"

[field.displayed]
type = "number"
config_key = "displayed.scale"
default = 1

[field.scale]
type = "number"
config_key = "display.scale_factor"
default = 2
"#,
        );

        assert!(
            errors
                .iter()
                .all(|error| !error.message.contains("collides with")),
            "{errors:?}"
        );
    }

    #[test]
    fn select_with_query_allows_dynamic_options() {
        let errors = validate_contract(
            r#"
schema_version = 1

[field.device]
type = "select"
config_key = "audio.device"
default = "default"
query = "audio_sources"
"#,
        );
        assert!(errors.is_empty(), "{errors:?}");

        let static_without_options = validate_contract(
            r#"
schema_version = 1

[field.device]
type = "select"
config_key = "audio.device"
default = "default"
"#,
        );
        assert_has_error(
            &static_without_options,
            "field.device.options",
            "select field requires options or a query",
        );
    }

    #[test]
    fn string_array_options_constrain_defaults_and_labels() {
        let valid = validate_contract(
            r#"
schema_version = 1

[field.inputs]
type = "string_array"
config_key = "audio.inputs"
default = ["mic"]
options = ["mic", "system"]

[field.inputs.option_labels]
mic = "Microphone"
system = "System Audio"
"#,
        );
        assert!(valid.is_empty(), "{valid:?}");

        let unknown_default = validate_contract(
            r#"
schema_version = 1

[field.inputs]
type = "string_array"
config_key = "audio.inputs"
default = ["mic", "webcam"]
options = ["mic", "system"]
"#,
        );
        assert_has_error(
            &unknown_default,
            "field.inputs.default[1]",
            "value must match one of the field options",
        );

        let dynamic = validate_contract(
            r#"
schema_version = 1

[field.inputs]
type = "string_array"
config_key = "audio.inputs"
default = ["gone-device"]
query = "audio_sources"

[field.inputs.option_labels]
default = "System Default"
"#,
        );
        assert!(dynamic.is_empty(), "{dynamic:?}");

        let options_elsewhere = validate_contract(
            r#"
schema_version = 1

[field.name]
type = "string"
default = "foo"
options = ["foo", "bar"]
"#,
        );
        assert_has_error(
            &options_elsewhere,
            "field.name.options",
            "options only supported for select and string_array fields",
        );
    }

    #[test]
    fn string_array_without_options_is_a_free_form_string_list() {
        let free_form = validate_contract(
            r#"
schema_version = 1

[field.service_commands]
type = "string_array"
config_key = "service_commands"
default = ["cargo watch", "tail -f"]
"#,
        );
        assert!(free_form.is_empty(), "{free_form:?}");

        let empty = validate_contract(
            r#"
schema_version = 1

[field.service_commands]
type = "string_array"
config_key = "service_commands"
default = []
"#,
        );
        assert!(empty.is_empty(), "{empty:?}");

        let scalar = validate_contract(
            r#"
schema_version = 1

[field.service_commands]
type = "string_array"
config_key = "service_commands"
default = "cargo watch"
"#,
        );
        assert!(
            scalar.is_empty(),
            "a scalar default widens to a single-item list, {scalar:?}"
        );

        let wrong_type = validate_contract(
            r#"
schema_version = 1

[field.service_commands]
type = "string_array"
config_key = "service_commands"
default = 5
"#,
        );
        assert_has_error(
            &wrong_type,
            "field.service_commands.default",
            "value does not match field type string_array",
        );
    }

    #[test]
    fn live_number_requires_a_numeric_default() {
        let missing = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
min = 0
max = 100
action = "set_volume"
active_query = "volume"
active_value_from = "volume"
"#,
        );
        assert_has_error(&missing, "field.volume.default", "missing default");

        let non_numeric = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
default = "loud"
min = 0
max = 100
action = "set_volume"
active_query = "volume"
active_value_from = "volume"
"#,
        );
        assert_has_error(
            &non_numeric,
            "field.volume.default",
            "value does not match field type number",
        );

        let sound = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
default = 0
min = 0
max = 100
step = 5
variant = "wide_slider"
action = "set_volume"
active_query = "volume"
active_value_from = "volume"
"#,
        );
        assert!(sound.is_empty(), "{sound:?}");
    }

    #[test]
    fn level_slider_requires_both_level_keys_on_a_number() {
        let valid = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
default = 0
min = 0
max = 100
variant = "level_slider"
action = "set_volume"
active_query = "volume"
active_value_from = "volume"
level_query = "levels"
level_value_from = "output"
"#,
        );
        assert!(valid.is_empty(), "{valid:?}");

        let missing = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
default = 0
variant = "level_slider"
"#,
        );
        assert_has_error(&missing, "field.volume.level_query", "is required");
        assert_has_error(&missing, "field.volume.level_value_from", "is required");

        let no_key = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "number"
default = 0
variant = "level_slider"
level_query = "levels"
"#,
        );
        assert_has_error(&no_key, "field.volume.level_value_from", "is required");

        let not_a_number = validate_contract(
            r#"
schema_version = 1

[field.volume]
type = "string"
default = ""
variant = "level_slider"
level_query = "levels"
level_value_from = "output"
"#,
        );
        assert_has_error(
            &not_a_number,
            "field.volume.variant",
            "only supported for number fields",
        );
    }

    #[test]
    fn level_keys_are_refused_on_every_other_variant() {
        for variant in ["", "variant = \"wide_slider\"", "variant = \"slider\""] {
            let errors = validate_contract(&format!(
                r#"
schema_version = 1

[field.volume]
type = "number"
default = 0
{variant}
level_query = "levels"
level_value_from = "output"
"#
            ));
            assert_has_error(&errors, "field.volume.level_query", "level_slider variant");
            assert_has_error(
                &errors,
                "field.volume.level_value_from",
                "level_slider variant",
            );
        }
    }

    #[test]
    fn platforms_must_name_known_operating_systems() {
        let errors = validate_contract(
            r#"
schema_version = 1

[section.host]
platforms = ["linux", "bsd"]

[field.frame]
type = "boolean"
default = true
platforms = ["Windows"]

[field.nowhere]
type = "boolean"
default = true
platforms = []
"#,
        );
        assert_has_error(&errors, "section.host.platforms[1]", "unknown platform bsd");
        assert_has_error(
            &errors,
            "field.frame.platforms[0]",
            "unknown platform Windows",
        );
        assert_has_error(
            &errors,
            "field.nowhere.platforms",
            "must name at least one platform",
        );
        assert_eq!(errors.len(), 3, "{errors:?}");

        let valid = validate_contract(
            r#"
schema_version = 1

[section.host]
platforms = ["linux"]

[field.frame]
type = "boolean"
section = "host"
default = true
platforms = ["linux", "macos", "windows"]
"#,
        );
        assert!(valid.is_empty(), "{valid:?}");
    }

    #[test]
    fn device_kinds_must_be_declared_in_the_devices_table() {
        let errors = validate_contract(
            r#"
schema_version = 1

[devices]
query = "devices"
kinds = { mouse = "Mouse", keyboard = "" }

[section.pointer]
device_kind = "mouse"

[section.trackpad]
device_kind = "touchpad"
"#,
        );
        assert_has_error(&errors, "devices.kinds.keyboard", "label must not be empty");
        assert_has_error(
            &errors,
            "section.trackpad.device_kind",
            "unknown device kind touchpad",
        );
        assert_eq!(errors.len(), 2, "{errors:?}");

        let orphan = validate_contract(
            r#"
schema_version = 1

[section.pointer]
device_kind = "mouse"
"#,
        );
        assert_has_error(
            &orphan,
            "section.pointer.device_kind",
            "requires a [devices]",
        );

        let empty = validate_contract(
            r#"
schema_version = 1

[devices]
query = "devices"
kinds = {}
"#,
        );
        assert_has_error(&empty, "devices.kinds", "at least one kind");
    }

    #[test]
    fn reorder_add_label_and_entry_row_labels_belong_to_entries_cards() {
        let valid = validate_contract(
            r#"
schema_version = 1

[field.pins]
type = "object_array"
card_description = "The folders Explorer lists first."
item_label = "folder"
add_label = "Pin a folder"
row_label = "{name}"
reorder = true
default = []

[field.pins.item.fields]
name = "string"
path = "string"

[field.paths]
type = "string_array"
reorder = true
default = []
"#,
        );
        assert!(valid.is_empty(), "{valid:?}");

        let cases = [
            (
                "type = \"select\"\noptions = [\"a\"]\ndefault = \"a\"\nreorder = true",
                "field.bad.reorder",
                "only supported",
            ),
            (
                "type = \"string_array\"\noptions = [\"a\"]\ndefault = []\nadd_label = \"Add one\"",
                "field.bad.add_label",
                "only supported",
            ),
            (
                "type = \"object_array\"\ndefault = []\nadd_label = \"\"\n[field.bad.item.fields]\nname = \"string\"",
                "field.bad.add_label",
                "cannot be empty",
            ),
            (
                "type = \"object_array\"\ndefault = []\nrow_label = \"{title}\"\n[field.bad.item.fields]\nname = \"string\"",
                "field.bad.row_label",
                "item fields",
            ),
            (
                "type = \"object_array\"\ndefault = []\nrow_label = \"plain\"\n[field.bad.item.fields]\nname = \"string\"",
                "field.bad.row_label",
                "item fields",
            ),
        ];
        for (field, path, message) in cases {
            let errors =
                validate_contract(&format!("schema_version = 1\n\n[field.bad]\n{field}\n"));
            assert_has_error(&errors, path, message);
        }
    }

    fn validate_contract(contract: &str) -> Vec<ValidationError> {
        let spec = parse_spec_str(contract).expect("contract parses");
        validate_spec_collect(&spec)
    }

    fn assert_has_error(errors: &[ValidationError], path: &str, message: &str) {
        assert!(
            errors
                .iter()
                .any(|error| error.path == path && error.message.contains(message)),
            "missing {path} / {message} in {errors:?}"
        );
    }

    fn assert_has_collision_between(errors: &[ValidationError], left: &str, right: &str) {
        let left_path = format!("field.{left}.config_key");
        let right_path = format!("field.{right}.config_key");
        let left_message = format!("field.{left}");
        let right_message = format!("field.{right}");
        assert!(
            errors.iter().any(|error| {
                error.message.contains("collides with")
                    && ((error.path == left_path && error.message.contains(&right_message))
                        || (error.path == right_path && error.message.contains(&left_message)))
            }),
            "missing collision between {left} and {right} in {errors:?}"
        );
    }
}
