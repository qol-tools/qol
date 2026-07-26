use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::SessionFacts;

use super::environment::{ClaudeEnvironment, ClaudeSessionLocation};

const SESSION_CACHE_TTL: Duration = Duration::from_secs(30);
const REVERSE_READ_CHUNK: u64 = 64 * 1024;

pub(super) struct ClaudeMetadata {
    pub custom_title: Option<String>,
    pub external_id: Option<String>,
}

pub(super) struct ClaudeMetadataResolver {
    environment: Arc<dyn ClaudeEnvironment>,
    cache: Mutex<ClaudeCache>,
}

#[derive(Default)]
struct ClaudeCache {
    sessions: HashMap<i32, Timed<Option<ClaudeSessionLocation>>>,
    titles: HashMap<PathBuf, CachedTitle>,
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

struct CachedTitle {
    signature: FileSignature,
    scanned_length: u64,
    value: Option<String>,
}

impl ClaudeMetadataResolver {
    pub fn new(environment: Arc<dyn ClaudeEnvironment>) -> Self {
        Self {
            environment,
            cache: Mutex::new(ClaudeCache::default()),
        }
    }

    pub fn resolve(&self, session: &SessionFacts) -> ClaudeMetadata {
        let mut cache = self.cache.lock().ok();
        let location = cache
            .as_mut()
            .and_then(|cache| session_location(session, self.environment.as_ref(), cache));
        let custom_title = cache.as_mut().and_then(|cache| {
            location
                .as_ref()
                .and_then(|location| cached_title(&location.transcript_path, cache))
        });
        ClaudeMetadata {
            custom_title,
            external_id: location.map(|location| location.external_id),
        }
    }

    pub fn subscription_path(&self, session: &SessionFacts) -> Option<PathBuf> {
        let mut cache = self.cache.lock().ok()?;
        session_location(session, self.environment.as_ref(), &mut cache)
            .map(|location| location.transcript_path)
    }
}

fn session_location(
    session: &SessionFacts,
    environment: &dyn ClaudeEnvironment,
    cache: &mut ClaudeCache,
) -> Option<ClaudeSessionLocation> {
    session
        .foreground_pids
        .iter()
        .find_map(|pid| cached_session(*pid, environment, cache))
}

fn cached_session(
    pid: i32,
    environment: &dyn ClaudeEnvironment,
    cache: &mut ClaudeCache,
) -> Option<ClaudeSessionLocation> {
    if let Some(entry) = cache.sessions.get(&pid) {
        if entry.checked_at.elapsed() < SESSION_CACHE_TTL {
            return entry.value.clone();
        }
    }
    let value = environment.session(pid);
    cache.sessions.insert(
        pid,
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

fn cached_title(path: &Path, cache: &mut ClaudeCache) -> Option<String> {
    let signature = file_signature(path)?;
    if let Some(entry) = cache.titles.get(path) {
        if entry.signature == signature {
            return entry.value.clone();
        }
        if signature.length > entry.signature.length
            && entry.scanned_length == entry.signature.length
        {
            let value = latest_custom_title_since(path, entry.scanned_length)
                .or_else(|| entry.value.clone());
            cache.titles.insert(
                path.to_path_buf(),
                CachedTitle {
                    signature,
                    scanned_length: complete_length(path, signature.length),
                    value: value.clone(),
                },
            );
            return value;
        }
    }
    let value = latest_custom_title(path);
    cache.titles.insert(
        path.to_path_buf(),
        CachedTitle {
            signature,
            scanned_length: complete_length(path, signature.length),
            value: value.clone(),
        },
    );
    value
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

fn latest_custom_title(path: &Path) -> Option<String> {
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
            if let Some(title) = custom_title(line) {
                return Some(title);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    custom_title(&suffix)
}

fn latest_custom_title_since(path: &Path, offset: u64) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut appended = Vec::new();
    file.read_to_end(&mut appended).ok()?;
    appended
        .split(|byte| *byte == b'\n')
        .filter_map(custom_title)
        .next_back()
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

fn custom_title(line: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("custom-title") {
        return None;
    }
    value
        .get("customTitle")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
}
