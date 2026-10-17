use std::path::Path;

use qol_platform::Os;
use serde_json::Value;

fn os_entry() -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["os"].take()
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect()
}

#[test]
fn rulebook_names_match_os() {
    assert_eq!(strings(&os_entry()["names"]), Os::ALL.map(Os::name));
}

#[test]
fn rulebook_labels_match_os() {
    assert_eq!(strings(&os_entry()["labels"]), Os::ALL.map(Os::label));
}

#[test]
fn rulebook_owner_is_the_os_module() {
    let owner = os_entry()["owner"].as_str().unwrap().to_string();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(owner, "libs/platform/src/os/");
    assert!(root.join(&owner).join("mod.rs").is_file());
}

#[test]
fn every_exception_names_a_file_that_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for item in os_entry()["allow"].as_array().unwrap() {
        let path = item["path"].as_str().unwrap();
        let text = item["text"].as_str().unwrap();
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        assert!(
            source.contains(&format!("\"{text}\"")),
            "{path} no longer has {text}"
        );
        assert!(
            item["why"].as_str().is_some_and(|why| !why.is_empty()),
            "{path} needs a why"
        );
    }
}
