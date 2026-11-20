use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use qol_gpui::ActionMark;
use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn chrome_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["window chrome"].take()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
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

fn owner_sources() -> Vec<String> {
    let mut found = Vec::new();
    for folder in strings(&chrome_entry()["owner"]) {
        rust_sources(&workspace().join(folder), &mut found);
    }
    found
}

fn defines(source: &str, function: &str) -> bool {
    source.contains(&format!("fn {function}("))
}

#[test]
fn rulebook_owners_are_the_gpui_platform_folders() {
    let folders = strings(&chrome_entry()["owner"]);
    assert_eq!(
        folders,
        [
            "libs/gpui/src/popup_window/platform/",
            "libs/gpui/src/platform/"
        ]
    );
    for folder in folders {
        assert!(
            workspace().join(&folder).join("mod.rs").is_file(),
            "{folder}"
        );
    }
}

#[test]
fn every_entry_point_is_defined_in_an_owner_folder() {
    let sources = owner_sources();
    for function in strings(&chrome_entry()["entry_points"]) {
        assert!(
            sources.iter().any(|source| defines(source, &function)),
            "{function} is not defined in the window chrome owners"
        );
    }
}

#[test]
fn every_exception_names_a_function_that_exists() {
    for item in chrome_entry()["allow"].as_array().unwrap() {
        let path = item["path"].as_str().unwrap();
        let function = item["text"].as_str().unwrap();
        let source = std::fs::read_to_string(workspace().join(path)).unwrap();
        assert!(
            defines(&source, function),
            "{path} no longer has {function}"
        );
        assert!(
            item["why"].as_str().is_some_and(|why| !why.is_empty()),
            "{path} needs a why"
        );
    }
}

fn marks_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["action marks"].take()
}

fn drawn_by_marks() -> BTreeSet<String> {
    ActionMark::ALL
        .iter()
        .flat_map(|mark| mark.drawn_by().iter().map(|item| item.to_string()))
        .collect()
}

#[test]
fn action_marks_are_owned_by_the_kit() {
    assert_eq!(marks_entry()["owner"].as_str(), Some("libs/gpui/"));
}

#[test]
fn action_mark_paths_are_what_the_marks_draw_with() {
    let listed: BTreeSet<String> = strings(&marks_entry()["paths"]).into_iter().collect();
    assert_eq!(listed, drawn_by_marks());
}

#[test]
fn every_action_mark_path_names_a_kit_item() {
    let kit = workspace().join("libs/gpui/src");
    for item in drawn_by_marks() {
        let (owner, name) = item.split_once("::").unwrap();
        let (file, shape) = match owner {
            "Icon" => ("icon.rs", format!("Self::{name} =>")),
            "pictures" => ("pictures/raster.rs", format!("pub fn {name}(")),
            "Busy" => ("busy.rs", format!("pub fn {name}(")),
            other => panic!("{item}: no kit file draws {other}"),
        };
        let source = std::fs::read_to_string(kit.join(file)).unwrap();
        assert!(
            source.contains(&shape),
            "{item} is not in libs/gpui/src/{file}"
        );
    }
}

fn scroll_entry() -> Value {
    let text = std::fs::read_to_string(workspace().join(".qac/owners.json")).unwrap();
    let mut owners: Value = serde_json::from_str(&text).unwrap();
    owners["scroll geometry"].take()
}

#[test]
fn scroll_geometry_is_owned_by_the_shared_scroll_files() {
    let files = strings(&scroll_entry()["owner"]);
    assert_eq!(
        files,
        ["libs/gpui/src/scrollbar.rs", "libs/gpui/src/scroll_list.rs"]
    );
    for file in files {
        assert!(workspace().join(&file).is_file(), "{file}");
    }
}

#[test]
fn every_scroll_geometry_path_is_a_gpui_scroll_method() {
    let elements = workspace().join("vendor/gpui/src/elements");
    let sources: Vec<String> = ["div.rs", "list.rs"]
        .iter()
        .map(|file| std::fs::read_to_string(elements.join(file)).unwrap())
        .collect();
    for name in strings(&scroll_entry()["paths"]) {
        assert!(
            sources
                .iter()
                .any(|source| source.contains(&format!("pub fn {name}("))),
            "{name} is not a scroll method in vendor/gpui/src/elements"
        );
    }
}

#[test]
fn scroll_geometry_names_kit_items_that_exist() {
    let entry = scroll_entry();
    let named = entry["use"].as_str().unwrap();
    let kit = workspace().join("libs/gpui/src");
    for (file, shape, item) in [
        ("kit.rs", "pub fn scroll_cue(", "scroll_cue"),
        ("scrollbar.rs", "pub enum ScrollSource", "ScrollSource"),
        (
            "scroll_list.rs",
            "pub struct SelectionScroll",
            "SelectionScroll",
        ),
        ("scroll_list.rs", "pub struct ScrollList", "ScrollList"),
        (
            "scroll_list.rs",
            "pub fn scroll_handle_by(",
            "scroll_handle_by",
        ),
    ] {
        assert!(named.contains(item), "the use text does not name {item}");
        let source = std::fs::read_to_string(kit.join(file)).unwrap();
        assert!(
            source.contains(shape),
            "{item} is not in libs/gpui/src/{file}"
        );
    }
}

#[test]
fn every_scroll_geometry_exception_says_why() {
    for item in scroll_entry()["allow"].as_array().unwrap() {
        let path = item["path"].as_str().unwrap();
        assert!(workspace().join(path).is_file(), "{path}");
        assert!(
            item["why"].as_str().is_some_and(|why| !why.is_empty()),
            "{path} needs a why"
        );
    }
}
