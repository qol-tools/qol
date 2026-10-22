use std::path::{Path, PathBuf};

use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn folders_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["qol folders"].take()
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
fn rulebook_owner_is_the_config_crate() {
    let owner = folders_entry()["owner"].as_str().unwrap().to_string();
    assert_eq!(owner, "libs/config/src/");
    assert!(workspace().join(&owner).join("lib.rs").is_file());
}

#[test]
fn every_path_is_used_in_the_owner() {
    let entry = folders_entry();
    let mut sources = Vec::new();
    rust_sources(
        &workspace().join(entry["owner"].as_str().unwrap()),
        &mut sources,
    );
    for path in entry["paths"].as_array().unwrap() {
        let path = path.as_str().unwrap();
        assert!(
            sources.iter().any(|source| source.contains(path)),
            "the owner never uses {path}"
        );
    }
}

#[test]
fn every_exception_names_a_file_that_still_has_its_path() {
    for item in folders_entry()["allow"].as_array().unwrap() {
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
