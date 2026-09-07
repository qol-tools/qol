use super::*;
use std::sync::mpsc;

struct Controlled {
    started: mpsc::Sender<()>,
    finish: mpsc::Receiver<()>,
}

impl Verifier for Controlled {
    fn identity(&self) -> &str {
        "fixture-verifier"
    }
    fn verify(&mut self, _: &str, _: &[Fact]) -> Result<Prediction> {
        self.started.send(())?;
        self.finish.recv()?;
        Ok(Prediction {
            consistent: true,
            polarity_preserved: true,
            scope_supported: true,
            comparison: "fixture".into(),
            answers: vec!["a".into()],
        })
    }
}

fn fixture_job() -> Job {
    Job {
        query: "how to boot KCD2 debug".into(),
        facts: vec![Fact {
            id: "a".into(),
            question: "How to run KCD2 debug?".into(),
            answer: "Run forge dev.".into(),
        }],
        context: "caller:one;revision:one".into(),
        lane: Some("launcher".into()),
    }
}

#[test]
fn cache_identity_preserves_scope_evidence_punctuation_and_model() {
    let original = fixture_job();
    let key = binding_key("v1", &original);
    for modified in [
        Job {
            query: "how to boot KCD2 without debug".into(),
            ..original.clone()
        },
        Job {
            query: "how to boot KCD2 debug?".into(),
            ..original.clone()
        },
        Job {
            context: "caller:two;revision:one".into(),
            ..original.clone()
        },
        Job {
            context: "caller:one;revision:two".into(),
            ..original.clone()
        },
        Job {
            facts: Vec::new(),
            ..original.clone()
        },
    ] {
        assert_ne!(key, binding_key("v1", &modified));
    }
    assert_ne!(key, binding_key("v2", &original));
}

#[test]
fn requests_remain_nonblocking_and_duplicates_share_one_job() {
    let (started, begin) = mpsc::channel();
    let (finish, release) = mpsc::channel();
    let root = std::env::temp_dir().join(format!(
        "qol-verification-{}-{}",
        std::process::id(),
        crate::text::now_iso().replace(':', "-")
    ));
    let service = Service::start(
        root.clone(),
        Controlled {
            started,
            finish: release,
        },
    )
    .unwrap();
    let job = fixture_job();
    assert_eq!(service.query(job.clone()), Status::Pending);
    begin.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(service.query(job.clone()), Status::Pending);
    finish.send(()).unwrap();
    let state = service.shared.state.lock().unwrap();
    let (state, timeout) = service
        .shared
        .changed
        .wait_timeout_while(state, Duration::from_secs(2), |state| {
            matches!(
                state.entries.get(&binding_key("fixture-verifier", &job)),
                Some(Entry::Pending)
            )
        })
        .unwrap();
    assert!(!timeout.timed_out());
    drop(state);
    assert_eq!(
        service.query(job.clone()),
        Status::Ready(Decision::Accepted("a".into()))
    );
    assert_eq!(
        load(
            &root,
            &binding_key("fixture-verifier", &job),
            "fixture-verifier"
        ),
        Some(Decision::Accepted("a".into()))
    );
    assert!(begin.try_recv().is_err());
    drop(service);
    let (started, calls) = mpsc::channel();
    let (_finish, release) = mpsc::channel();
    let restarted = Service::start(
        root.clone(),
        Controlled {
            started,
            finish: release,
        },
    )
    .unwrap();
    assert_eq!(restarted.query(job.clone()), Status::Pending);
    let state = restarted.shared.state.lock().unwrap();
    let (state, timeout) = restarted
        .shared
        .changed
        .wait_timeout_while(state, Duration::from_secs(2), |state| {
            matches!(
                state.entries.get(&binding_key("fixture-verifier", &job)),
                Some(Entry::Pending)
            )
        })
        .unwrap();
    assert!(!timeout.timed_out());
    drop(state);
    assert_eq!(
        restarted.query(job),
        Status::Ready(Decision::Accepted("a".into()))
    );
    assert!(calls.try_recv().is_err());
    drop(restarted);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn latest_launcher_query_replaces_queued_work_and_other_callers_are_bounded() {
    let (started, begin) = mpsc::channel();
    let (finish, release) = mpsc::channel();
    let root = std::env::temp_dir().join(format!(
        "qol-verification-queue-{}-{}",
        std::process::id(),
        crate::text::now_iso().replace(':', "-")
    ));
    let service = Service::start(
        root,
        Controlled {
            started,
            finish: release,
        },
    )
    .unwrap();
    assert_eq!(service.query(fixture_job()), Status::Pending);
    begin.recv_timeout(Duration::from_secs(2)).unwrap();
    for index in 0..10 {
        assert_eq!(
            service.query(Job {
                query: format!("launcher question {index}"),
                ..fixture_job()
            }),
            Status::Pending
        );
    }
    {
        let state = service.shared.state.lock().unwrap();
        assert_eq!(state.queued.len(), 1);
        assert_eq!(state.queued[0].1.query, "launcher question 9");
        assert_eq!(state.entries.len(), 2);
    }
    for index in 1..=QUEUE_LIMIT {
        let status = service.query(Job {
            query: format!("agent question {index}"),
            lane: None,
            ..fixture_job()
        });
        assert_eq!(
            status,
            if index < QUEUE_LIMIT {
                Status::Pending
            } else {
                Status::Unavailable
            }
        );
    }
    drop(service);
    drop(finish);
}
