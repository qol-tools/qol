use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Signature {
    pub mtime_ns: i64,
    pub entry_count: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubtreeRecord {
    pub bytes: u64,
    pub sig: Option<Signature>,
    pub scanned_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScanLedger {
    pub records: BTreeMap<String, SubtreeRecord>,
}

impl ScanLedger {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn signature_of(path: &Path) -> Option<Signature> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let entry_count = if meta.is_dir() {
        count_entries_capped(path, 4096)
    } else {
        None
    };
    Some(Signature {
        mtime_ns: mtime_ns(&meta),
        entry_count,
    })
}

fn count_entries_capped(path: &Path, cap: u32) -> Option<u32> {
    let mut count = 0u32;
    for _ in std::fs::read_dir(path).ok()? {
        count += 1;
        if count > cap {
            return None;
        }
    }
    Some(count)
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime() * 1_000_000_000 + meta.mtime_nsec()
    }
    #[cfg(not(unix))]
    {
        let secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        secs as i64 * 1_000_000_000
    }
}

pub fn resolve_bytes(
    path: &Path,
    ledger: &mut ScanLedger,
    now_ms: u64,
    max_age_ms: u64,
    deep: &mut dyn FnMut(&Path) -> u64,
) -> (u64, bool) {
    let key = path.to_string_lossy().into_owned();
    let fresh = signature_of(path);
    let sig = match fresh {
        Some(sig) => sig,
        None => {
            ledger.records.remove(&key);
            return (0, false);
        }
    };
    if let Some(record) = ledger.records.get(&key) {
        if let Some(cached) = record.sig {
            if cached == sig && now_ms.saturating_sub(record.scanned_at_ms) <= max_age_ms {
                return (record.bytes, false);
            }
        }
    }
    let bytes = deep(path);
    ledger.records.insert(
        key,
        SubtreeRecord {
            bytes,
            sig: Some(sig),
            scanned_at_ms: now_ms,
        },
    );
    (bytes, true)
}

pub fn resolve_dir_by_children(
    path: &Path,
    ledger: &mut ScanLedger,
    now_ms: u64,
    max_age_ms: u64,
    deep: &mut dyn FnMut(&Path) -> u64,
) -> (u64, bool) {
    let prefix = path.to_string_lossy().into_owned();
    let prefix_sep = format!("{}/", prefix);
    let mut total = 0u64;
    let mut rescanned = false;
    let mut live_children = BTreeMap::new();
    let read = match std::fs::read_dir(path) {
        Ok(read) => read,
        Err(_) => return (0, false),
    };
    for entry in read.flatten() {
        let child_path = entry.path();
        let child_key = child_path.to_string_lossy().into_owned();
        live_children.insert(child_key, ());
        let meta = match std::fs::symlink_metadata(&child_path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.is_dir() {
            let (bytes, changed) = resolve_bytes(&child_path, ledger, now_ms, max_age_ms, deep);
            total += bytes;
            rescanned |= changed;
        } else {
            total += meta.len();
        }
    }
    ledger.records.retain(|key, _| {
        let Some(rest) = key.strip_prefix(&prefix_sep) else {
            return true;
        };
        let child = rest.split('/').next().unwrap_or_default();
        live_children.contains_key(&format!("{}{}", prefix_sep, child))
    });
    (total, rescanned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    fn deep_size(path: &Path, deep_calls: &std::cell::Cell<u32>) -> u64 {
        deep_calls.set(deep_calls.get() + 1);
        let mut pacer = crate::target_cache::Pacer::new();
        walkdir::WalkDir::new(path)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                pacer.tick();
                entry
                    .file_type()
                    .is_file()
                    .then(|| entry.metadata().ok().map_or(0, |meta| meta.len()))
            })
            .sum()
    }

    #[test]
    fn signature_of_counts_entries_and_caps() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            fs::write(tmp.path().join(format!("f{}", i)), b"x").unwrap();
        }
        let sig = signature_of(tmp.path()).unwrap();
        assert_eq!(sig.entry_count, Some(3));
        assert!(sig.mtime_ns > 0);

        let small = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::write(small.path().join(format!("g{}", i)), b"x").unwrap();
        }
        assert_eq!(count_entries_capped(small.path(), 3), None);
        assert_eq!(count_entries_capped(small.path(), 4), None);
        assert_eq!(count_entries_capped(small.path(), 5), Some(5));
        assert_eq!(count_entries_capped(small.path(), 10), Some(5));
        assert!(signature_of(small.path()).is_some());

        let file = small.path().join("g0");
        let file_sig = signature_of(&file).unwrap();
        assert_eq!(file_sig.entry_count, None);
        assert!(file_sig.mtime_ns > 0);

        assert!(signature_of(&small.path().join("missing")).is_none());
    }

    #[test]
    fn resolve_bytes_reuses_and_rescans() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let mut ledger = ScanLedger::new();
        let deep_calls = std::cell::Cell::new(0u32);
        let mut deep = |path: &Path| deep_size(path, &deep_calls);

        let (bytes, scanned) = resolve_bytes(tmp.path(), &mut ledger, 1_000, 10_000, &mut deep);
        assert!(scanned);
        assert_eq!(bytes, 5);
        assert_eq!(deep_calls.get(), 1);

        let (bytes, scanned) = resolve_bytes(tmp.path(), &mut ledger, 2_000, 10_000, &mut deep);
        assert!(!scanned);
        assert_eq!(bytes, 5);
        assert_eq!(deep_calls.get(), 1);

        std::thread::sleep(Duration::from_millis(10));
        fs::write(tmp.path().join("b.txt"), b"world").unwrap();
        let (bytes, scanned) = resolve_bytes(tmp.path(), &mut ledger, 3_000, 10_000, &mut deep);
        assert!(scanned);
        assert_eq!(bytes, 10);
        assert_eq!(deep_calls.get(), 2);

        let (bytes, scanned) = resolve_bytes(tmp.path(), &mut ledger, 13_001, 10_000, &mut deep);
        assert!(scanned);
        assert_eq!(bytes, 10);
        assert_eq!(deep_calls.get(), 3);

        let missing = tmp.path().join("missing");
        let (bytes, scanned) = resolve_bytes(&missing, &mut ledger, 14_000, 10_000, &mut deep);
        assert_eq!((bytes, scanned), (0, false));
        assert!(!ledger
            .records
            .contains_key(&missing.to_string_lossy().into_owned()));
    }

    #[test]
    fn resolve_dir_by_children_rescans_only_changed_child() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        fs::create_dir_all(a.join("sub")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("f1"), b"aaaa").unwrap();
        fs::write(b.join("f2"), b"bb").unwrap();
        fs::write(tmp.path().join("loose.txt"), b"loose").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let mut ledger = ScanLedger::new();
        let deep_calls = std::cell::Cell::new(0u32);
        let mut deep = |path: &Path| deep_size(path, &deep_calls);

        let (bytes, scanned) =
            resolve_dir_by_children(tmp.path(), &mut ledger, 1_000, 10_000, &mut deep);
        assert!(scanned);
        assert_eq!(bytes, 4 + 2 + 5);
        assert_eq!(deep_calls.get(), 2);

        let (bytes, scanned) =
            resolve_dir_by_children(tmp.path(), &mut ledger, 2_000, 10_000, &mut deep);
        assert!(!scanned);
        assert_eq!(bytes, 11);
        assert_eq!(deep_calls.get(), 2);

        std::thread::sleep(Duration::from_millis(10));
        fs::write(a.join("f3"), b"zzz").unwrap();
        let (bytes, scanned) =
            resolve_dir_by_children(tmp.path(), &mut ledger, 3_000, 10_000, &mut deep);
        assert!(scanned);
        assert_eq!(bytes, 4 + 3 + 2 + 5);
        assert_eq!(deep_calls.get(), 3);

        let a_key = a.to_string_lossy().into_owned();
        let b_key = b.to_string_lossy().into_owned();
        assert!(ledger.records.contains_key(&a_key));
        assert!(ledger.records.contains_key(&b_key));

        fs::remove_dir_all(&b).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let (bytes, scanned) =
            resolve_dir_by_children(tmp.path(), &mut ledger, 4_000, 10_000, &mut deep);
        assert!(!scanned);
        assert_eq!(bytes, 4 + 3 + 5);
        assert_eq!(deep_calls.get(), 3);
        assert!(!ledger.records.contains_key(&b_key));
        assert!(ledger.records.contains_key(&a_key));
    }

    #[test]
    fn ledger_serde_round_trip() {
        let mut ledger = ScanLedger::new();
        ledger.records.insert(
            "target/debug".to_string(),
            SubtreeRecord {
                bytes: 12345,
                sig: Some(Signature {
                    mtime_ns: 1_234_567_890,
                    entry_count: Some(42),
                }),
                scanned_at_ms: 999,
            },
        );
        ledger.records.insert(
            "target/debug/deps".to_string(),
            SubtreeRecord {
                bytes: 0,
                sig: None,
                scanned_at_ms: 1_000,
            },
        );
        let json = serde_json::to_string(&ledger).unwrap();
        let back: ScanLedger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ledger);
        assert!(ScanLedger::new().records.is_empty());
    }
}
