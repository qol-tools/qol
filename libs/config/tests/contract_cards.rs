use qol_config::contract::{parse_spec_str, ConfigSpec, FieldKind, FieldSpec};
use qol_config::validation::validate_spec;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("../..")
}

fn plugin_directories(root: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let plugins = root.join("plugins");
    let entries = std::fs::read_dir(plugins).expect("read plugins directory");
    for entry in entries {
        directories.push(entry.expect("plugin entry").path());
    }
    directories.sort();
    directories
}

fn config_files() -> Vec<PathBuf> {
    let root = repository_root();
    let core = root.join("apps/qol-tray");
    let mut files = vec![core.join("src/settings_surface/platform/core-config.toml")];
    for directory in plugin_directories(&root) {
        let config = directory.join("qol-config.toml");
        if config.is_file() {
            files.push(config);
        }
    }
    files
}

fn configs() -> Vec<(PathBuf, ConfigSpec)> {
    let mut loaded = Vec::new();
    for path in config_files() {
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let spec = parse_spec_str(&content)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        loaded.push((path, spec));
    }
    loaded
}

fn opens_card(field: &FieldSpec) -> bool {
    let card_kinds = [
        FieldKind::Select,
        FieldKind::ObjectArray,
        FieldKind::ObjectMap,
        FieldKind::List,
        FieldKind::DisplayLayout,
        FieldKind::Gamepad,
        FieldKind::QrCode,
    ];
    if card_kinds.contains(&field.kind) {
        return true;
    }
    needs_item_label(field)
}

fn needs_item_label(field: &FieldSpec) -> bool {
    if field.kind == FieldKind::ObjectArray {
        return true;
    }
    if field.kind == FieldKind::ObjectMap {
        return true;
    }
    field.kind == FieldKind::StringArray && field.options.is_empty() && field.query.is_none()
}

fn nested_string_keys(field: &FieldSpec) -> Vec<&str> {
    let mut keys = Vec::new();
    if let Some(item) = field.item.as_ref() {
        for (key, kind) in &item.fields {
            if *kind == FieldKind::StringArray && !key.ends_with("_mods") {
                keys.push(key.as_str());
            }
        }
    }
    for (key, kind) in &field.entry_fields {
        if *kind == FieldKind::StringArray && !key.ends_with("_mods") {
            keys.push(key.as_str());
        }
    }
    keys
}

#[test]
fn every_shipped_contract_parses_and_validates() {
    for (path, spec) in configs() {
        if let Err(errors) = validate_spec(&spec) {
            panic!("{} does not validate: {errors:?}", path.display());
        }
    }
}

#[test]
fn card_opening_fields_carry_a_card_description() {
    for (path, spec) in configs() {
        for (id, field) in &spec.fields {
            if !opens_card(field) {
                continue;
            }
            assert!(
                field.card_description.is_some(),
                "{}: field.{id} opens a card without card_description",
                path.display()
            );
        }
    }
}

#[test]
fn card_opening_fields_carry_an_item_label() {
    for (path, spec) in configs() {
        for (id, field) in &spec.fields {
            if !needs_item_label(field) {
                continue;
            }
            assert!(
                field.item_label.is_some(),
                "{}: field.{id} opens a card without item_label",
                path.display()
            );
        }
    }
}

#[test]
fn selects_declare_a_picture_for_every_option() {
    for (path, spec) in configs() {
        for (id, field) in &spec.fields {
            if field.kind != FieldKind::Select {
                continue;
            }
            for option in field.options.iter() {
                assert!(
                    field.option_pictures.contains_key(option),
                    "{}: field.{id} option {option} has no picture",
                    path.display()
                );
            }
            for option in field.option_labels.keys() {
                assert!(
                    field.option_pictures.contains_key(option),
                    "{}: field.{id} option {option} has no picture",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn lookup_selects_carry_a_lookup_label() {
    for (path, spec) in configs() {
        for (id, field) in &spec.fields {
            if field.kind != FieldKind::Select || field.query.is_none() {
                continue;
            }
            assert!(
                field.lookup_label.is_some(),
                "{}: field.{id} looks up options without lookup_label",
                path.display()
            );
        }
    }
}

#[test]
fn nested_string_lists_carry_both_copy_keys() {
    for (path, spec) in configs() {
        for (id, field) in &spec.fields {
            for key in nested_string_keys(field) {
                let list = match field.lists.get(key) {
                    Some(list) => list,
                    None => panic!("{}: field.{id} lists.{key} is missing", path.display()),
                };
                assert!(
                    list.item_label.is_some(),
                    "{}: field.{id} lists.{key} has no item_label",
                    path.display()
                );
                assert!(
                    list.card_description.is_some(),
                    "{}: field.{id} lists.{key} has no card_description",
                    path.display()
                );
            }
        }
    }
}
