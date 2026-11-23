use std::ffi::OsString;

use serde_json::{json, Value};

use super::cli::execute;
use super::store::{PageStore, PAGES_MAX};
use crate::verb::{EXIT_REFUSED, EXIT_UNKNOWN, EXIT_USAGE};

fn store() -> (tempfile::TempDir, PageStore) {
    let root = tempfile::TempDir::new().unwrap();
    let store = PageStore::with_dir(root.path().join("pages"));
    (root, store)
}

fn run(store: &PageStore, args: &[&str], stdin: &str, now_ms: u64) -> (Value, u8) {
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    let outcome = execute(&args, store, &mut stdin.as_bytes(), now_ms);
    (
        serde_json::from_str(&outcome.stdout).unwrap_or(Value::Null),
        outcome.code,
    )
}

fn page(id: &str, owner: &str) -> Value {
    json!({
        "id": id,
        "owner": owner,
        "title": "KCD2 tests",
        "tab": "tests",
        "blocks": [
            { "kind": "kv", "rows": [{ "label": "build", "value": "main 609fd3a5", "tone": "good" }] },
            { "kind": "checklist", "items": [
                { "id": "1", "text": "Jump with hip flash on", "detail": "the beam comes from the gun" },
                { "id": "2", "text": "Aim down sights" }
            ] },
            { "kind": "steps", "items": [{ "text": "Start the game", "state": "done" }, { "text": "Load a save" }] },
            { "kind": "links", "items": [{ "label": "notes", "url": "https://example.com/notes" }] },
            { "kind": "callout", "tone": "warn", "text": "Save first" },
            { "kind": "actions", "items": [{ "id": "rerun", "label": "run the round again" }] }
        ]
    })
}

fn set(store: &PageStore, page: &Value, now_ms: u64) -> (Value, u8) {
    run(store, &["set"], &page.to_string(), now_ms)
}

#[test]
fn a_page_set_from_stdin_reads_back_with_its_defaults() {
    let (_root, store) = store();
    let (shown, code) = set(&store, &page("kcd2-tests", "check-kcd2"), 10);
    assert_eq!(code, 0, "{shown}");
    assert_eq!(shown["set_ms"], 10);
    assert_eq!(shown["answers"], json!([]));
    let (read, code) = run(&store, &["read"], "", 11);
    assert_eq!(code, 0);
    let first = &read["pages"][0]["page"];
    assert_eq!(first["owner"], "check-kcd2");
    assert_eq!(first["blocks"][2]["items"][1]["state"], "todo");
    assert_eq!(first["blocks"][4]["tone"], "warn");
    assert_eq!(first["blocks"][0]["rows"][0]["tone"], "good");
}

#[test]
fn a_page_set_from_a_file_is_the_same_page() {
    let (root, store) = store();
    let file = root.path().join("page.json");
    std::fs::write(&file, page("from-file", "fork-a").to_string()).unwrap();
    let (shown, code) = run(&store, &["set", "--file", file.to_str().unwrap()], "", 5);
    assert_eq!(code, 0, "{shown}");
    assert_eq!(shown["page"]["id"], "from-file");
}

#[test]
fn answers_are_recorded_replaced_and_kept_across_a_reset() {
    let (_root, store) = store();
    set(&store, &page("p", "fork-a"), 1);
    let (_, code) = run(
        &store,
        &[
            "answer",
            "--id",
            "p",
            "--item",
            "1",
            "--verdict",
            "fail",
            "--note",
            "beam\nwent  straight",
        ],
        "",
        2,
    );
    assert_eq!(code, 0);
    run(
        &store,
        &["answer", "--id", "p", "--item", "2", "--verdict", "pass"],
        "",
        3,
    );
    let (shown, _) = run(&store, &["answers", "--id", "p"], "", 4);
    assert_eq!(
        shown["answers"],
        json!([
            { "item": "1", "verdict": "fail", "note": "beam went straight", "at_ms": 2 },
            { "item": "2", "verdict": "pass", "at_ms": 3 }
        ])
    );
    run(
        &store,
        &["answer", "--id", "p", "--item", "1", "--verdict", "skip"],
        "",
        5,
    );
    let mut changed = page("p", "fork-a");
    changed["blocks"][1]["items"] = json!([{ "id": "1", "text": "Jump again" }]);
    let (shown, code) = set(&store, &changed, 6);
    assert_eq!(code, 0);
    assert_eq!(shown["set_ms"], 1);
    assert_eq!(shown["updated_ms"], 6);
    assert_eq!(
        shown["answers"],
        json!([{ "item": "1", "verdict": "skip", "at_ms": 5 }])
    );
}

#[test]
fn unknown_pages_and_items_exit_three() {
    let (_root, store) = store();
    set(&store, &page("p", "fork-a"), 1);
    for args in [
        &["answers", "--id", "nope"][..],
        &["answer", "--id", "nope", "--item", "1", "--verdict", "pass"],
        &[
            "answer",
            "--id",
            "p",
            "--item",
            "rerun",
            "--verdict",
            "pass",
        ],
        &["answer", "--id", "p", "--item", "9", "--verdict", "pass"],
        &["clear", "--id", "nope"],
    ] {
        let (shown, code) = run(&store, args, "", 2);
        assert_eq!(code, EXIT_UNKNOWN, "{args:?} {shown}");
    }
}

#[test]
fn usage_errors_exit_two() {
    let (_root, store) = store();
    let mut unknown_field = page("p", "fork-a");
    unknown_field["colour"] = json!("red");
    let mut unknown_kind = page("p", "fork-a");
    unknown_kind["blocks"][0]["kind"] = json!("table");
    let mut unknown_block_field = page("p", "fork-a");
    unknown_block_field["blocks"][4]["colour"] = json!("red");
    for stdin in [
        "not json".to_owned(),
        unknown_field.to_string(),
        unknown_kind.to_string(),
        unknown_block_field.to_string(),
    ] {
        let (shown, code) = run(&store, &["set"], &stdin, 1);
        assert_eq!(code, EXIT_USAGE, "{stdin} {shown}");
    }
    for args in [
        &[][..],
        &["post"],
        &["read", "--id", "p"],
        &["answer", "--id", "p", "--item", "1", "--verdict", "maybe"],
        &["clear"],
        &["set", "--file", "/no/such/page.json"],
    ] {
        let (_, code) = run(&store, args, "", 1);
        assert_eq!(code, EXIT_USAGE, "{args:?}");
    }
}

#[test]
fn pages_that_break_the_rules_are_refused() {
    let (_root, store) = store();
    let edits: [(&str, Value); 9] = [
        ("/id", json!("Has Caps")),
        ("/owner", json!("")),
        ("/title", json!(" \n ")),
        ("/tab", json!("a tab that is far too long")),
        ("/blocks", json!([])),
        ("/blocks/1/items", json!([])),
        ("/blocks/1/items/1/id", json!("1")),
        ("/blocks/5/items/0/id", json!("1")),
        ("/blocks/3/items/0/url", json!("javascript:alert(1)")),
    ];
    for (pointer, value) in edits {
        let mut broken = page("p", "fork-a");
        *broken.pointer_mut(pointer).unwrap() = value;
        let (shown, code) = set(&store, &broken, 1);
        assert_eq!(code, EXIT_REFUSED, "{pointer} {shown}");
    }
    let (read, _) = run(&store, &["read"], "", 2);
    assert_eq!(read["pages"], json!([]));
}

#[test]
fn hidden_characters_never_reach_the_board() {
    let (_root, store) = store();
    let mut sneaky = page("p", "fork-a");
    sneaky["title"] = json!("KCD2\u{202E}stset\u{200B} tests\u{7}");
    let (shown, code) = set(&store, &sneaky, 1);
    assert_eq!(code, 0, "{shown}");
    assert_eq!(shown["page"]["title"], "KCD2 stset tests");
}

#[test]
fn a_page_id_belongs_to_its_owner_and_pages_are_capped() {
    let (_root, store) = store();
    set(&store, &page("p", "fork-a"), 1);
    let (_, code) = set(&store, &page("p", "fork-b"), 2);
    assert_eq!(code, EXIT_REFUSED);
    let (_, code) = run(&store, &["clear", "--id", "p", "--owner", "fork-b"], "", 3);
    assert_eq!(code, EXIT_REFUSED);
    for n in 1..PAGES_MAX {
        let (_, code) = set(&store, &page(&format!("p{n}"), "fork-b"), 3 + n as u64);
        assert_eq!(code, 0);
    }
    let (_, code) = set(&store, &page("one-more", "fork-b"), 99);
    assert_eq!(code, EXIT_REFUSED);
    let (_, code) = set(&store, &page("p", "fork-a"), 100);
    assert_eq!(code, 0, "replacing a page is not a new one");
}

#[test]
fn clear_takes_one_page_or_every_page_of_an_owner() {
    let (_root, store) = store();
    set(&store, &page("a1", "fork-a"), 1);
    set(&store, &page("a2", "fork-a"), 2);
    set(&store, &page("b1", "fork-b"), 3);
    let (shown, code) = run(&store, &["clear", "--id", "b1", "--owner", "fork-b"], "", 4);
    assert_eq!(code, 0);
    assert_eq!(shown, json!({ "cleared": ["b1"] }));
    let (shown, code) = run(&store, &["clear", "--owner", "fork-a"], "", 5);
    assert_eq!(code, 0);
    assert_eq!(shown, json!({ "cleared": ["a1", "a2"] }));
    let (read, _) = run(&store, &["read"], "", 6);
    assert_eq!(read["pages"], json!([]));
}

#[test]
fn the_tool_sets_pages_under_the_calling_fork_and_reads_only_its_own() {
    let (_root, store) = store();
    let mut unowned = page("p", "");
    unowned.as_object_mut().unwrap().remove("owner");
    let created = super::tool(
        &store,
        "fork-a",
        &json!({ "action": "set", "page": unowned }),
        1,
    );
    let shown: Value = serde_json::from_str(&created.unwrap()).unwrap();
    assert_eq!(shown["page"]["owner"], "fork-a");
    let posing = super::tool(
        &store,
        "fork-a",
        &json!({ "action": "set", "page": page("q", "fork-b") }),
        2,
    );
    assert!(posing.unwrap_err().contains("leave owner out"));
    super::tool(
        &store,
        "fork-b",
        &json!({ "action": "set", "page": page("q", "") }),
        3,
    )
    .unwrap();
    let mine: Value = serde_json::from_str(
        &super::tool(&store, "fork-a", &json!({ "action": "read" }), 4).unwrap(),
    )
    .unwrap();
    assert_eq!(mine["pages"].as_array().unwrap().len(), 1);
    assert!(super::tool(
        &store,
        "fork-a",
        &json!({ "action": "clear", "id": "q" }),
        5
    )
    .is_err());
    let cleared = super::tool(
        &store,
        "fork-a",
        &json!({ "action": "clear", "id": "p" }),
        6,
    );
    assert_eq!(cleared.unwrap(), r#"{"cleared":["p"]}"#);
    assert!(super::tool(&store, "fork-a", &json!({ "action": "post" }), 7).is_err());
}

#[test]
fn help_prints_the_contract() {
    let (_root, store) = store();
    let args = [OsString::from("help")];
    let outcome = execute(&args, &store, &mut "".as_bytes(), 1);
    assert_eq!(outcome.code, 0);
    assert!(outcome.stdout.contains("Page JSON:"));
}
