use std::path::{Path, PathBuf};

use serde_json::Value;

const ENTRIES: [&str; 2] = ["qol folders", "temp folder"];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn owners_entry(name: &str) -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners[name].take()
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

fn identifier_at(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

#[test]
fn rulebook_owner_is_the_config_crate() {
    for name in ENTRIES {
        let owner = owners_entry(name)["owner"].as_str().unwrap().to_string();
        assert_eq!(owner, "libs/config/src/", "{name}");
        assert!(workspace().join(&owner).join("lib.rs").is_file());
    }
}

#[test]
fn every_path_is_used_in_the_owner() {
    for name in ENTRIES {
        let entry = owners_entry(name);
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
}

#[test]
fn the_advice_names_public_functions_of_the_owner() {
    let lib = std::fs::read_to_string(workspace().join("libs/config/src/lib.rs")).unwrap();
    for name in ENTRIES {
        let entry = owners_entry(name);
        let advice = entry["use"].as_str().unwrap();
        let named: Vec<&str> = advice
            .match_indices("qol_config::")
            .map(|(at, prefix)| identifier_at(&advice[at + prefix.len()..]))
            .collect();
        assert!(!named.is_empty(), "{name} names no qol_config function");
        for function in named {
            assert!(
                lib.contains(&format!("pub fn {function}(")),
                "{name} names qol_config::{function}, which qol_config does not export"
            );
        }
    }
}

#[test]
fn every_exception_names_a_file_that_still_has_its_path() {
    for name in ENTRIES {
        for item in owners_entry(name)["allow"].as_array().unwrap() {
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
}
