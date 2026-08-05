use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::SessionFacts;

use super::environment::CodexEnvironment;
use crate::cli::activity::file_activity;

const ROLLOUT_CACHE_TTL: Duration = Duration::from_secs(30);

pub(super) struct CodexMetadata {
    pub thread_name: Option<String>,
    pub external_id: Option<String>,
    pub has_activity: Option<bool>,
}

pub(super) struct CodexMetadataResolver {
    environment: Arc<dyn CodexEnvironment>,
    cache: Mutex<CodexCache>,
}

#[derive(Default)]
struct CodexCache {
    rollouts: HashMap<i32, Timed<Option<PathBuf>>>,
    index: Option<SessionIndex>,
}

struct Timed<T> {
    value: T,
    checked_at: Instant,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct FileSignature {
    modified: Option<SystemTime>,
    length: u64,
}

struct SessionIndex {
    path: PathBuf,
    signature: FileSignature,
    names: HashMap<String, String>,
}

impl CodexMetadataResolver {
    pub fn new(environment: Arc<dyn CodexEnvironment>) -> Self {
        Self {
            environment,
            cache: Mutex::new(CodexCache::default()),
        }
    }

    pub fn resolve(&self, session: &SessionFacts) -> CodexMetadata {
        let mut cache = self.cache.lock().ok();
        let rollout = cache
            .as_mut()
            .and_then(|cache| rollout_path(session, self.environment.as_ref(), cache));
        let external_id = rollout.as_deref().and_then(uuid_from_path);
        let indexed_name = cache
            .as_mut()
            .zip(external_id.as_deref())
            .and_then(|(cache, id)| thread_name(id, self.environment.as_ref(), cache));
        CodexMetadata {
            thread_name: title_thread_name(&session.title).or(indexed_name),
            external_id,
            has_activity: title_activity(&session.title)
                .or_else(|| rollout.as_deref().and_then(rollout_activity)),
        }
    }

    pub fn subscription_path(&self) -> Option<PathBuf> {
        self.environment.session_index_path()
    }
}

fn rollout_path(
    session: &SessionFacts,
    environment: &dyn CodexEnvironment,
    cache: &mut CodexCache,
) -> Option<PathBuf> {
    session
        .foreground_pids
        .iter()
        .find_map(|pid| cached_rollout(*pid, environment, cache))
}

fn cached_rollout(
    pid: i32,
    environment: &dyn CodexEnvironment,
    cache: &mut CodexCache,
) -> Option<PathBuf> {
    if let Some(entry) = cache.rollouts.get(&pid) {
        if entry.checked_at.elapsed() < ROLLOUT_CACHE_TTL {
            return entry.value.clone();
        }
    }
    let value = environment.open_rollout(pid);
    cache.rollouts.insert(
        pid,
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

fn uuid_from_path(path: &Path) -> Option<String> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let parts = stem.split('-').collect::<Vec<_>>();
    (parts.len() >= 5).then(|| parts[parts.len() - 5..].join("-"))
}

fn thread_name(
    id: &str,
    environment: &dyn CodexEnvironment,
    cache: &mut CodexCache,
) -> Option<String> {
    let path = environment.session_index_path()?;
    let signature = file_signature(&path)?;
    let stale = cache
        .index
        .as_ref()
        .is_none_or(|index| index.path != path || index.signature != signature);
    if stale {
        cache.index = load_index(path, signature);
    }
    cache.index.as_ref()?.names.get(id).cloned()
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

fn load_index(path: PathBuf, signature: FileSignature) -> Option<SessionIndex> {
    let content = fs::read_to_string(&path).ok()?;
    let mut names = HashMap::new();
    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = value.get("thread_name").and_then(Value::as_str) else {
            continue;
        };
        if !name.is_empty() {
            names.insert(id.to_owned(), name.to_owned());
        }
    }
    Some(SessionIndex {
        path,
        signature,
        names,
    })
}

fn title_items(title: &str) -> Option<Vec<&str>> {
    let trimmed = title.trim();
    let items = trimmed.split(" | ").map(str::trim).collect::<Vec<_>>();
    (items.len() >= 3).then_some(items)
}

fn title_activity(title: &str) -> Option<bool> {
    let items = title_items(title)?;
    for item in &items {
        match *item {
            "Working" | "Thinking" => return Some(true),
            "Ready" => return Some(false),
            "Action Required" => return Some(false),
            _ => {}
        }
    }
    None
}

fn title_thread_name(title: &str) -> Option<String> {
    let items = title_items(title)?;
    let state = items
        .iter()
        .copied()
        .find(|item| matches!(*item, "Working" | "Thinking" | "Ready" | "Action Required"))?;
    let name = items.last().copied()?;
    let project = items.first().copied()?;
    (state != name && name != project).then(|| name.to_owned())
}

fn rollout_activity(path: &Path) -> Option<bool> {
    let signature = file_signature(path)?;
    file_activity(signature.modified, rollout_has_work(path))
}

fn rollout_has_work(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .take(2)
        .count()
        > 1
}
