use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use qol_config::contract::{parse_spec_str, ConfigSpec, FieldSpec};
use qol_platform::Os;

const NATIVE_PAGE_SETTINGS: &[(&str, &[&str])] = &[("Profiles", &["core:profile"])];

const DELIBERATE_SHORTCUTS: &[(&str, &str)] = &[];

struct ContractPage {
    page: String,
    scope: String,
    spec: ConfigSpec,
}

fn contract_pages() -> Vec<ContractPage> {
    let mut pages = vec![
        contract_page("General", "core", include_str!("core-config.toml")),
        contract_page(
            "Notifications",
            "notifications",
            include_str!("notifications-config.toml"),
        ),
    ];
    let plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut folders: Vec<_> = std::fs::read_dir(&plugins)
        .unwrap_or_else(|err| panic!("read {}: {err}", plugins.display()))
        .map(|entry| entry.expect("plugin folder entry").path())
        .filter(|folder| folder.join("qol-config.toml").is_file())
        .collect();
    folders.sort();
    for folder in folders {
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .expect("plugin folder name")
            .to_string();
        let contract = std::fs::read_to_string(folder.join("qol-config.toml"))
            .unwrap_or_else(|err| panic!("read the {name} contract: {err}"));
        pages.push(contract_page(&name, &name, &contract));
    }
    pages
}

fn contract_page(page: &str, scope: &str, contract: &str) -> ContractPage {
    ContractPage {
        page: page.to_string(),
        scope: scope.to_string(),
        spec: parse_spec_str(contract).unwrap_or_else(|err| panic!("{page} contract: {err}")),
    }
}

fn shows_on(platforms: &Option<Vec<String>>, os: Os) -> bool {
    platforms
        .as_ref()
        .is_none_or(|platforms| platforms.iter().any(|name| Os::from_name(name) == Some(os)))
}

fn field_shows_on(spec: &ConfigSpec, field: &FieldSpec, os: Os) -> bool {
    shows_on(&field.platforms, os)
        && field
            .section
            .as_deref()
            .and_then(|section| spec.section(section))
            .is_none_or(|section| shows_on(&section.platforms, os))
}

fn page_groups(spec: &ConfigSpec, os: Os) -> Vec<(String, Vec<String>)> {
    let in_cards: BTreeSet<&str> = spec
        .fields
        .values()
        .filter_map(|field| field.item_card_field.as_deref())
        .collect();
    let rows = |section: Option<&str>| -> Vec<String> {
        spec.fields
            .iter()
            .filter(|(name, field)| {
                field.section.as_deref() == section
                    && !in_cards.contains(name.as_str())
                    && field_shows_on(spec, field, os)
            })
            .map(|(name, field)| field.label.clone().unwrap_or_else(|| name.clone()))
            .collect()
    };
    let mut groups = vec![("General".to_string(), rows(None))];
    groups.extend(
        spec.sections
            .iter()
            .filter(|(_, section)| shows_on(&section.platforms, os))
            .map(|(id, section)| {
                let title = section.label.clone().unwrap_or_else(|| id.clone());
                (title, rows(Some(id)))
            }),
    );
    groups.retain(|(_, rows)| !rows.is_empty());
    groups
}

#[test]
fn every_setting_has_one_home_unless_another_page_is_a_deliberate_shortcut() {
    let pages = contract_pages();
    for os in Os::ALL {
        let mut homes: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for page in &pages {
            for (name, field) in &page.spec.fields {
                if field.has_stored_value() && field_shows_on(&page.spec, field, os) {
                    let key = field.config_key.as_deref().unwrap_or(name);
                    homes
                        .entry(format!("{}:{key}", page.scope))
                        .or_default()
                        .push(&page.page);
                }
            }
        }
        for (page, settings) in NATIVE_PAGE_SETTINGS {
            for setting in *settings {
                homes.entry(setting.to_string()).or_default().push(page);
            }
        }
        for (setting, pages) in &homes {
            let homes: Vec<&&str> = pages
                .iter()
                .filter(|page| !DELIBERATE_SHORTCUTS.contains(&(setting.as_str(), **page)))
                .collect();
            assert_eq!(
                homes.len(),
                1,
                "{setting} has {homes:?} as its homes on {os}: give it one page, and list any second page in DELIBERATE_SHORTCUTS"
            );
        }
    }
}

#[test]
fn every_row_sits_under_its_own_section_header() {
    for page in contract_pages() {
        for os in Os::ALL {
            let groups = page_groups(&page.spec, os);
            for pair in groups.windows(2) {
                let (above, (title, rows)) = (&pair[0].0, &pair[1]);
                let hidden =
                    rows.len() == 1 && rows[0].trim().to_lowercase() == title.trim().to_lowercase();
                assert!(
                    !hidden,
                    "{}: the panel hides the {title:?} header because its only row repeats it, so on {os} that row sits under {above:?}; rename the section or merge it",
                    page.page
                );
            }
        }
    }
}

#[test]
fn every_native_page_setting_names_a_scope() {
    for (page, settings) in NATIVE_PAGE_SETTINGS {
        for setting in *settings {
            assert!(
                setting
                    .split_once(':')
                    .is_some_and(|(scope, key)| !scope.is_empty() && !key.is_empty()),
                "{page} lists {setting:?}; name it scope:key like core:profile"
            );
        }
    }
}
