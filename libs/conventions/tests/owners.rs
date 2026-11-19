use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn values_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["conventions values"].take()
}

fn rust_sources(dir: &Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(std::fs::read_to_string(&path).unwrap());
        }
    }
}

fn str_const_values(source: &str) -> Vec<&str> {
    source
        .match_indices("pub const ")
        .filter_map(|(at, prefix)| {
            let (_, rest) = source[at + prefix.len()..].split_once(':')?;
            let rest = rest.trim_start();
            let rest = rest
                .strip_prefix("&'static str")
                .or_else(|| rest.strip_prefix("&str"))?;
            let rest = rest.trim_start().strip_prefix('=')?;
            let rest = rest.trim_start().strip_prefix('"')?;
            rest.find('"').map(|end| &rest[..end])
        })
        .collect()
}

const INT_TYPES: [&str; 12] = [
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
];

fn int_const_values(source: &str) -> Vec<String> {
    source
        .match_indices("pub const ")
        .filter_map(|(at, prefix)| {
            let (_, rest) = source[at + prefix.len()..].split_once(':')?;
            let rest = rest.trim_start();
            let kind = INT_TYPES.iter().find(|kind| rest.starts_with(**kind))?;
            let rest = rest[kind.len()..].trim_start().strip_prefix('=')?;
            let (literal, _) = rest.split_once(';')?;
            let literal = literal.trim().replace('_', "");
            let digits = literal
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(literal.len());
            let (number, suffix) = literal.split_at(digits);
            if number.is_empty() || !(suffix.is_empty() || INT_TYPES.contains(&suffix)) {
                return None;
            }
            let trimmed = number.trim_start_matches('0');
            Some(match trimmed {
                "" => "0".to_string(),
                _ => trimmed.to_string(),
            })
        })
        .collect()
}

fn owner_values() -> BTreeSet<String> {
    let mut sources = Vec::new();
    rust_sources(&workspace().join("libs/conventions/src"), &mut sources);
    let mut values = BTreeSet::new();
    for source in &sources {
        values.extend(str_const_values(source).into_iter().map(str::to_string));
        values.extend(int_const_values(source));
    }
    values
}

fn module_body<'a>(source: &'a str, module: &str) -> &'a str {
    let start = source
        .find(&format!("pub mod {module} {{"))
        .unwrap_or_else(|| panic!("qol_conventions has no pub mod {module}"));
    let rest = &source[start..];
    &rest[..rest.find("\n}\n").expect("the module closes")]
}

#[test]
fn rulebook_owner_is_the_conventions_crate() {
    let entry = values_entry();
    let owner = entry["owner"].as_str().unwrap();
    assert_eq!(owner, "libs/conventions/src/");
    assert!(workspace().join(owner).join("lib.rs").is_file());
    assert_eq!(entry["const_texts"], Value::Bool(true));
    assert_eq!(entry["const_numbers"], Value::Bool(true));
}

#[test]
fn every_skipped_text_is_a_value_of_the_owner_with_a_why() {
    let values = owner_values();
    let mut skipped = BTreeSet::new();
    for item in values_entry()["skip"].as_array().unwrap() {
        let text = item["text"].as_str().unwrap();
        assert!(
            values.contains(text),
            "{text:?} is no longer the value of a qol_conventions constant; drop it from skip"
        );
        assert!(
            item["why"].as_str().is_some_and(|why| !why.is_empty()),
            "{text:?} needs a why"
        );
        assert!(
            skipped.insert(text.to_string()),
            "{text:?} is skipped twice"
        );
    }
}

#[test]
fn the_prefix_starts_every_api_route() {
    let lib = std::fs::read_to_string(workspace().join("libs/conventions/src/lib.rs")).unwrap();
    let routes = str_const_values(module_body(&lib, "api_routes"));
    let first = routes.first().expect("api_routes has a route");
    let shared = routes.iter().fold(first.len(), |len, route| {
        first
            .bytes()
            .zip(route.bytes())
            .take(len)
            .take_while(|(a, b)| a == b)
            .count()
    });
    let folder_end = first[..shared]
        .rfind('/')
        .expect("the routes share a folder");
    let prefix = &first[..=folder_end];
    let entry = values_entry();
    let prefixes: Vec<&str> = entry["prefixes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect();
    assert_eq!(prefixes, [prefix]);
}

#[test]
fn every_exception_names_a_file_that_still_has_its_text() {
    for item in values_entry()["allow"].as_array().unwrap() {
        let path = item["path"].as_str().unwrap();
        let text = item["text"].as_str().unwrap();
        let source = std::fs::read_to_string(workspace().join(path)).unwrap();
        assert!(source.contains(text), "{path} no longer has {text}");
        assert!(
            item["why"].as_str().is_some_and(|why| !why.is_empty()),
            "{path} needs a why"
        );
    }
}
