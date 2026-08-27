use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::Result;

use crate::aliases::AliasMap;
use crate::ask::{doc_refs, notes_refs};
use crate::retrieval::cache;
use crate::store::{
    dedupe_user_units, is_boilerplate_unit, Note, NotesLayer, Store, Unit, UnitsLayer,
};

pub struct WarmState {
    store: Store,
    aliases: AliasMap,
    keys: crate::ingest::KeySet,
    layers_cache: Option<CachedLayers>,
}

struct CachedLayers {
    fingerprint: LayerFingerprint,
    units: UnitsLayer,
    notes: NotesLayer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LayerFingerprint {
    units_len: u64,
    units_mtime_ms: u64,
    notes_run: Option<String>,
}

impl WarmState {
    pub fn open(store: Store, aliases: AliasMap) -> Result<WarmState> {
        let keys = crate::ingest::KeySet::load(&store)?;
        Ok(WarmState {
            store,
            aliases,
            keys,
            layers_cache: None,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn aliases(&self) -> &AliasMap {
        &self.aliases
    }

    pub fn keys(&mut self) -> &mut crate::ingest::KeySet {
        &mut self.keys
    }

    pub fn layers(&mut self) -> Result<(&UnitsLayer, &NotesLayer)> {
        self.refresh_layers()?;
        let cache = self.layers_cache.as_ref().expect("layers cache is fresh");
        Ok((&cache.units, &cache.notes))
    }

    pub(crate) fn views(&mut self) -> Result<(&Store, &AliasMap, &UnitsLayer, &NotesLayer)> {
        self.refresh_layers()?;
        let cache = self.layers_cache.as_ref().expect("layers cache is fresh");
        Ok((&self.store, &self.aliases, &cache.units, &cache.notes))
    }

    pub fn push_units(&mut self, units: &[serde_json::Value]) {
        if self.layers_cache.is_none() {
            return;
        }
        let mut parsed = Vec::with_capacity(units.len());
        for value in units {
            match serde_json::from_value::<Unit>(value.clone()) {
                Ok(unit) => parsed.push(unit),
                Err(_) => {
                    self.invalidate_layers();
                    return;
                }
            }
        }
        if let Some(cache) = self.layers_cache.as_mut() {
            cache.units.items.extend(parsed);
            cache.fingerprint = layer_fingerprint(&self.store);
        }
    }

    pub fn invalidate_layers(&mut self) {
        self.layers_cache = None;
    }

    fn refresh_layers(&mut self) -> Result<()> {
        let fingerprint = layer_fingerprint(&self.store);
        if self
            .layers_cache
            .as_ref()
            .is_some_and(|cache| cache.fingerprint == fingerprint)
        {
            return Ok(());
        }
        let units = self.store.read_units()?;
        let notes = self.store.read_notes()?;
        self.layers_cache = Some(CachedLayers {
            fingerprint,
            units,
            notes,
        });
        Ok(())
    }
}

fn layer_fingerprint(store: &Store) -> LayerFingerprint {
    let (units_len, units_mtime_ms) = store
        .units_path()
        .metadata()
        .map(|meta| (meta.len(), mtime_millis(&meta)))
        .unwrap_or((0, 0));
    LayerFingerprint {
        units_len,
        units_mtime_ms,
        notes_run: newest_notes_run(store),
    }
}

fn newest_notes_run(store: &Store) -> Option<String> {
    let mut runs: Vec<String> = std::fs::read_dir(store.notes_root())
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| is_run_dir_name(name))
        .collect();
    runs.sort();
    runs.pop()
}

fn is_run_dir_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 11
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[10] == b'T'
}

fn mtime_millis(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn reindex(store: &Store) -> Result<Vec<String>> {
    remove_index_files(store.root());
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    let user_input: Vec<Unit> = units
        .items
        .iter()
        .filter(|unit| unit.kind == "user")
        .cloned()
        .collect();
    let user_units = dedupe_user_units(&user_input);
    let pool_units: Vec<Unit> = user_units
        .iter()
        .filter(|unit| !is_boilerplate_unit(unit))
        .cloned()
        .collect();
    let note_items: Vec<Note> = notes.items.clone();
    cache::build_or_load(
        store.root(),
        "pool",
        &doc_refs(&pool_units),
        Some(&units.path),
    );
    cache::build_or_load(
        store.root(),
        "user",
        &doc_refs(&user_units),
        Some(&units.path),
    );
    cache::build_or_load(store.root(), "notes", &notes_refs(&note_items), None);
    Ok(vec![
        "pool".to_string(),
        "user".to_string(),
        "notes".to_string(),
    ])
}

fn remove_index_files(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        let is_index_file =
            name.starts_with("idx-") && (name.ends_with(".json") || name.ends_with(".meta"));
        if is_index_file {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_store(tag: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "qol-memory-warm-{tag}-{}-{id}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp store");
        dir
    }

    fn unit_value(key: &str, text: &str) -> serde_json::Value {
        json!({
            "key": key,
            "source": "pi",
            "file": "a.jsonl",
            "session": "sess-1",
            "cwd": "/repo",
            "kind": "user",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": text
        })
    }

    fn write_units_file(root: &Path, lines: &[serde_json::Value]) {
        let body = lines
            .iter()
            .map(|value| serde_json::to_string(value).expect("serialize"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("units.jsonl"), format!("{body}\n")).expect("write units");
    }

    #[test]
    fn warm_layers_refresh_after_units_change() {
        let root = temp_store("refresh");
        let first = unit_value("u-1", "first settled fact about the daemon");
        write_units_file(&root, std::slice::from_ref(&first));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let mut warm = WarmState::open(store, AliasMap::default()).expect("warm opens");

        let (units, _) = warm.layers().expect("layers load");
        assert_eq!(units.items.len(), 1);

        let second = unit_value("u-2", "second settled fact about the watcher");
        write_units_file(&root, &[first, second]);

        let (units, _) = warm.layers().expect("layers refresh");
        assert_eq!(units.items.len(), 2);
        assert_eq!(units.items[1].key, "u-2");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn push_units_keeps_cache_hit() {
        let root = temp_store("push-hit");
        let first = unit_value("u-1", "first settled fact about the daemon");
        write_units_file(&root, std::slice::from_ref(&first));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let mut warm = WarmState::open(store, AliasMap::default()).expect("warm opens");
        warm.layers().expect("layers load");

        let second = unit_value("u-2", "second settled fact about the watcher");
        write_units_file(&root, &[first, second.clone()]);
        warm.push_units(&[second]);

        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.units.items.len(), 2);
        assert_eq!(cache.fingerprint, layer_fingerprint(warm.store()));

        let (units, _) = warm.layers().expect("layers hit");
        assert_eq!(units.items.len(), 2);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn push_units_with_unparseable_value_invalidates() {
        let root = temp_store("push-bad");
        let first = unit_value("u-1", "first settled fact about the daemon");
        write_units_file(&root, &[first]);
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let mut warm = WarmState::open(store, AliasMap::default()).expect("warm opens");
        warm.layers().expect("layers load");

        warm.push_units(&[json!({ "key": 7 })]);

        assert!(warm.layers_cache.is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reindex_rebuilds_three_layers() {
        let root = temp_store("reindex");
        let lines = [
            unit_value("u-1", "first settled fact about the daemon"),
            unit_value("u-2", "second settled fact about the watcher"),
        ];
        write_units_file(&root, &lines);
        let run_dir = root.join("notes").join("2026-08-05T10-00-00-000Z");
        std::fs::create_dir_all(&run_dir).expect("notes run dir");
        std::fs::write(
            run_dir.join("notes.jsonl"),
            "{\"key\":\"n-1\",\"cls\":\"decision\",\"text\":\"Decision: keep the daemon warm\"}\n",
        )
        .expect("write notes");
        let store = Store::resolve(Some(&root)).expect("store resolves");

        let layers = reindex(&store).expect("first reindex");
        assert_eq!(layers, vec!["pool", "user", "notes"]);
        assert!(root.join("idx-pool.json").exists());
        assert!(root.join("idx-pool.json.meta").exists());
        assert!(root.join("idx-user.json").exists());
        assert!(root.join("idx-notes.json").exists());
        let stale_meta = "{\"fingerprint\":\"stale\"}";
        std::fs::write(root.join("idx-pool.json.meta"), stale_meta).expect("stale meta");

        let layers = reindex(&store).expect("second reindex");
        assert_eq!(layers, vec!["pool", "user", "notes"]);
        let meta = std::fs::read_to_string(root.join("idx-pool.json.meta")).expect("rebuilt meta");
        assert_ne!(meta, stale_meta);

        std::fs::remove_dir_all(&root).ok();
    }
}
