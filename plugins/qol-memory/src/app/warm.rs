use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::Result;

use crate::aliases::AliasMap;
use crate::ask::{doc_refs, notes_refs, visible_notes, WarmIndexes};
use crate::retrieval::{build_index, cache, DocRef, Index};
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
    caller: String,
    units: UnitsLayer,
    notes: NotesLayer,
    user_units: Vec<Unit>,
    answer_pool: Vec<Unit>,
    visible_notes: Vec<Note>,
    answer: Index,
    all: Index,
    notes_index: Index,
    dedupe_seen: HashSet<String>,
    by_key: HashMap<String, usize>,
    indexed_keys: HashSet<String>,
    notes_dirty: bool,
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

    pub(crate) fn ask_views(
        &mut self,
        caller: &str,
    ) -> Result<(
        &Store,
        &AliasMap,
        &UnitsLayer,
        &NotesLayer,
        Option<WarmIndexes<'_>>,
    )> {
        self.refresh_layers()?;
        let cache = self.layers_cache.as_ref().expect("layers cache is fresh");
        let indexes = (cache.caller == caller).then_some(WarmIndexes {
            answer: &cache.answer,
            all: &cache.all,
            notes: &cache.notes_index,
            user_units: &cache.user_units,
            answer_pool: &cache.answer_pool,
            visible_notes: &cache.visible_notes,
            by_key: &cache.by_key,
        });
        Ok((
            &self.store,
            &self.aliases,
            &cache.units,
            &cache.notes,
            indexes,
        ))
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
            let registry = qol_agent_homes::Registry::load();
            extend_unit_indexes(cache, &parsed, &registry);
            cache.units.items.extend(parsed);
            let fresh = layer_fingerprint(&self.store);
            cache.fingerprint.units_len = fresh.units_len;
            cache.fingerprint.units_mtime_ms = fresh.units_mtime_ms;
        }
    }

    pub fn invalidate_layers(&mut self) {
        self.layers_cache = None;
    }

    pub fn invalidate_notes_index(&mut self) {
        if let Some(cache) = self.layers_cache.as_mut() {
            cache.notes_dirty = true;
        }
    }

    fn refresh_layers(&mut self) -> Result<()> {
        let fingerprint = layer_fingerprint(&self.store);
        if self.layers_cache.is_none() {
            self.layers_cache = Some(build_layers(&self.store, fingerprint)?);
            return Ok(());
        }
        let Some(cache) = self.layers_cache.as_ref() else {
            return Ok(());
        };
        if cache.fingerprint == fingerprint && !cache.notes_dirty {
            return Ok(());
        }
        let prev = cache.fingerprint.clone();
        let notes_changed = cache.notes_dirty || prev.notes_run != fingerprint.notes_run;
        let caller = cache.caller.clone();
        let appended = if cache.units.run == "live" && fingerprint.units_len > prev.units_len {
            read_units_tail(&self.store.units_path(), prev.units_len).ok()
        } else {
            None
        };
        if appended.is_none()
            && (fingerprint.units_len != prev.units_len
                || fingerprint.units_mtime_ms != prev.units_mtime_ms)
        {
            self.layers_cache = Some(build_layers(&self.store, fingerprint)?);
            return Ok(());
        }
        if let Some((units, consumed)) = appended {
            let registry = qol_agent_homes::Registry::load();
            let Some(cache) = self.layers_cache.as_mut() else {
                return Ok(());
            };
            extend_unit_indexes(cache, &units, &registry);
            cache.units.items.extend(units);
            cache.fingerprint.units_len = prev.units_len + consumed;
            cache.fingerprint.units_mtime_ms = fingerprint.units_mtime_ms;
        }
        if notes_changed {
            let notes = self.store.read_notes()?;
            let registry = qol_agent_homes::Registry::load();
            let Some(cache) = self.layers_cache.as_mut() else {
                return Ok(());
            };
            let visible = visible_notes(&notes.items, &cache.units.items, &caller, &registry);
            cache.notes = notes;
            cache.visible_notes = visible;
            cache.notes_index = build_index(&notes_refs(&cache.visible_notes));
            cache.fingerprint.notes_run = fingerprint.notes_run;
            cache.notes_dirty = false;
        }
        Ok(())
    }
}

fn build_layers(store: &Store, fingerprint: LayerFingerprint) -> Result<CachedLayers> {
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    let registry = qol_agent_homes::Registry::load();
    let caller = registry.resolve_caller(None);
    let user_input: Vec<Unit> = units
        .items
        .iter()
        .filter(|unit| {
            crate::store::in_answer_pool(&unit.kind)
                && crate::agent_home::visible(unit, &caller, &registry)
        })
        .cloned()
        .collect();
    let user_units = dedupe_user_units(&user_input);
    let answer_pool: Vec<Unit> = user_units
        .iter()
        .filter(|unit| !is_boilerplate_unit(unit))
        .cloned()
        .collect();
    let visible = visible_notes(&notes.items, &units.items, &caller, &registry);
    let dedupe_seen: HashSet<String> = user_units
        .iter()
        .map(|unit| crate::text::collapse_ws_lower(&unit.text))
        .collect();
    let by_key: HashMap<String, usize> = user_units
        .iter()
        .enumerate()
        .map(|(position, unit)| (unit.key.clone(), position))
        .collect();
    let indexed_keys: HashSet<String> = user_units.iter().map(|unit| unit.key.clone()).collect();
    let answer_index = build_index(&doc_refs(&answer_pool));
    let all_index = build_index(&doc_refs(&user_units));
    let notes_index = build_index(&notes_refs(&visible));
    Ok(CachedLayers {
        fingerprint,
        caller,
        units,
        notes,
        user_units,
        answer_pool,
        visible_notes: visible,
        answer: answer_index,
        all: all_index,
        notes_index,
        dedupe_seen,
        by_key,
        indexed_keys,
        notes_dirty: false,
    })
}

fn extend_unit_indexes(
    cache: &mut CachedLayers,
    units: &[Unit],
    registry: &qol_agent_homes::Registry,
) {
    for unit in units {
        if !crate::store::in_answer_pool(&unit.kind)
            || !crate::agent_home::visible(unit, &cache.caller, registry)
            || !cache
                .dedupe_seen
                .insert(crate::text::collapse_ws_lower(&unit.text))
            || !cache.indexed_keys.insert(unit.key.clone())
        {
            continue;
        }
        let refs = [DocRef {
            key: unit.key.as_str(),
            text: unit.text.as_str(),
        }];
        cache.all.extend(&refs);
        cache
            .by_key
            .insert(unit.key.clone(), cache.user_units.len());
        cache.user_units.push(unit.clone());
        if !is_boilerplate_unit(unit) {
            cache.answer.extend(&refs);
            cache.answer_pool.push(unit.clone());
        }
    }
}

fn read_units_tail(path: &Path, from: u64) -> Result<(Vec<Unit>, u64)> {
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() < from {
        return Err(anyhow::anyhow!("units file shrank while reading its tail"));
    }
    file.seek(SeekFrom::Start(from))?;
    let mut tail = String::new();
    file.read_to_string(&mut tail)?;
    let mut units = Vec::new();
    for line in tail.split('\n').filter(|line| !line.is_empty()) {
        units.push(serde_json::from_str::<Unit>(line)?);
    }
    Ok((units, tail.len() as u64))
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
    let registry = qol_agent_homes::Registry::load();
    let caller = registry.resolve_caller(None);
    let slug = crate::agent_home::cache_slug(&caller);
    let pool_layer = format!("pool-{slug}");
    let user_layer = format!("user-{slug}");
    let notes_layer = format!("notes-{slug}");
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    let user_input: Vec<Unit> = units
        .items
        .iter()
        .filter(|unit| {
            crate::store::in_answer_pool(&unit.kind)
                && crate::agent_home::visible(unit, &caller, &registry)
        })
        .cloned()
        .collect();
    let user_units = dedupe_user_units(&user_input);
    let pool_units: Vec<Unit> = user_units
        .iter()
        .filter(|unit| !is_boilerplate_unit(unit))
        .cloned()
        .collect();
    let note_items = visible_notes(&notes.items, &units.items, &caller, &registry);
    cache::build_or_load(
        store.root(),
        &pool_layer,
        &doc_refs(&pool_units),
        Some(&units.path),
    );
    cache::build_or_load(
        store.root(),
        &user_layer,
        &doc_refs(&user_units),
        Some(&units.path),
    );
    cache::build_or_load(store.root(), &notes_layer, &notes_refs(&note_items), None);
    Ok(vec![pool_layer, user_layer, notes_layer])
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
    fn warm_indexes_extend_and_rebuild_with_units_growth() {
        let root = temp_store("index-extend");
        let first = unit_value("u-1", "first settled fact about the daemon");
        let second = unit_value("u-2", "second settled fact about the watcher");
        let boiler = unit_value("u-3", "continued from a previous conversation noise");
        write_units_file(&root, std::slice::from_ref(&first));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let mut warm = WarmState::open(store, AliasMap::default()).expect("warm opens");
        warm.layers().expect("layers load");
        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.all.n, 1);
        assert_eq!(cache.answer.n, 1);
        assert!(cache.indexed_keys.contains("u-1"));

        write_units_file(&root, &[first.clone(), second.clone()]);
        warm.layers().expect("layers refresh");
        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.units.items.len(), 2);
        assert_eq!(cache.all.n, 2);
        assert_eq!(cache.answer.n, 2);
        assert!(cache.indexed_keys.contains("u-2"));

        write_units_file(&root, &[first.clone(), second.clone(), boiler.clone()]);
        warm.layers().expect("layers refresh");
        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.all.n, 3);
        assert_eq!(cache.answer.n, 2);

        write_units_file(&root, std::slice::from_ref(&first));
        warm.layers().expect("layers rebuild after shrink");
        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.units.items.len(), 1);
        assert_eq!(cache.all.n, 1);
        assert_eq!(cache.answer.n, 1);
        assert!(cache.indexed_keys.contains("u-1"));
        assert!(!cache.indexed_keys.contains("u-2"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn warm_notes_index_rebuilds_on_notes_run_change() {
        let root = temp_store("notes-run");
        write_units_file(
            &root,
            &[unit_value("u-1", "first settled fact about the daemon")],
        );
        let run = root.join("notes").join("2026-08-05T10-00-00-000Z");
        std::fs::create_dir_all(&run).expect("notes run dir");
        std::fs::write(
            run.join("notes.jsonl"),
            "{\"key\":\"n-1\",\"cls\":\"decision\",\"text\":\"Decision: keep the daemon warm\"}\n",
        )
        .expect("write notes");
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let mut warm = WarmState::open(store, AliasMap::default()).expect("warm opens");
        warm.layers().expect("layers load");
        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.notes_index.n, 1);

        let newer = root.join("notes").join("2026-08-06T10-00-00-000Z");
        std::fs::create_dir_all(&newer).expect("new notes run dir");
        std::fs::write(
            newer.join("notes.jsonl"),
            "{\"key\":\"n-1\",\"cls\":\"decision\",\"text\":\"Decision: keep the daemon warm\"}\n{\"key\":\"n-2\",\"cls\":\"decision\",\"text\":\"Decision: refresh only the notes index\"}\n",
        )
        .expect("write notes");
        warm.layers().expect("layers refresh");
        let cache = warm.layers_cache.as_ref().expect("cache present");
        assert_eq!(cache.notes_index.n, 2);
        assert_eq!(cache.all.n, 1);
        assert_eq!(cache.units.items.len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn push_units_updates_by_key_and_answer_pool() {
        let root = temp_store("push-derived");
        let first = unit_value("u-1", "first settled fact about the daemon");
        write_units_file(&root, std::slice::from_ref(&first));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let mut warm = WarmState::open(store, AliasMap::default()).expect("warm opens");
        warm.layers().expect("layers load");

        let second = unit_value("u-2", "second settled fact about the watcher");
        let boiler = unit_value("u-3", "continued from a previous conversation noise");
        write_units_file(&root, &[first, second.clone(), boiler.clone()]);
        warm.push_units(&[second, boiler]);

        let cache = warm.layers_cache.as_ref().expect("cache present");
        let second_position = *cache
            .by_key
            .get("u-2")
            .expect("by_key resolves the pushed unit");
        assert_eq!(cache.user_units[second_position].key, "u-2");
        assert_eq!(cache.user_units.len(), 3);
        assert!(cache.answer_pool.iter().any(|unit| unit.key == "u-1"));
        assert!(cache.answer_pool.iter().any(|unit| unit.key == "u-2"));
        assert!(!cache.answer_pool.iter().any(|unit| unit.key == "u-3"));

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
        let registry = qol_agent_homes::Registry::load();
        let caller = registry.resolve_caller(None);
        let slug = crate::agent_home::cache_slug(&caller);
        let pool_index = format!("idx-pool-{slug}.json");
        let pool_meta = format!("idx-pool-{slug}.json.meta");
        let user_index = format!("idx-user-{slug}.json");
        let notes_index = format!("idx-notes-{slug}.json");

        let layers = reindex(&store).expect("first reindex");
        assert_eq!(
            layers,
            vec![
                format!("pool-{slug}"),
                format!("user-{slug}"),
                format!("notes-{slug}")
            ]
        );
        assert!(root.join(&pool_index).exists());
        assert!(root.join(&pool_meta).exists());
        assert!(root.join(&user_index).exists());
        assert!(root.join(&notes_index).exists());
        let stale_meta = "{\"fingerprint\":\"stale\"}";
        std::fs::write(root.join(&pool_meta), stale_meta).expect("stale meta");

        let layers = reindex(&store).expect("second reindex");
        assert_eq!(
            layers,
            vec![
                format!("pool-{slug}"),
                format!("user-{slug}"),
                format!("notes-{slug}")
            ]
        );
        let meta = std::fs::read_to_string(root.join(&pool_meta)).expect("rebuilt meta");
        assert_ne!(meta, stale_meta);

        std::fs::remove_dir_all(&root).ok();
    }
}
