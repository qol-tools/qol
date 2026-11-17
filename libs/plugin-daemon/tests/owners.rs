use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn launch_env_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["daemon launch env"].take()
}

fn daemon_module_sources() -> Vec<String> {
    let folder = workspace().join("libs/plugin-daemon/src/daemon");
    std::fs::read_dir(folder)
        .unwrap()
        .map(|item| item.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect()
}

fn identifier_at(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

fn daemon_variables_conventions_declares() -> BTreeSet<String> {
    let source = std::fs::read_to_string(workspace().join("libs/conventions/src/lib.rs")).unwrap();
    source
        .match_indices("pub const ")
        .map(|(at, prefix)| identifier_at(&source[at + prefix.len()..]))
        .filter(|name| name.starts_with("ENV_DAEMON_"))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_owner_folder_exists() {
    for owner in launch_env_entry()["owner"].as_array().unwrap() {
        let folder = owner.as_str().unwrap();
        assert!(
            workspace().join(folder).is_dir(),
            "{folder} is not a folder"
        );
    }
}

#[test]
fn guarded_names_are_every_daemon_variable_conventions_declares() {
    let entry = launch_env_entry();
    let guarded: BTreeSet<String> = entry["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().to_string())
        .collect();
    let declared = daemon_variables_conventions_declares();
    assert!(
        !declared.is_empty(),
        "qol_conventions declares no ENV_DAEMON_ constant"
    );
    assert_eq!(guarded, declared);
}

#[test]
fn the_advice_names_functions_the_daemon_module_exports() {
    let sources = daemon_module_sources();
    let module =
        std::fs::read_to_string(workspace().join("libs/plugin-daemon/src/daemon/mod.rs")).unwrap();
    let entry = launch_env_entry();
    let advice = entry["use"].as_str().unwrap();
    let named: Vec<&str> = advice
        .match_indices("qol_plugin_daemon::daemon::")
        .map(|(at, prefix)| identifier_at(&advice[at + prefix.len()..]))
        .collect();
    assert!(
        !named.is_empty(),
        "the advice names no qol_plugin_daemon::daemon function"
    );
    for name in named {
        let defined = format!("pub fn {name}(");
        assert!(
            sources.iter().any(|source| source.contains(&defined)),
            "the daemon module has no pub fn {name}"
        );
        assert!(
            module.contains(&defined)
                || module
                    .lines()
                    .any(|line| line.starts_with("pub use ") && line.contains(name)),
            "qol_plugin_daemon::daemon does not export {name}"
        );
    }
}

#[test]
fn every_exception_names_a_file_that_still_has_its_text() {
    for item in launch_env_entry()["allow"].as_array().unwrap() {
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
