use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn liveness_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["process liveness"].take()
}

fn owner_lib() -> String {
    std::fs::read_to_string(workspace().join("libs/process/src/lib.rs")).unwrap()
}

fn public_function<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let start = source.find(&format!("pub fn {name}("))?;
    let rest = &source[start..];
    rest.find("\n}\n").map(|end| &rest[..end])
}

fn identifier_at(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

fn identifiers(text: &str) -> BTreeSet<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect()
}

#[test]
fn rulebook_owner_is_the_process_crate() {
    let owner = liveness_entry()["owner"].as_str().unwrap().to_string();
    assert_eq!(owner, "libs/process/src/");
    assert!(workspace().join(&owner).join("lib.rs").is_file());
}

#[test]
fn guarded_names_are_the_checks_is_pid_gone_combines() {
    let lib = owner_lib();
    let gone = public_function(&lib, "is_pid_gone").expect("qol_process has no pub fn is_pid_gone");
    let combined: BTreeSet<&str> = identifiers(gone)
        .into_iter()
        .filter(|word| word.starts_with("is_pid_") && *word != "is_pid_gone")
        .collect();
    let entry = liveness_entry();
    let guarded: BTreeSet<&str> = entry["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect();
    assert_eq!(guarded, combined);
    for name in combined {
        assert!(
            public_function(&lib, name).is_some(),
            "qol_process has no pub fn {name}"
        );
    }
}

#[test]
fn the_advice_names_public_functions_of_the_owner() {
    let lib = owner_lib();
    let entry = liveness_entry();
    let advice = entry["use"].as_str().unwrap();
    let named: Vec<&str> = advice
        .match_indices("qol_process::")
        .map(|(at, prefix)| identifier_at(&advice[at + prefix.len()..]))
        .collect();
    assert!(
        !named.is_empty(),
        "the advice names no qol_process function"
    );
    for name in named {
        assert!(
            public_function(&lib, name).is_some(),
            "qol_process has no pub fn {name}"
        );
    }
}

#[test]
fn every_exception_names_a_file_that_still_has_its_text() {
    for item in liveness_entry()["allow"].as_array().unwrap() {
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
