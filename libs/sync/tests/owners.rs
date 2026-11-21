use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn locks_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["locks"].take()
}

fn owner_sources() -> String {
    let owner = locks_entry()["owner"].as_str().unwrap().to_string();
    let mut sources = String::new();
    for entry in std::fs::read_dir(workspace().join(owner)).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            sources.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    sources
}

fn identifier_at(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

fn after_generics(text: &str) -> &str {
    if !text.starts_with('<') {
        return text;
    }
    let mut depth = 0;
    for (at, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &text[at + 1..];
                }
            }
            _ => {}
        }
    }
    text
}

fn std_locks_the_owner_wraps(sources: &str) -> BTreeSet<String> {
    sources
        .match_indices("pub struct ")
        .filter_map(|(at, prefix)| {
            let rest = &sources[at + prefix.len()..];
            let name = identifier_at(rest);
            let body = after_generics(&rest[name.len()..]);
            body.starts_with(&format!("(std::sync::{name}"))
                .then(|| name.to_string())
        })
        .collect()
}

fn guarded_std_locks() -> BTreeSet<String> {
    locks_entry()["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| {
            let path = path.as_str().unwrap();
            path.strip_prefix("std::sync::")
                .unwrap_or_else(|| panic!("{path} is not a std::sync lock"))
                .to_string()
        })
        .collect()
}

#[test]
fn rulebook_owner_is_the_sync_crate() {
    let owner = locks_entry()["owner"].as_str().unwrap().to_string();
    assert_eq!(owner, "libs/sync/src/");
    assert!(workspace().join(&owner).join("lib.rs").is_file());
}

#[test]
fn guarded_std_locks_are_the_ones_the_owner_wraps() {
    let wrapped = std_locks_the_owner_wraps(&owner_sources());
    assert!(!wrapped.is_empty(), "qol_sync wraps no std::sync lock");
    assert_eq!(guarded_std_locks(), wrapped);
}

#[test]
fn the_advice_names_every_lock_of_the_owner() {
    let lib = std::fs::read_to_string(workspace().join("libs/sync/src/lib.rs")).unwrap();
    let entry = locks_entry();
    let advice = entry["use"].as_str().unwrap();
    assert!(advice.starts_with("qol_sync::"), "{advice}");
    for name in guarded_std_locks() {
        assert!(advice.contains(&name), "the advice never names {name}");
        assert!(
            lib.contains(&format!("{name},")) || lib.contains(&format!("{name};")),
            "qol_sync does not export {name}"
        );
    }
}

#[test]
fn every_exception_names_a_file_that_still_has_its_text() {
    for item in locks_entry()["allow"].as_array().unwrap() {
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
