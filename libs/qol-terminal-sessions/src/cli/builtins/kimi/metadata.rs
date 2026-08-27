use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::cli::CliActivityEvidence;
use crate::{SessionBinding, SessionFacts};

use super::environment::{newest_write, KimiEnvironment, KimiSessionLocation};
use crate::cli::activity::{quiet_secs, recently_active};

pub(super) const SESSION_CACHE_TTL: Duration = Duration::from_secs(30);
pub(super) const MISSING_SESSION_CACHE_TTL: Duration = Duration::from_millis(500);
const NEW_SESSION_TITLE: &str = "New Session";
const MAX_SESSION_NAME_CHARS: usize = 80;

pub(super) struct KimiMetadata {
    pub session_name: Option<String>,
    pub external_id: Option<String>,
    pub has_activity: Option<bool>,
    pub activity: CliActivityEvidence,
}

pub(super) struct KimiMetadataResolver {
    environment: Arc<dyn KimiEnvironment>,
    cache: Mutex<KimiCache>,
}

#[derive(Default)]
struct KimiCache {
    locations: HashMap<String, Timed<Option<KimiSessionLocation>>>,
    facts: HashMap<PathBuf, CachedFacts>,
    observed: HashMap<String, HashMap<SessionBinding, Instant>>,
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

struct CachedFacts {
    signature: FileSignature,
    session_name: Option<String>,
    has_prompt: bool,
}

impl KimiMetadataResolver {
    pub fn new(environment: Arc<dyn KimiEnvironment>) -> Self {
        Self {
            environment,
            cache: Mutex::new(KimiCache::default()),
        }
    }

    pub fn resolve(&self, session: &SessionFacts) -> KimiMetadata {
        let mut cache = self.cache.lock().ok();
        let location = cache
            .as_mut()
            .and_then(|cache| cached_location(session, self.environment.as_ref(), cache));
        let facts = cache.as_mut().and_then(|cache| {
            location
                .as_ref()
                .and_then(|l| cached_facts(&l.state_path, cache))
        });
        let activity = match (facts.as_ref(), location.as_ref()) {
            (Some(facts), Some(location)) => {
                let newest = location.state_path.parent().and_then(newest_write);
                let fresh = newest.is_some_and(|write| recently_active(Some(write)) == Some(true));
                CliActivityEvidence {
                    file_fresh: Some(fresh),
                    file_has_work: Some(facts.has_prompt),
                    file_quiet_secs: quiet_secs(newest),
                }
            }
            _ => CliActivityEvidence::default(),
        };
        KimiMetadata {
            session_name: facts.as_ref().and_then(|f| f.session_name.clone()),
            external_id: location.as_ref().map(|l| l.session_id.clone()),
            has_activity: activity.combined(),
            activity,
        }
    }

    pub fn subscription_path(&self, session: &SessionFacts) -> Option<PathBuf> {
        let mut cache = self.cache.lock().ok()?;
        cached_location(session, self.environment.as_ref(), &mut cache)
            .map(|location| location.state_path)
    }

    pub fn subscription_dir(&self, session: &SessionFacts) -> Option<PathBuf> {
        let home = kimi_subscription_home()?;
        let index = fs::read_to_string(home.join("session_index.jsonl")).ok()?;
        let mut groups: Vec<PathBuf> = index
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|entry| {
                entry.get("workDir").and_then(serde_json::Value::as_str)
                    == Some(session.cwd.as_str())
            })
            .filter_map(|entry| {
                let session_dir = PathBuf::from(
                    entry
                        .get("sessionDir")
                        .and_then(serde_json::Value::as_str)?,
                );
                let group = session_dir.parent()?.to_path_buf();
                group.is_dir().then_some(group)
            })
            .collect();
        groups.sort();
        groups.dedup();
        groups.into_iter().next()
    }
}

fn kimi_subscription_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("KIMI_CODE_HOME") {
        return expand_tilde(PathBuf::from(dir));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".kimi-code"))
}

fn expand_tilde(path: PathBuf) -> Option<PathBuf> {
    let text = path.to_str()?;
    if text != "~" && !text.starts_with("~/") {
        return Some(path);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    if text == "~" || text == "~/" {
        return Some(home);
    }
    Some(home.join(text.strip_prefix("~/")?))
}

fn cached_location(
    session: &SessionFacts,
    environment: &dyn KimiEnvironment,
    cache: &mut KimiCache,
) -> Option<KimiSessionLocation> {
    if cwd_is_ambiguous(session, cache) {
        return None;
    }
    let cwd = &session.cwd;
    if let Some(entry) = cache.locations.get(cwd) {
        let fresh_for = if entry.value.is_some() {
            SESSION_CACHE_TTL
        } else {
            MISSING_SESSION_CACHE_TTL
        };
        if entry.checked_at.elapsed() < fresh_for {
            return entry.value.clone();
        }
    }
    let value = environment.session(cwd);
    cache.locations.insert(
        cwd.to_owned(),
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

fn cwd_is_ambiguous(session: &SessionFacts, cache: &mut KimiCache) -> bool {
    let now = Instant::now();
    for observed in cache.observed.values_mut() {
        observed.retain(|_, seen_at| now.duration_since(*seen_at) < SESSION_CACHE_TTL);
    }
    cache.observed.retain(|_, observed| !observed.is_empty());
    let Ok(binding) = session.binding() else {
        return true;
    };
    let observed = cache.observed.entry(session.cwd.clone()).or_default();
    observed.insert(binding, now);
    observed.len() > 1
}

fn cached_facts(path: &Path, cache: &mut KimiCache) -> Option<CachedFacts> {
    let signature = file_signature(path)?;
    if let Some(entry) = cache.facts.get(path) {
        if entry.signature == signature {
            return Some(clone_facts(entry));
        }
    }
    let facts = parse_state(path).map(|parsed| CachedFacts {
        signature,
        session_name: parsed.session_name,
        has_prompt: parsed.has_prompt,
    })?;
    cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
    Some(facts)
}

fn clone_facts(facts: &CachedFacts) -> CachedFacts {
    CachedFacts {
        signature: facts.signature,
        session_name: facts.session_name.clone(),
        has_prompt: facts.has_prompt,
    }
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

struct ParsedState {
    session_name: Option<String>,
    has_prompt: bool,
}

fn parse_state(path: &Path) -> Option<ParsedState> {
    let record = serde_json::from_str::<StateRecord>(&fs::read_to_string(path).ok()?).ok()?;
    let session_name = record
        .title
        .map(|title| title.trim().to_owned())
        .filter(|title| {
            !title.is_empty()
                && title != NEW_SESSION_TITLE
                && title.chars().count() <= MAX_SESSION_NAME_CHARS
        });
    let has_prompt = record
        .last_prompt
        .is_some_and(|prompt| !prompt.trim().is_empty());
    Some(ParsedState {
        session_name,
        has_prompt,
    })
}

#[derive(Deserialize)]
struct StateRecord {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "lastPrompt")]
    last_prompt: Option<String>,
}
