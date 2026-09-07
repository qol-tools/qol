use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use qol_plugin_daemon::daemon::ReadResult;
use qol_runtime::protocol::DaemonRequest;
use serde_json::{json, Value};

use super::{request, warm::WarmState};
use crate::store::Store;
use crate::verification::service::Verifier;
use crate::verification::{Fact, Prediction};

const QUERY: &str = "how do I fire up KCD2 debug";
const CALLER: &str = "/fixture/verification-private-a";

struct Fixture {
    state: Arc<Mutex<WarmState>>,
    root: PathBuf,
}

impl Fixture {
    fn new(verifier: impl Verifier) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "memory-verifier-{}-{}-{}",
            std::process::id(),
            crate::text::now_iso().replace(':', "-"),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        write_units(&root, &[unit("original command")]);
        let store = Store::resolve(Some(&root)).unwrap();
        let mut warm = WarmState::open(store, crate::aliases::embedded()).unwrap();
        warm.enable_verification(verifier).unwrap();
        Self {
            state: Arc::new(Mutex::new(warm)),
            root,
        }
    }

    fn ask(&mut self, fields: Value) -> Value {
        let mut input = json!({"query":QUERY,"agent_home":CALLER,"no_log":true});
        input
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        match request::handle(
            &mut self.state,
            &DaemonRequest {
                action: "ask".into(),
                input,
            },
        ) {
            ReadResult::HandledWithData(value) => value,
            _ => panic!("ask response required"),
        }
    }

    fn answer(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let result = self.ask(json!({}));
            if result["verification"]["status"] != "pending" {
                return result;
            }
            assert!(Instant::now() < deadline, "verification never completed");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn rows(&mut self) -> Value {
        match request::handle(
            &mut self.state,
            &DaemonRequest {
                action: "rows".into(),
                input: json!({"query":QUERY,"agent_home":CALLER,"no_log":true}),
            },
        ) {
            ReadResult::HandledWithData(value) => value,
            _ => panic!("rows response required"),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn unit(answer: &str) -> Value {
    json!({"key":"a","kind":"capture","agent_home":CALLER,"session":"fixture-session","text":format!("Q: How to run KCD2 in debug mode? A: {answer}")})
}

fn write_units(root: &std::path::Path, units: &[Value]) {
    let body = units
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    qol_fs::atomic_write(&root.join("units.jsonl"), body.as_bytes()).unwrap();
}

struct Immediate(Arc<AtomicUsize>);

impl Verifier for Immediate {
    fn identity(&self) -> &str {
        "fixture"
    }
    fn verify(&mut self, _: &str, _: &[Fact]) -> Result<Prediction> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Prediction {
            consistent: true,
            polarity_preserved: true,
            scope_supported: true,
            comparison: String::new(),
            answers: vec!["a".into()],
        })
    }
}

#[test]
fn edited_deleted_conflicting_and_disliked_evidence_cannot_reuse_a_binding() {
    for mutation in ["edit", "delete", "conflict", "dislike", "visibility"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut fixture = Fixture::new(Immediate(Arc::clone(&calls)));
        assert_eq!(fixture.ask(json!({}))["verification"]["status"], "pending");
        assert_eq!(fixture.answer()["answer"]["text"], "original command");
        assert_eq!(fixture.answer()["answer"]["text"], "original command");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let accepted = fixture.ask(json!({}));
        assert_eq!(accepted["outcome"], "supported");
        assert_eq!(accepted["reason_code"], "verified_answer");
        assert_eq!(fixture.rows()["outcome"], "supported");
        assert!(
            fixture.ask(json!({"agent_home":"/fixture/verification-private-b"}))["answer"]
                .is_null()
        );
        assert!(fixture.ask(json!({"exclude_session":"fixture-session"}))["answer"].is_null());
        match mutation {
            "edit" => write_units(
                &fixture.root,
                &[unit("replacement command with additional arguments")],
            ),
            "delete" => write_units(&fixture.root, &[]),
            "conflict" => {
                let mut conflicting = unit("incompatible command");
                conflicting["key"] = json!("b");
                write_units(&fixture.root, &[unit("original command"), conflicting]);
            }
            "dislike" => crate::feedback::append_vote(
                &fixture.root,
                &crate::retrieval_log::normalize_query(QUERY),
                "a",
                -1,
            ),
            "visibility" => {
                let mut private = unit("original command");
                private["agent_home"] = json!("/fixture/verification-private-b");
                write_units(&fixture.root, &[private]);
            }
            _ => unreachable!(),
        }
        let next = fixture.ask(json!({}));
        assert!(next["answer"].is_null(), "{mutation} reused a stale answer");
        if mutation == "edit" {
            assert_eq!(next["verification"]["status"], "pending");
            assert_eq!(
                fixture.answer()["answer"]["text"],
                "replacement command with additional arguments"
            );
            assert_eq!(calls.load(Ordering::Relaxed), 2);
        }
    }
}

struct Controlled {
    started: mpsc::Sender<String>,
    released: mpsc::Receiver<()>,
}

impl Verifier for Controlled {
    fn identity(&self) -> &str {
        "controlled-fixture"
    }
    fn verify(&mut self, _: &str, facts: &[Fact]) -> Result<Prediction> {
        self.started.send(facts[0].answer.clone())?;
        self.released.recv()?;
        Ok(Prediction {
            consistent: true,
            polarity_preserved: true,
            scope_supported: true,
            comparison: String::new(),
            answers: vec!["a".into()],
        })
    }
}

#[test]
fn in_flight_result_does_not_promote_replaced_evidence() {
    let (started, next) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut fixture = Fixture::new(Controlled { started, released });
    assert_eq!(fixture.ask(json!({}))["verification"]["status"], "pending");
    assert_eq!(
        next.recv_timeout(Duration::from_secs(2)).unwrap(),
        "original command"
    );
    write_units(
        &fixture.root,
        &[unit("replacement command, with different parameters")],
    );
    assert_eq!(fixture.ask(json!({}))["verification"]["status"], "pending");
    release.send(()).unwrap();
    assert_eq!(
        next.recv_timeout(Duration::from_secs(2)).unwrap(),
        "replacement command, with different parameters"
    );
    let waiting = fixture.ask(json!({}));
    assert_eq!(waiting["verification"]["status"], "pending");
    assert!(waiting["answer"].is_null());
    release.send(()).unwrap();
    assert_eq!(
        fixture.answer()["answer"]["text"],
        "replacement command, with different parameters"
    );
}
