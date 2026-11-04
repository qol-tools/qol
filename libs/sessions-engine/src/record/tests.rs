use std::ffi::OsString;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::cli::{execute, EXIT_UNKNOWN, EXIT_USAGE};
use super::store::RecordStore;

fn abs(path: &str) -> String {
    std::env::temp_dir()
        .join(path.trim_start_matches('/'))
        .to_string_lossy()
        .into_owned()
}

fn store(root: &Path) -> RecordStore {
    RecordStore::with_dir(root.join("records"))
}

fn run(store: &RecordStore, now_ms: u64, args: &[&str]) -> (Value, u8) {
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    let outcome = execute(&args, store, now_ms);
    let value = serde_json::from_str(&outcome.stdout)
        .unwrap_or_else(|error| panic!("{args:?} printed {:?}: {error}", outcome.stdout));
    (value, outcome.code)
}

fn claim(store: &RecordStore, now_ms: u64, description: &str) -> Value {
    let (value, code) = run(
        store,
        now_ms,
        &[
            "claim",
            "--description",
            description,
            "--cwd",
            &abs("/project"),
        ],
    );
    assert_eq!(code, 0, "{value}");
    value
}

#[test]
fn claim_gives_the_key_friendly_and_description_for_each_form_of_description() {
    let cases = [
        (
            "switchboard-flow-improvements: orchestration revisions",
            "switchboard-flow-improvements",
            "orchestration revisions",
            "switchboard-flow-improvements: orchestration revisions",
        ),
        ("my-key:", "my-key", "", "my-key"),
        (
            "Fix the Login Bug",
            "fix-the-login-bug",
            "Fix the Login Bug",
            "fix-the-login-bug: Fix the Login Bug",
        ),
        (
            "Key: Upper",
            "key-upper",
            "Key: Upper",
            "key-upper: Key: Upper",
        ),
        ("日本語", "session", "日本語", "session: 日本語"),
        ("", "session", "", "session"),
    ];
    for (description, key, friendly, text) in cases {
        let root = tempfile::TempDir::new().unwrap();
        let claimed = claim(&store(root.path()), 5, description);
        assert_eq!(
            claimed,
            json!({ "key": key, "friendly": friendly, "description": text }),
            "{description:?}"
        );
    }
}

#[test]
fn claim_stores_the_record_and_never_reuses_a_key() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    let keys = (0..4)
        .map(|_| {
            claim(&store, 7, "dup: same")["key"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["dup", "dup-2", "dup-3", "dup-4"]);
    let slugged = (0..3)
        .map(|_| {
            claim(&store, 7, "Same Words")["key"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(slugged, ["same-words", "same-words-2", "same-words-3"]);

    let (record, code) = run(&store, 0, &["show", "--key", "dup-2"]);
    assert_eq!(code, 0);
    assert_eq!(
        record,
        json!({
            "key": "dup-2",
            "friendly": "same",
            "cwd": abs("/project"),
            "created_ms": 7,
            "parts": [],
        }),
        "done and stopped_ms stay absent until set"
    );
}

#[test]
fn part_appends_the_next_number_and_an_agent_id_already_held_changes_nothing() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "work: thing");
    let steps = [
        ("a1", abs("/t/a1.jsonl"), 10, 1),
        ("a2", abs("/t/a2.jsonl"), 20, 2),
        ("a1", abs("/t/other.jsonl"), 30, 1),
        ("a2", abs("/t/a2.jsonl"), 40, 2),
        ("a3", abs("/t/a3.jsonl"), 50, 3),
    ];
    for (agent, transcript, now, part) in steps {
        let (value, code) = run(
            &store,
            now,
            &[
                "part",
                "--key",
                "work",
                "--agent-id",
                agent,
                "--transcript",
                &transcript,
            ],
        );
        assert_eq!(
            (code, value),
            (0, json!({ "key": "work", "part": part })),
            "{agent}"
        );
    }
    let (record, _) = run(&store, 0, &["show", "--key", "work"]);
    assert_eq!(
        record["parts"],
        json!([
            { "n": 1, "agent_id": "a1", "transcript": abs("/t/a1.jsonl"), "started_ms": 10 },
            { "n": 2, "agent_id": "a2", "transcript": abs("/t/a2.jsonl"), "started_ms": 20 },
            { "n": 3, "agent_id": "a3", "transcript": abs("/t/a3.jsonl"), "started_ms": 50 },
        ])
    );
}

#[test]
fn archive_copies_each_existing_transcript_and_skips_a_missing_one() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "work: thing");
    let first = root.path().join("agent-a1.jsonl");
    let third = root.path().join("agent-a3.jsonl");
    fs::write(&first, "one\n").unwrap();
    fs::write(&third, "three\n").unwrap();
    for (agent, path) in [
        ("a1", first.clone()),
        ("a2", root.path().join("gone.jsonl")),
        ("a3", third.clone()),
    ] {
        run(
            &store,
            2,
            &[
                "part",
                "--key",
                "work",
                "--agent-id",
                agent,
                "--transcript",
                path.to_str().unwrap(),
            ],
        );
    }

    let (value, code) = run(&store, 3, &["archive", "--key", "work"]);
    assert_eq!(code, 0);
    let archive = |n: u32| store.archive_path("work", n);
    assert_eq!(
        value,
        json!({
            "key": "work",
            "archived": [archive(1).display().to_string(), archive(3).display().to_string()],
        })
    );
    assert_eq!(fs::read_to_string(archive(1)).unwrap(), "one\n");
    assert_eq!(fs::read_to_string(archive(3)).unwrap(), "three\n");
    assert!(!archive(2).exists());
    assert!(archive(1).ends_with("records/work/part-1.jsonl"));

    fs::write(&first, "one, grown\n").unwrap();
    fs::remove_file(&third).unwrap();
    let (value, _) = run(&store, 4, &["archive", "--key", "work"]);
    assert_eq!(value["archived"], json!([archive(1).display().to_string()]));
    assert_eq!(fs::read_to_string(archive(1)).unwrap(), "one, grown\n");
    assert_eq!(
        fs::read_to_string(archive(3)).unwrap(),
        "three\n",
        "a transcript that is gone keeps its earlier copy"
    );
    let leftovers = fs::read_dir(root.path().join("records/work"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 2, "no temporary copy is left behind");
}

#[test]
fn done_and_stopped_mark_the_record_and_the_latest_done_wins() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "work: thing");
    let (first, code) = run(
        &store,
        100,
        &["done", "--key", "work", "--reason", "merged"],
    );
    assert_eq!(code, 0);
    assert_eq!(first["done"], json!({ "at_ms": 100, "reason": "merged" }));
    let (second, _) = run(&store, 200, &["done", "--key", "work", "--reason", "later"]);
    assert_eq!(second["done"], json!({ "at_ms": 200, "reason": "later" }));
    let (stopped, code) = run(&store, 300, &["stopped", "--key", "work"]);
    assert_eq!((code, stopped["stopped_ms"].clone()), (0, json!(300)));
    assert_eq!(stopped["done"], second["done"]);
}

#[test]
fn resume_clears_done_and_stopped_and_is_a_no_op_when_neither_is_set() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "work: thing");
    run(
        &store,
        2,
        &[
            "part",
            "--key",
            "work",
            "--agent-id",
            "a1",
            "--transcript",
            &abs("/t.jsonl"),
        ],
    );
    let (untouched, code) = run(&store, 5, &["resume", "--key", "work"]);
    assert_eq!(code, 0);
    assert!(untouched.get("done").is_none() && untouched.get("stopped_ms").is_none());

    type Marks = fn(&RecordStore);
    let cases: [(&str, Marks); 3] = [
        ("done only", |store| {
            run(store, 10, &["done", "--key", "work", "--reason", "merged"]);
        }),
        ("stopped only", |store| {
            run(store, 10, &["stopped", "--key", "work"]);
        }),
        ("both", |store| {
            run(store, 10, &["done", "--key", "work", "--reason", "merged"]);
            run(store, 11, &["stopped", "--key", "work"]);
        }),
    ];
    for (name, mark) in cases {
        mark(&store);
        let (marked, _) = run(&store, 0, &["show", "--key", "work"]);
        assert!(
            marked.get("done").is_some() || marked.get("stopped_ms").is_some(),
            "{name}"
        );
        let (resumed, code) = run(&store, 20, &["resume", "--key", "work"]);
        assert_eq!(code, 0, "{name}");
        assert!(resumed.get("done").is_none(), "{name}: {resumed}");
        assert!(resumed.get("stopped_ms").is_none(), "{name}: {resumed}");
        assert_eq!(resumed["parts"].as_array().unwrap().len(), 1, "{name}");
        let (stored, _) = run(&store, 0, &["show", "--key", "work"]);
        assert_eq!(stored, resumed, "{name}");
    }
    let (unknown, code) = run(&store, 0, &["resume", "--key", "nope"]);
    assert_eq!(code, EXIT_UNKNOWN);
    assert!(unknown["error"].is_string());
}

#[test]
fn show_finds_a_record_by_key_or_by_an_agent_id_among_its_parts() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "one: first");
    claim(&store, 2, "two: second");
    for (key, agent) in [("one", "a1"), ("two", "b1"), ("two", "b2")] {
        run(
            &store,
            3,
            &[
                "part",
                "--key",
                key,
                "--agent-id",
                agent,
                "--transcript",
                &abs("/t.jsonl"),
            ],
        );
    }
    for (agent, key) in [("a1", "one"), ("b1", "two"), ("b2", "two")] {
        let (record, code) = run(&store, 0, &["show", "--agent-id", agent]);
        assert_eq!((code, record["key"].as_str()), (0, Some(key)), "{agent}");
    }
}

#[test]
fn list_is_newest_created_first() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    assert_eq!(run(&store, 0, &["list"]), (json!({ "records": [] }), 0));
    claim(&store, 30, "middle: m");
    claim(&store, 10, "oldest: o");
    claim(&store, 50, "newest: n");
    let (value, code) = run(&store, 0, &["list"]);
    let keys = value["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["key"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!((code, keys), (0, vec!["newest", "middle", "oldest"]));
}

#[test]
fn an_unknown_key_or_agent_id_exits_3_with_an_error_object() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "work: thing");
    let cases: [&[&str]; 10] = [
        &["resume", "--key", "nope"],
        &["show", "--key", "nope"],
        &["show", "--agent-id", "nope"],
        &[
            "part",
            "--key",
            "nope",
            "--agent-id",
            "a",
            "--transcript",
            &abs("/t"),
        ],
        &["archive", "--key", "nope"],
        &["done", "--key", "nope", "--reason", "x"],
        &["stopped", "--key", "nope"],
        &["show", "--key", "../etc/passwd"],
        &["show", "--key", "Work"],
        &["show", "--key", ""],
    ];
    for args in cases {
        let (value, code) = run(&store, 0, args);
        assert_eq!(code, EXIT_UNKNOWN, "{args:?}");
        assert!(
            value["error"].as_str().is_some_and(|text| !text.is_empty()),
            "{args:?}"
        );
        assert_eq!(value.as_object().unwrap().len(), 1, "{args:?}");
    }
}

#[test]
fn a_usage_error_exits_2_with_an_error_object() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    claim(&store, 1, "work: thing");
    let cases: [&[&str]; 18] = [
        &[],
        &["resume"],
        &["resume", "--key", "work", "--reason", "x"],
        &["bogus"],
        &["claim"],
        &["claim", "--description", "x"],
        &["claim", "--description", "x", "--cwd", "relative/dir"],
        &["claim", "--description", "x", "--cwd", "/a", "--extra", "1"],
        &[
            "claim",
            "--description",
            "x",
            "--description",
            "y",
            "--cwd",
            "/a",
        ],
        &["part", "--key", "work", "--agent-id", "a"],
        &[
            "part",
            "--key",
            "work",
            "--agent-id",
            "a",
            "--transcript",
            "rel.jsonl",
        ],
        &["archive"],
        &["done", "--key", "work"],
        &["stopped", "--key"],
        &["show"],
        &["show", "--key", "work", "--agent-id", "a"],
        &["list", "--key", "work"],
        &["show", "work"],
    ];
    for args in cases {
        let (value, code) = run(&store, 0, args);
        assert_eq!(code, EXIT_USAGE, "{args:?}");
        assert!(value["error"].is_string(), "{args:?}");
    }
}

#[test]
fn help_prints_the_usage_and_exits_0() {
    let root = tempfile::TempDir::new().unwrap();
    let store = store(root.path());
    for verb in ["help", "--help", "-h"] {
        let outcome = execute(&[OsString::from(verb)], &store, 0);
        assert_eq!(outcome.code, 0);
        assert!(outcome.stdout.contains("record claim --description"));
    }
}

mod search {
    use super::*;
    use crate::record::rows;

    fn archived(root: &Path, store: &RecordStore, key: &str, agent: &str, lines: &[&str]) {
        let transcript = root.join(format!("{agent}.jsonl"));
        let content = lines
            .iter()
            .map(|text| {
                json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&transcript, content).unwrap();
        run(
            store,
            2,
            &[
                "part",
                "--key",
                key,
                "--agent-id",
                agent,
                "--transcript",
                transcript.to_str().unwrap(),
            ],
        );
        run(store, 3, &["archive", "--key", key]);
    }

    #[test]
    fn rows_filter_by_query_and_by_the_words_in_the_archived_chat() {
        let root = tempfile::TempDir::new().unwrap();
        let store = store(root.path());
        claim(&store, 10, "alpha: Billing Fix");
        claim(&store, 20, "beta: other thing");
        claim(&store, 30, "gamma: nothing saved");
        archived(
            root.path(),
            &store,
            "alpha",
            "a1",
            &["please refund the invoice"],
        );
        archived(
            root.path(),
            &store,
            "beta",
            "b1",
            &["something about bikes"],
        );

        let keys = |query: Option<&str>, terms: Option<Vec<String>>, limit: usize| {
            rows(&store, query, terms.as_deref(), limit)
                .unwrap()
                .into_iter()
                .map(|row| row.key)
                .collect::<Vec<_>>()
        };
        let words = |text: &str| Some(text.split(' ').map(str::to_owned).collect::<Vec<_>>());
        assert_eq!(keys(None, None, 20), ["gamma", "beta", "alpha"]);
        assert_eq!(keys(None, None, 2), ["gamma", "beta"]);
        assert_eq!(keys(Some("billing"), None, 20), ["alpha"]);
        assert_eq!(
            keys(Some(&abs("/project").to_lowercase()), None, 20),
            ["gamma", "beta", "alpha"]
        );
        assert_eq!(keys(Some("nomatch"), None, 20), Vec::<String>::new());
        assert_eq!(keys(None, words("invoice refund"), 20), ["alpha"]);
        assert_eq!(keys(None, words("bikes"), 20), ["beta"]);
        assert_eq!(keys(None, words("invoice bikes"), 20), Vec::<String>::new());
        assert_eq!(
            keys(Some("beta"), words("invoice"), 20),
            Vec::<String>::new()
        );

        let found = rows(&store, None, words("invoice").as_deref(), 20).unwrap();
        assert!(found[0].excerpt.as_deref().unwrap().contains("invoice"));
        assert_eq!(
            found[0].parts[0].archive.as_deref(),
            Some(store.archive_path("alpha", 1).to_str().unwrap())
        );
        let plain = rows(&store, Some("gamma"), None, 20).unwrap();
        assert!(plain[0].parts.is_empty() && plain[0].excerpt.is_none());
    }
}
