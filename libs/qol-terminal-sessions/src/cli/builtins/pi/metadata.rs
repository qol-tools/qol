use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::SessionFacts;

use super::environment::PiEnvironment;
use crate::cli::activity::file_activity;

const SESSION_CACHE_TTL: Duration = Duration::from_secs(30);
const REVERSE_READ_CHUNK: u64 = 64 * 1024;

pub(super) struct PiMetadata {
    pub session_name: Option<String>,
    pub external_id: Option<String>,
    pub has_activity: Option<bool>,
}

pub(super) struct PiMetadataResolver {
    environment: Arc<dyn PiEnvironment>,
    cache: Mutex<PiCache>,
}

#[derive(Default)]
struct PiCache {
    session_files: HashMap<i32, Timed<Option<PathBuf>>>,
    facts: HashMap<PathBuf, CachedFacts>,
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
    scanned_length: u64,
    session_name: Option<String>,
    has_message: bool,
}

impl PiMetadataResolver {
    pub fn new(environment: Arc<dyn PiEnvironment>) -> Self {
        Self {
            environment,
            cache: Mutex::new(PiCache::default()),
        }
    }

    pub fn resolve(&self, session: &SessionFacts) -> PiMetadata {
        let mut cache = self.cache.lock().ok();
        let path = cache
            .as_mut()
            .and_then(|cache| session_file(session, self.environment.as_ref(), cache));
        let external_id = path.as_deref().and_then(id_from_path);
        let facts = cache
            .as_mut()
            .and_then(|cache| path.as_deref().and_then(|path| cached_facts(path, cache)));
        let session_name = facts.as_ref().and_then(|facts| facts.session_name.clone());
        let has_activity = facts
            .as_ref()
            .and_then(|facts| file_activity(facts.signature.modified, facts.has_message));
        PiMetadata {
            session_name,
            external_id,
            has_activity,
        }
    }

    pub fn subscription_path(&self, session: &SessionFacts) -> Option<PathBuf> {
        let mut cache = self.cache.lock().ok()?;
        session_file(session, self.environment.as_ref(), &mut cache)
    }
}

fn session_file(
    session: &SessionFacts,
    environment: &dyn PiEnvironment,
    cache: &mut PiCache,
) -> Option<PathBuf> {
    session
        .foreground_pids
        .iter()
        .find_map(|pid| cached_session_file(*pid, &session.cwd, environment, cache))
}

fn cached_session_file(
    pid: i32,
    cwd: &str,
    environment: &dyn PiEnvironment,
    cache: &mut PiCache,
) -> Option<PathBuf> {
    if let Some(entry) = cache.session_files.get(&pid) {
        if entry.checked_at.elapsed() < SESSION_CACHE_TTL {
            return entry.value.clone();
        }
    }
    let value = environment.session_file(pid, cwd);
    cache.session_files.insert(
        pid,
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

fn id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let (_, id) = stem.rsplit_once('_')?;
    (!id.is_empty()).then(|| id.to_owned())
}

fn cached_facts(path: &Path, cache: &mut PiCache) -> Option<CachedFacts> {
    let signature = file_signature(path)?;
    if let Some(entry) = cache.facts.get(path) {
        if entry.signature == signature {
            return Some(clone_facts(entry));
        }
        if signature.length >= entry.scanned_length && entry.scanned_length > 0 {
            let (name, message) = scan_appended(path, entry.scanned_length);
            let facts = CachedFacts {
                signature,
                scanned_length: complete_length(path, signature.length),
                session_name: name.or_else(|| entry.session_name.clone()),
                has_message: entry.has_message || message,
            };
            cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
            return Some(facts);
        }
    }
    let facts = CachedFacts {
        signature,
        scanned_length: complete_length(path, signature.length),
        session_name: latest_session_name(path),
        has_message: any_message(path),
    };
    cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
    Some(facts)
}

fn clone_facts(facts: &CachedFacts) -> CachedFacts {
    CachedFacts {
        signature: facts.signature,
        scanned_length: facts.scanned_length,
        session_name: facts.session_name.clone(),
        has_message: facts.has_message,
    }
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

fn complete_length(path: &Path, length: u64) -> u64 {
    if length == 0 {
        return 0;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return 0;
    };
    if file.seek(SeekFrom::Start(length - 1)).is_err() {
        return 0;
    }
    let mut final_byte = [0];
    if file.read_exact(&mut final_byte).is_ok() && final_byte[0] == b'\n' {
        length
    } else {
        0
    }
}

fn scan_appended(path: &Path, offset: u64) -> (Option<String>, bool) {
    let Ok(mut file) = fs::File::open(path) else {
        return (None, false);
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (None, false);
    }
    let mut appended = Vec::new();
    if file.read_to_end(&mut appended).is_err() {
        return (None, false);
    }
    let mut name = None;
    let mut message = false;
    for line in appended.split(|byte| *byte == b'\n') {
        if let Some(found) = session_name(line) {
            name = Some(found);
        }
        if is_message(line) {
            message = true;
        }
    }
    (name, message)
}

fn latest_session_name(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut cursor = file.metadata().ok()?.len();
    let mut suffix = Vec::new();
    while cursor > 0 {
        let start = cursor.saturating_sub(REVERSE_READ_CHUNK);
        let length = usize::try_from(cursor - start).ok()?;
        let mut chunk = vec![0; length];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&suffix);
        let lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let complete_from = usize::from(start > 0);
        for line in lines[complete_from..].iter().rev() {
            if let Some(name) = session_name(line) {
                return Some(name);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    session_name(&suffix)
}

fn any_message(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    std::io::BufRead::lines(std::io::BufReader::new(file))
        .map_while(Result::ok)
        .any(|line| is_message(line.as_bytes()))
}

fn is_message(line: &[u8]) -> bool {
    memchr_contains(line, b"\"type\":\"message\"")
}

fn session_name(line: &[u8]) -> Option<String> {
    if !memchr_contains(line, b"\"type\":\"session_info\"") {
        return None;
    }
    let value = serde_json::from_slice::<Value>(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session_info") {
        return None;
    }
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn memchr_contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
