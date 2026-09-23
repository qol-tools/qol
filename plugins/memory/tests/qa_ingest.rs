use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use qol_memory::ingest::{self, IngestRoot, IngestRoots, KeySet};
use qol_memory::store::{Store, Unit};
use serde_json::{json, Value};

const QUESTION: &str = "where is the launcher configuration stored?";
const ANSWER: &str = "The launcher configuration lives in the launcher plugin configuration file.";
const PRIVATE_HOME: &str = "/fixture/private-agent-home";

struct Fixture {
    root: PathBuf,
    store: Store,
    roots: IngestRoots,
    path: PathBuf,
    source: &'static str,
}

impl Fixture {
    fn new(source: &'static str) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("qol-memory-qa-{}-{now}-{id}", std::process::id()));
        let transcript_root = root.join("transcripts");
        std::fs::create_dir_all(&transcript_root).unwrap();
        let store = Store::resolve(Some(&root.join("store"))).unwrap();
        std::fs::create_dir_all(store.root()).unwrap();
        let path = transcript_root.join("session.jsonl");
        std::fs::write(&path, "").unwrap();
        let roots = IngestRoots {
            roots: vec![IngestRoot {
                path: transcript_root,
                source,
                agent_home: PRIVATE_HOME.into(),
            }],
        };
        Self {
            root,
            store,
            roots,
            path,
            source,
        }
    }

    fn append(&self, role: &str, text: &str, stop_reason: Option<&str>) {
        let event = match self.source {
            "pi" => json!({
                "type": "message",
                "message": {
                    "role": role, "content": text, "timestamp": 1788600000000u64,
                    "stopReason": stop_reason,
                },
            }),
            "claude" => json!({
                "type": role, "sessionId": "fixture-session", "cwd": "/fixture/project",
                "timestamp": "2026-09-05T10:00:00.000Z",
                "message": { "content": [{ "type": "text", "text": text }], "stop_reason": stop_reason },
            }),
            source => panic!("unsupported fixture source: {source}"),
        };
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .unwrap();
        if self.source == "pi" && file.metadata().unwrap().len() == 0 {
            writeln!(
                file,
                "{}",
                json!({"type":"session", "id":"fixture-session", "cwd":"/fixture/project"})
            )
            .unwrap();
        }
        writeln!(file, "{event}").unwrap();
    }

    fn ingest(&self) -> ingest::IngestReport {
        let mut keys = KeySet::load(&self.store).unwrap();
        ingest::ingest_paths(
            &self.store,
            &self.roots,
            std::slice::from_ref(&self.path),
            &mut keys,
        )
        .unwrap()
    }

    fn captures(&self) -> Vec<Unit> {
        self.store
            .read_units()
            .unwrap()
            .items
            .into_iter()
            .filter(|unit| unit.source.as_deref() == Some(ingest::qa::QA_SOURCE))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn qa_capture_survives_checkpoint_reload_between_question_and_answer() {
    for source in ingest::SUPPORTED_SOURCES {
        for split in [false, true] {
            let fixture = Fixture::new(source);
            fixture.append("user", QUESTION, None);
            if split {
                fixture.ingest();
            }
            fixture.append("assistant", ANSWER, None);
            fixture.ingest();
            let captures = fixture.captures();
            assert_eq!(captures.len(), 1, "source={source} split={split}");
            assert_eq!(captures[0].text, format!("Q: {QUESTION} A: {ANSWER}"));
            assert_eq!(fixture.ingest().appended, 0);
        }
    }
}

#[test]
fn qa_capture_retains_the_source_identity() {
    for source in ingest::SUPPORTED_SOURCES {
        let fixture = Fixture::new(source);
        fixture.append("user", QUESTION, None);
        fixture.append("assistant", ANSWER, None);
        fixture.ingest();
        let captures = fixture.captures();
        assert_eq!(captures.len(), 1);
        let capture = &captures[0];
        assert_eq!(capture.agent_home.as_deref(), Some(PRIVATE_HOME));
        assert_eq!(capture.file.as_deref(), Some("session.jsonl"));
        assert_eq!(capture.session.as_deref(), Some("fixture-session"));
        assert_eq!(capture.cwd.as_deref(), Some("/fixture/project"));
        let units = fixture.store.read_units().unwrap();
        assert_eq!(capture.host, units.items[0].host);
        let raw = std::fs::read_to_string(fixture.store.units_path()).unwrap();
        let stored: Vec<Value> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(stored[2]["question_key"], stored[0]["key"]);
        assert_eq!(stored[2]["answer_key"], stored[1]["key"]);
    }
}

#[test]
fn qa_capture_waits_through_progress_for_the_answer_in_every_batch_partition() {
    let progress =
        "I will inspect the launcher configuration and report back after checking the code.";
    for source in ingest::SUPPORTED_SOURCES {
        for boundaries in 0..4 {
            let fixture = Fixture::new(source);
            for (index, (role, text)) in [
                ("user", QUESTION),
                ("assistant", progress),
                ("assistant", ANSWER),
            ]
            .into_iter()
            .enumerate()
            {
                fixture.append(role, text, None);
                if index == 2 || boundaries & (1 << index) != 0 {
                    fixture.ingest();
                }
            }
            let captures = fixture.captures();
            assert_eq!(captures.len(), 1, "source={source} boundaries={boundaries}");
            assert_eq!(captures[0].text, format!("Q: {QUESTION} A: {ANSWER}"));
        }
    }
}

#[test]
fn qa_capture_ignores_nonfinal_assistant_messages() {
    for source in ingest::SUPPORTED_SOURCES {
        for stop in [
            "tool_use",
            "toolUse",
            "max_tokens",
            "length",
            "error",
            "aborted",
            "unknown_stop_reason",
        ] {
            let fixture = Fixture::new(source);
            fixture.append("user", QUESTION, None);
            fixture.append("assistant", "The launcher configuration location still needs confirmation from the next tool result.", Some(stop));
            fixture.ingest();
            assert!(fixture.captures().is_empty(), "source={source} stop={stop}");
            fixture.append("assistant", ANSWER, Some("end_turn"));
            fixture.ingest();
            assert_eq!(fixture.captures().len(), 1);
        }
    }
}

#[test]
fn qa_checkpoint_clears_on_new_user_input_and_transcript_replacement() {
    for replace in [false, true] {
        let fixture = Fixture::new("claude");
        fixture.append("user", QUESTION, None);
        fixture.ingest();
        if replace {
            std::fs::write(&fixture.path, "").unwrap();
        }
        if !replace {
            fixture.append("user", "Never mind, move on to the next task.", None);
            fixture.ingest();
        }
        fixture.append("assistant", ANSWER, None);
        fixture.ingest();
        assert!(fixture.captures().is_empty(), "replace={replace}");
    }
}

#[test]
fn qa_checkpoint_recovers_when_loading_an_older_parser_state() {
    let fixture = Fixture::new("claude");
    fixture.append("user", QUESTION, None);
    fixture.ingest();
    let state_path = fixture.store.ingest_state_path();
    let mut state: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    for file in state["files"].as_object_mut().unwrap().values_mut() {
        file["parser"] = json!(ingest::PARSER_VERSION - 1);
        file.as_object_mut().unwrap().remove("pending_question");
    }
    std::fs::write(&state_path, state.to_string()).unwrap();
    fixture.append("assistant", ANSWER, None);
    assert_eq!(fixture.ingest().reparsed, 1);
    assert_eq!(fixture.captures().len(), 1);
    assert_eq!(fixture.ingest().appended, 0);
}

#[test]
fn reingestion_replaces_legacy_qa_in_retrieval_without_removing_history() {
    let fixture = Fixture::new("claude");
    let text = format!("Q: {QUESTION} A: {ANSWER}");
    let legacy = json!({
        "key": "legacy-qa", "source": ingest::qa::QA_SOURCE, "kind": ingest::CAPTURE_KIND,
        "text": text, "ts": "2026-09-01T10:00:00.000Z", "cwd": "/fixture/project",
    });
    std::fs::write(fixture.store.units_path(), format!("{legacy}\n")).unwrap();
    fixture.append("user", QUESTION, None);
    fixture.append("assistant", ANSWER, None);
    fixture.ingest();
    assert_eq!(fixture.captures().len(), 2);
    let request = qol_memory::ask::AskRequest {
        query: QUESTION.into(),
        k: 8,
        brief: false,
        exclude_session: None,
        agent_home: Some(PRIVATE_HOME.into()),
    };
    let output =
        qol_memory::ask::run(&fixture.store, &qol_memory::aliases::embedded(), &request).unwrap();
    let hits = output.units.unwrap();
    assert!(!hits.iter().any(|unit| unit.key == "legacy-qa"));
    assert!(hits
        .iter()
        .any(|unit| unit.kind == ingest::CAPTURE_KIND && unit.text == text));
}
