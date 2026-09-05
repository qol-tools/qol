use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{policy_identity, Decision, Fact, Prediction};

const QUEUE_LIMIT: usize = 4;
const CACHE_LIMIT: usize = 256;
const RETRY_AFTER: Duration = Duration::from_secs(30);

pub trait Verifier: Send + 'static {
    fn identity(&self) -> &str;
    fn verify(&mut self, query: &str, facts: &[Fact]) -> Result<Prediction>;
}

#[derive(Clone)]
pub struct Job {
    pub query: String,
    pub facts: Vec<Fact>,
    pub context: String,
    pub lane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "decision")]
pub enum Status {
    Pending,
    Ready(Decision),
    Unavailable,
}

enum Entry {
    Pending,
    Ready(Decision),
    Unavailable(Instant),
}

struct State {
    queued: VecDeque<(String, Job)>,
    entries: HashMap<String, Entry>,
    finished: VecDeque<String>,
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

pub struct Service {
    shared: Arc<Shared>,
    identity: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    key: String,
    identity: String,
    policy: String,
    decision: Decision,
}

impl Service {
    pub fn start(root: PathBuf, verifier: impl Verifier) -> Result<Self> {
        let identity = verifier.identity().to_owned();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queued: VecDeque::new(),
                entries: HashMap::new(),
                finished: VecDeque::new(),
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("qol-memory-verifier".into())
            .spawn(move || {
                run(worker, root, verifier);
            })?;
        Ok(Self { shared, identity })
    }

    pub fn query(&self, job: Job) -> Status {
        let key = binding_key(&self.identity, &job);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match state.entries.get(&key) {
            Some(Entry::Pending) => return Status::Pending,
            Some(Entry::Ready(decision)) => return Status::Ready(decision.clone()),
            Some(Entry::Unavailable(at)) if at.elapsed() < RETRY_AFTER => {
                return Status::Unavailable
            }
            Some(Entry::Unavailable(_)) | None => {}
        }
        if state.closed {
            return Status::Unavailable;
        }
        if let Some(lane) = &job.lane {
            if let Some(index) = state
                .queued
                .iter()
                .position(|(_, pending)| pending.lane.as_ref() == Some(lane))
            {
                if let Some((old, _)) = state.queued.remove(index) {
                    state.entries.remove(&old);
                }
            }
        }
        if state.queued.len() >= QUEUE_LIMIT {
            return Status::Unavailable;
        }
        state.entries.insert(key.clone(), Entry::Pending);
        state.queued.push_back((key.clone(), job));
        self.shared.changed.notify_one();
        qol_runtime::probe!("QOL_MEMORY_DAEMON", "event=verification_queued key={key}");
        Status::Pending
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.queued.clear();
        self.shared.changed.notify_one();
    }
}

fn run(shared: Arc<Shared>, root: PathBuf, mut verifier: impl Verifier) {
    loop {
        let (key, job) = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while state.queued.is_empty() && !state.closed {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                return;
            }
            let Some(job) = state.queued.pop_front() else {
                continue;
            };
            job
        };
        let started = Instant::now();
        let cached = load(&root, &key, verifier.identity());
        let result = match cached {
            Some(decision) => {
                qol_runtime::probe!("QOL_MEMORY_DAEMON", "event=verification_cached key={key}");
                Ok(match decision {
                    Decision::Accepted(key) => super::check(
                        &job.query,
                        &job.facts,
                        &Prediction {
                            polarity_preserved: true,
                            scope_supported: true,
                            comparison: String::new(),
                            answers: vec![key],
                        },
                    ),
                    rejected => rejected,
                })
            }
            None => verifier
                .verify(&job.query, &job.facts)
                .map(|prediction| super::check(&job.query, &job.facts, &prediction)),
        };
        let entry = match result {
            Ok(decision) => {
                if let Err(error) = save(&root, &key, verifier.identity(), &decision) {
                    eprintln!("qol-memory: verification binding write failed: {error}");
                }
                qol_runtime::probe!(
                    "QOL_MEMORY_DAEMON",
                    "event=verification_completed key={key} decision={decision:?} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                Entry::Ready(decision)
            }
            Err(error) => {
                eprintln!("qol-memory: answer verification unavailable: {error}");
                qol_runtime::probe!(
                    "QOL_MEMORY_DAEMON",
                    "event=verification_unavailable key={key}"
                );
                Entry::Unavailable(Instant::now())
            }
        };
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.entries.insert(key.clone(), entry);
        state.finished.retain(|old| old != &key);
        state.finished.push_back(key);
        while state.finished.len() > CACHE_LIMIT {
            if let Some(old) = state.finished.pop_front() {
                state.entries.remove(&old);
            }
        }
        shared.changed.notify_all();
    }
}

pub fn binding_key(identity: &str, job: &Job) -> String {
    let input = serde_json::json!([
        policy_identity(),
        identity,
        job.context,
        job.query,
        job.facts
    ]);
    format!("{:x}", Sha256::digest(input.to_string().as_bytes()))
}

fn load(root: &Path, key: &str, identity: &str) -> Option<Decision> {
    let raw = std::fs::read(root.join(format!("{key}.json"))).ok()?;
    let binding: Binding = serde_json::from_slice(&raw).ok()?;
    (binding.key == key && binding.identity == identity && binding.policy == policy_identity())
        .then_some(binding.decision)
}

fn save(root: &Path, key: &str, identity: &str, decision: &Decision) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let binding = Binding {
        key: key.to_owned(),
        identity: identity.to_owned(),
        policy: policy_identity().to_owned(),
        decision: decision.clone(),
    };
    qol_fs::atomic_write(
        &root.join(format!("{key}.json")),
        &serde_json::to_vec(&binding)?,
    )?;
    let mut files = std::fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() == 69
                && name.ends_with(".json")
                && name[..64].bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    for old in files.iter().take(files.len().saturating_sub(CACHE_LIMIT)) {
        std::fs::remove_file(old.path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
