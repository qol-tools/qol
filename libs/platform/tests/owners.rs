use std::path::Path;

use qol_platform::Os;
use serde_json::Value;

fn entry(name: &str) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners[name].take()
}

fn os_entry() -> Value {
    entry("os")
}

fn display_entry() -> Value {
    entry("display server")
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect()
}

fn assert_exceptions_hold(entry: &Value) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for item in entry["allow"].as_array().unwrap() {
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
    assert_exceptions_hold(&os_entry());
}

fn rust_sources(dir: &Path, found: &mut Vec<String>) {
    for item in std::fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(std::fs::read_to_string(&path).unwrap());
        }
    }
}

#[test]
fn display_rulebook_owner_is_the_platform_crate() {
    let owner = display_entry()["owner"].as_str().unwrap().to_string();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(owner, "libs/platform/src/");
    assert!(root.join(&owner).join("lib.rs").is_file());
}

#[test]
fn every_display_text_is_read_by_the_owner() {
    let display = display_entry();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    rust_sources(&root.join(display["owner"].as_str().unwrap()), &mut sources);
    for text in strings(&display["texts"]) {
        let quoted = format!("\"{text}\"");
        assert!(
            sources.iter().any(|source| source.contains(&quoted)),
            "the display server owner no longer reads {text}"
        );
    }
}

#[test]
fn every_display_exception_names_a_file_that_exists() {
    assert_exceptions_hold(&display_entry());
}
