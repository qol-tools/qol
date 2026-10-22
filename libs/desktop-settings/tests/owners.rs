use std::path::{Path, PathBuf};

use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn settings_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["desktop settings"].take()
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

#[test]
fn rulebook_owner_is_the_desktop_settings_crate() {
    let owner = settings_entry()["owner"].as_str().unwrap().to_string();
    assert_eq!(owner, "libs/desktop-settings/src/");
    assert!(workspace().join(&owner).join("lib.rs").is_file());
}

#[test]
fn every_text_is_a_literal_in_the_owner() {
    let entry = settings_entry();
    let mut sources = Vec::new();
    rust_sources(
        &workspace().join(entry["owner"].as_str().unwrap()),
        &mut sources,
    );
    for text in entry["texts"].as_array().unwrap() {
        let quoted = format!("\"{}\"", text.as_str().unwrap());
        assert!(
            sources.iter().any(|source| source.contains(&quoted)),
            "the owner never uses {quoted}"
        );
    }
}

#[test]
fn every_exception_names_a_file_that_exists() {
    for item in settings_entry()["allow"].as_array().unwrap() {
        let path = item["path"].as_str().unwrap();
        let text = item["text"].as_str().unwrap();
        let source = std::fs::read_to_string(workspace().join(path)).unwrap();
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
