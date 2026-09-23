use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub mod lock;
pub mod seal;

pub const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    fn new(root: PathBuf) -> Store {
        Store { root }
    }

    pub fn resolve(explicit: Option<&Path>) -> anyhow::Result<Store> {
        if let Some(path) = explicit {
            return Ok(Store::new(path.to_path_buf()));
        }
        if let Ok(env_root) = std::env::var("QOL_MEMORY_STORE") {
            if !env_root.is_empty() {
                return Ok(Store::new(env_root.into()));
            }
        }
        qol_config::data_subdir("plugins/qol-memory")
            .map(Store::new)
            .ok_or_else(|| anyhow::anyhow!("qol-memory: cannot resolve store root"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn units_path(&self) -> PathBuf {
        self.root.join("units.jsonl")
    }

    pub fn snapshot_root(&self) -> PathBuf {
        self.root.join("snapshot")
    }

    pub fn notes_root(&self) -> PathBuf {
        self.root.join("notes")
    }

    pub fn skills_index_path(&self) -> PathBuf {
        self.root.join("skills").join("index.json")
    }

    pub fn retrievals_path(&self) -> PathBuf {
        self.root.join("retrievals.jsonl")
    }

    pub fn candidates_path(&self) -> PathBuf {
        self.root.join("candidates.jsonl")
    }

    pub fn ingest_state_path(&self) -> PathBuf {
        self.root.join("ingest-state.json")
    }

    pub fn continue_marker_path(&self) -> PathBuf {
        self.root.join("continue.marker.json")
    }

    pub fn distill_lock_path(&self) -> PathBuf {
        self.root.join(".distill.lock")
    }

    pub fn read_units(&self) -> anyhow::Result<UnitsLayer> {
        let live = self.units_path();
        if live.exists() {
            let raw = std::fs::read(&live)?;
            let text = seal::try_sealed_text(&self.root, &raw)
                .unwrap_or_else(|| String::from_utf8_lossy(&raw).into_owned());
            return Ok(UnitsLayer {
                run: "live".to_string(),
                path: live,
                items: parse_units_text(&text),
            });
        }
        if !self.snapshot_root().exists() {
            return Ok(UnitsLayer {
                run: "empty".to_owned(),
                path: live,
                items: Vec::new(),
            });
        }
        let run = newest_run_name(&self.snapshot_root())
            .ok_or_else(|| anyhow::anyhow!("no runs under {}", self.snapshot_root().display()))?;
        let path = self.snapshot_root().join(&run).join("snapshot.jsonl");
        let raw = std::fs::read(&path)?;
        Ok(UnitsLayer {
            run,
            path,
            items: parse_units_text(String::from_utf8_lossy(&raw).as_ref()),
        })
    }

    pub fn read_notes(&self) -> anyhow::Result<NotesLayer> {
        let Some(run) = newest_run_name(&self.notes_root()) else {
            return Ok(NotesLayer {
                run: None,
                items: Vec::new(),
            });
        };
        let raw = std::fs::read(self.notes_root().join(&run).join("notes.jsonl"))?;
        let text = String::from_utf8_lossy(&raw);
        let mut items = Vec::new();
        for line in text.trim().split('\n').filter(|l| !l.is_empty()) {
            items.push(
                serde_json::from_str(line)
                    .map_err(|err| anyhow::anyhow!("qol-memory: invalid note line: {}", err))?,
            );
        }
        Ok(NotesLayer {
            run: Some(run),
            items,
        })
    }

    pub fn prune_notes_runs(&self, keep: usize) -> anyhow::Result<usize> {
        if keep == 0 {
            return Ok(0);
        }
        let notes_root = self.notes_root();
        let entries = match std::fs::read_dir(&notes_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut runs: Vec<String> = entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| is_run_dir_name(name))
            .filter_map(|name| name.to_str().map(str::to_owned))
            .collect();
        runs.sort();
        if runs.len() <= keep {
            return Ok(0);
        }
        let mut removed = 0usize;
        for name in &runs[..runs.len() - keep] {
            let path = notes_root.join(name);
            if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
                continue;
            }
            if std::fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
        }
        qol_runtime::probe!(
            "QOL_MEMORY_DISTILL",
            "event=prune outcome=done removed={removed} kept={}",
            runs.len() - removed
        );
        Ok(removed)
    }

    pub fn sweep_stale_temp_files(&self, older_than: Duration) -> usize {
        let mut removed = 0usize;
        for (path, is_dir) in self.stale_temp_entries(older_than) {
            let result = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if result.is_ok() {
                removed += 1;
            }
        }
        removed
    }

    pub fn count_stale_temp_files(&self, older_than: Duration) -> usize {
        self.stale_temp_entries(older_than).len()
    }

    fn stale_temp_entries(&self, older_than: Duration) -> Vec<(PathBuf, bool)> {
        let cutoff = SystemTime::now()
            .checked_sub(older_than)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut stale = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.filter_map(std::result::Result::ok) {
                if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    continue;
                }
                if !is_atomic_temp_name(&entry.file_name()) {
                    continue;
                }
                if is_older_than(&entry.path(), cutoff) {
                    stale.push((entry.path(), false));
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(self.notes_root()) {
            for entry in entries.filter_map(std::result::Result::ok) {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                if !entry.file_name().to_string_lossy().starts_with(".tmp-") {
                    continue;
                }
                if is_older_than(&entry.path(), cutoff) {
                    stale.push((entry.path(), true));
                }
            }
        }
        stale
    }
}

fn is_atomic_temp_name(name: &OsStr) -> bool {
    let Some(text) = name.to_str() else {
        return false;
    };
    let Some(middle) = text
        .strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((stem, suffix)) = middle.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty() && suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_older_than(path: &Path, cutoff: SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified < cutoff)
}

pub(crate) fn is_run_dir_name(name: &OsStr) -> bool {
    let Some(s) = name.to_str() else {
        return false;
    };
    let b = s.as_bytes();
    b.len() >= 11
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'T'
}

pub(crate) fn newest_run_name(root: &Path) -> Option<String> {
    let mut runs: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| is_run_dir_name(name))
        .filter_map(|name| name.to_str().map(str::to_owned))
        .collect();
    runs.sort();
    runs.pop()
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Unit {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug)]
pub struct UnitsLayer {
    pub run: String,
    pub path: PathBuf,
    pub items: Vec<Unit>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Note {
    pub key: String,
    #[serde(default)]
    pub cls: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_host: Option<String>,
}

pub struct NotesLayer {
    pub run: Option<String>,
    pub items: Vec<Note>,
}

pub const BRIDGE_TASK_MARKER: &str = "[qol session bridge]";

pub const BOILERPLATE_MARKERS: [&str; 5] = [
    BRIDGE_TASK_MARKER,
    "Base directory for this skill:",
    "continued from a previous conversation",
    "Review this change for security vulnerabilities",
    "qolmem:",
];

pub const CLAUDE_COMPACTION_MARKER: &str =
    "This session is being continued from a previous conversation";
pub const ANSWER_POOL_KINDS: [&str; 3] = ["user", "capture", "assistant"];
pub const CLAIM_UNIT_KINDS: [&str; 2] = ["capture", "assistant"];
pub const CLAIM_NOTE_CLS: &str = "decision";

pub fn in_answer_pool(kind: &str) -> bool {
    ANSWER_POOL_KINDS.contains(&kind)
}

pub fn is_claim_unit_kind(kind: &str) -> bool {
    CLAIM_UNIT_KINDS.contains(&kind)
}

pub fn is_claim_note(note: &Note) -> bool {
    note.cls == CLAIM_NOTE_CLS
}

pub fn is_compaction_unit(unit: &Unit) -> bool {
    unit.kind == "compaction" || unit.text.starts_with(CLAUDE_COMPACTION_MARKER)
}

pub fn dedupe_user_units(units: &[Unit]) -> Vec<Unit> {
    let mut sorted: Vec<Unit> = units.to_vec();
    sorted.sort_by_key(|unit| crate::text::parse_iso_millis(unit.ts.as_deref()));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(sorted.len());
    for unit in sorted {
        if seen.insert(dedupe_key(&unit)) {
            out.push(unit);
        }
    }
    out
}

pub(crate) fn dedupe_key(unit: &Unit) -> String {
    if unit.kind == "capture" {
        return format!("capture:{}", unit.key);
    }
    format!("text:{}", crate::text::collapse_ws_lower(&unit.text))
}

pub fn is_boilerplate_unit(unit: &Unit) -> bool {
    BOILERPLATE_MARKERS
        .iter()
        .any(|marker| unit.text.contains(marker))
}

pub fn parse_units_text<T: serde::de::DeserializeOwned>(text: &str) -> Vec<T> {
    text.split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-store-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn unit(key: &str, ts: Option<&str>, text: &str) -> Unit {
        Unit {
            key: key.to_string(),
            source: None,
            agent_home: None,
            host: None,
            file: None,
            session: None,
            cwd: None,
            kind: "user".to_string(),
            ts: ts.map(str::to_owned),
            text: text.to_string(),
        }
    }

    #[test]
    fn parse_units_text_skips_bad_lines() {
        let text = "{\"key\":\"a\"}\nnot json\n\n{\"key\":\"b\",\"text\":\"x y z w\"}\n[[[\n";
        let parsed: Vec<Unit> = parse_units_text(text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].key, "a");
        assert_eq!(parsed[1].key, "b");
    }

    #[test]
    fn dedupe_keeps_earliest_and_collapses_ws_case() {
        let units = vec![
            unit(
                "later",
                Some("2026-08-10T12:00:00.000Z"),
                "Fix the launcher",
            ),
            unit(
                "first",
                Some("2026-08-09T08:00:00.000Z"),
                "fix   The Launcher",
            ),
            unit("other", Some("2026-08-11T09:00:00.000Z"), "different fact"),
        ];
        let out = dedupe_user_units(&units);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, "first");
        assert_eq!(out[1].key, "other");
    }

    #[test]
    fn read_units_uses_live_then_snapshot_and_starts_empty() {
        let store_dir = TempDir::new("units-live");
        let store = Store::resolve(Some(store_dir.0.as_path())).unwrap();
        assert!(store.read_units().unwrap().items.is_empty());

        std::fs::write(store.units_path(), "{\"key\":\"live-1\"}\n").unwrap();
        let live = store.read_units().unwrap();
        assert_eq!(live.run, "live");
        assert_eq!(live.items.len(), 1);

        std::fs::remove_file(store.units_path()).unwrap();
        let snap_older = store.snapshot_root().join("2026-08-01T10-00-00-000Z");
        let snap_newer = store.snapshot_root().join("2026-08-05T10-00-00-000Z");
        std::fs::create_dir_all(&snap_older).unwrap();
        std::fs::create_dir_all(&snap_newer).unwrap();
        std::fs::write(snap_older.join("snapshot.jsonl"), "{\"key\":\"old\"}\n").unwrap();
        std::fs::write(
            snap_newer.join("snapshot.jsonl"),
            "junk\n{\"key\":\"new\"}\n",
        )
        .unwrap();
        let snapped = store.read_units().unwrap();
        assert_eq!(snapped.run, "2026-08-05T10-00-00-000Z");
        assert_eq!(snapped.items.len(), 1);
        assert_eq!(snapped.items[0].key, "new");

        std::fs::remove_dir_all(store.snapshot_root()).unwrap();
        assert!(store.read_units().unwrap().items.is_empty());
    }

    #[test]
    fn read_notes_picks_newest_run_and_requires_parseable_lines() {
        let store_dir = TempDir::new("notes-newest");
        let store = Store::resolve(Some(store_dir.0.as_path())).unwrap();
        let none_layer = store.read_notes().unwrap();
        assert!(none_layer.run.is_none());
        assert!(none_layer.items.is_empty());

        let run = store.notes_root().join("2026-08-07T09-30-00-000Z");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(
            run.join("notes.jsonl"),
            "\n{\"key\":\"n1\",\"cls\":\"decision\",\"text\":\"pick rust\"}\nbogus\n",
        )
        .unwrap();
        assert!(store.read_notes().is_err());

        std::fs::write(
            run.join("notes.jsonl"),
            "{\"key\":\"n1\",\"cls\":\"decision\",\"text\":\"pick rust\"}\n",
        )
        .unwrap();
        let layer = store.read_notes().unwrap();
        assert_eq!(layer.run.as_deref(), Some("2026-08-07T09-30-00-000Z"));
        assert_eq!(layer.items.len(), 1);
        assert_eq!(layer.items[0].cls, "decision");
    }

    #[test]
    fn answer_pool_accepts_user_capture_and_assistant_only() {
        assert!(in_answer_pool("user"));
        assert!(in_answer_pool("capture"));
        assert!(in_answer_pool("assistant"));
        assert!(!in_answer_pool("compaction"));
        assert!(!in_answer_pool("observation"));
        assert!(!in_answer_pool(""));
    }

    #[test]
    fn claim_and_compaction_predicates_follow_the_shared_contract() {
        assert!(is_claim_unit_kind("capture"));
        assert!(is_claim_unit_kind("assistant"));
        assert!(!is_claim_unit_kind("user"));
        assert!(!is_claim_unit_kind("compaction"));
        assert!(!is_claim_unit_kind(""));

        let mut compaction = unit("c-1", Some("2026-08-01T09:00:00.000Z"), "compaction body");
        compaction.kind = "compaction".to_string();
        assert!(is_compaction_unit(&compaction));
        assert!(is_compaction_unit(&unit(
            "c-2",
            Some("2026-08-01T09:00:00.000Z"),
            "This session is being continued from a previous conversation that ran out of context"
        )));
        assert!(!is_compaction_unit(&unit("c-3", None, "plain user unit")));

        let decision = Note {
            key: "n-1".to_string(),
            cls: "decision".to_string(),
            text: "pick rust".to_string(),
            source_key: None,
            source_ts: None,
            source_kind: None,
            source_host: None,
        };
        let path = Note {
            key: "n-2".to_string(),
            cls: "path".to_string(),
            text: "src/lib.rs".to_string(),
            source_key: None,
            source_ts: None,
            source_kind: None,
            source_host: None,
        };
        assert!(is_claim_note(&decision));
        assert!(!is_claim_note(&path));
    }

    #[test]
    fn note_serialization_omits_absent_optional_fields() {
        let decision = Note {
            key: "n-1".to_string(),
            cls: "decision".to_string(),
            text: "pick rust".to_string(),
            source_key: None,
            source_ts: None,
            source_kind: None,
            source_host: None,
        };
        let value = serde_json::to_value(&decision).expect("note serializes");
        assert_eq!(
            value,
            serde_json::json!({"key": "n-1", "cls": "decision", "text": "pick rust"})
        );
    }

    #[test]
    fn boilerplate_markers_match() {
        assert!(is_boilerplate_unit(&unit(
            "b",
            None,
            "note [qol session bridge] start"
        )));
        assert!(is_boilerplate_unit(&unit(
            "r",
            None,
            "qolmem: launcher receipt body"
        )));
        assert!(!is_boilerplate_unit(&unit("c", None, "real user fact")));
    }

    fn seed_run(store: &Store, name: &str, body: &str) {
        let run = store.notes_root().join(name);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("notes.jsonl"), body).unwrap();
    }

    fn run_names(store: &Store) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.notes_root())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| is_run_dir_name(OsStr::new(name)))
            .collect();
        names.sort();
        names
    }

    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 33
    }

    #[test]
    fn prune_notes_runs_boundary_table() {
        let names = [
            "2026-08-01T09:00:00.000Z",
            "2026-08-02T09:00:00.000Z",
            "2026-08-03T09:00:00.000Z",
            "2026-08-04T09:00:00.000Z",
            "2026-08-05T09:00:00.000Z",
        ];
        for keep in [0usize, 1, 4, 5, 6] {
            let dir = TempDir::new("prune-boundary");
            let store = Store::resolve(Some(dir.0.as_path())).unwrap();
            for (position, name) in names.iter().enumerate() {
                seed_run(&store, name, &format!("{{\"key\":\"n{position}\"}}\n"));
            }
            let expected: Vec<String> = if keep == 0 || keep >= names.len() {
                names.iter().map(|name| (*name).to_string()).collect()
            } else {
                names[names.len() - keep..]
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect()
            };
            let removed = store.prune_notes_runs(keep).unwrap();
            assert_eq!(removed, names.len() - expected.len(), "keep {keep}");
            assert_eq!(run_names(&store), expected, "keep {keep}");
            for (position, name) in names.iter().enumerate() {
                let path = store.notes_root().join(name);
                if expected.iter().any(|kept| kept.as_str() == *name) {
                    let body = std::fs::read_to_string(path.join("notes.jsonl")).unwrap();
                    assert_eq!(
                        body,
                        format!("{{\"key\":\"n{position}\"}}\n"),
                        "keep {keep}"
                    );
                } else {
                    assert!(!path.exists(), "keep {keep}");
                }
            }
        }
    }

    #[test]
    fn prune_notes_runs_never_removes_the_newest_run() {
        let dir = TempDir::new("prune-newest");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        seed_run(&store, "2026-08-05T09:00:00.000Z", "{\"key\":\"n\"}\n");
        assert_eq!(store.prune_notes_runs(1).unwrap(), 0);
        assert_eq!(store.prune_notes_runs(0).unwrap(), 0);
        assert_eq!(
            run_names(&store),
            vec!["2026-08-05T09:00:00.000Z".to_string()]
        );
    }

    #[test]
    fn prune_notes_runs_leaves_non_run_entries_and_links_alone() {
        let dir = TempDir::new("prune-non-run");
        let notes = dir.0.join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        std::fs::write(notes.join("loose.txt"), "loose\n").unwrap();
        std::fs::create_dir_all(notes.join(".tmp-2026-09-14T01:00:00.000Z")).unwrap();
        std::fs::create_dir_all(notes.join("not-a-run")).unwrap();
        std::fs::create_dir_all(notes.join("2026-9-01T")).unwrap();
        std::fs::create_dir_all(notes.join("notdigits")).unwrap();
        std::fs::create_dir_all(notes.join("2026-09-01X00")).unwrap();
        seed_run(&store, "2026-08-01T09:00:00.000Z", "{\"key\":\"n\"}\n");
        seed_run(&store, "2026-08-02T09:00:00.000Z", "{\"key\":\"n\"}\n");
        #[cfg(unix)]
        {
            let link_name = notes.join("2026-07-15T09:00:00.000Z");
            let link_target = dir.0.join("missing-target");
            std::os::unix::fs::symlink(&link_target, &link_name).unwrap();
        }

        assert_eq!(store.prune_notes_runs(1).unwrap(), 1);
        assert!(notes.join("loose.txt").exists());
        assert!(notes.join(".tmp-2026-09-14T01:00:00.000Z").exists());
        assert!(notes.join("not-a-run").exists());
        assert!(notes.join("2026-9-01T").exists());
        assert!(notes.join("notdigits").exists());
        assert!(notes.join("2026-09-01X00").exists());
        assert!(notes.join("2026-08-02T09:00:00.000Z").exists());
        assert!(!notes.join("2026-08-01T09:00:00.000Z").exists());
        #[cfg(unix)]
        {
            let link_name = notes.join("2026-07-15T09:00:00.000Z");
            let meta = std::fs::symlink_metadata(&link_name).unwrap();
            assert!(meta.file_type().is_symlink());
            assert_eq!(
                std::fs::read_link(&link_name).unwrap(),
                dir.0.join("missing-target")
            );
        }
    }

    #[test]
    fn sweep_removes_only_stale_atomic_temps() {
        let dir = TempDir::new("sweep");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        let notes = store.notes_root();
        std::fs::create_dir_all(&notes).unwrap();
        let old_time = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        let backdate = |path: &Path| {
            std::fs::File::open(path)
                .unwrap()
                .set_modified(old_time)
                .unwrap();
        };
        let stale_temp = dir.0.join(".units.jsonl.abc123.tmp");
        let fresh_temp = dir.0.join(".retrievals.jsonl.def456.tmp");
        let catchall = dir.0.join(".distill-catchall.ts");
        let lock = store.distill_lock_path();
        let units = store.units_path();
        let stale_tmp_dir = notes.join(".tmp-2026-09-14T01:00:00.000Z");
        let fresh_tmp_dir = notes.join(".tmp-fresh");
        for file in [&stale_temp, &fresh_temp, &catchall, &lock, &units] {
            std::fs::write(file, b"x").unwrap();
        }
        for directory in [&stale_tmp_dir, &fresh_tmp_dir] {
            std::fs::create_dir_all(directory).unwrap();
        }
        std::fs::write(notes.join("loose.txt"), b"x").unwrap();
        backdate(&stale_temp);
        backdate(&stale_tmp_dir);

        assert_eq!(store.count_stale_temp_files(STALE_TEMP_AGE), 2);
        assert_eq!(store.sweep_stale_temp_files(STALE_TEMP_AGE), 2);
        assert!(!stale_temp.exists());
        assert!(!stale_tmp_dir.exists());
        assert!(fresh_temp.exists());
        assert!(fresh_tmp_dir.exists());
        assert!(catchall.exists());
        assert!(lock.exists());
        assert!(units.exists());
        assert!(notes.join("loose.txt").exists());
        assert_eq!(store.count_stale_temp_files(STALE_TEMP_AGE), 0);
        assert_eq!(store.sweep_stale_temp_files(STALE_TEMP_AGE), 0);
    }

    #[test]
    fn sweep_tolerates_missing_roots() {
        let dir = TempDir::new("sweep-missing");
        let absent = dir.0.join("absent");
        let store = Store::resolve(Some(absent.as_path())).unwrap();
        assert_eq!(store.count_stale_temp_files(STALE_TEMP_AGE), 0);
        assert_eq!(store.sweep_stale_temp_files(STALE_TEMP_AGE), 0);
        let empty = TempDir::new("sweep-empty");
        let store = Store::resolve(Some(empty.0.as_path())).unwrap();
        assert_eq!(store.sweep_stale_temp_files(STALE_TEMP_AGE), 0);
    }

    #[test]
    fn prune_notes_runs_tolerates_missing_and_empty_roots() {
        let dir = TempDir::new("prune-roots");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        assert_eq!(store.prune_notes_runs(3).unwrap(), 0);
        std::fs::create_dir_all(store.notes_root()).unwrap();
        assert_eq!(store.prune_notes_runs(3).unwrap(), 0);
    }

    #[test]
    fn prune_notes_runs_is_idempotent() {
        let dir = TempDir::new("prune-idempotent");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        seed_run(&store, "2026-08-01T09:00:00.000Z", "{\"key\":\"n\"}\n");
        seed_run(&store, "2026-08-02T09:00:00.000Z", "{\"key\":\"n\"}\n");
        seed_run(&store, "2026-08-03T09:00:00.000Z", "{\"key\":\"n\"}\n");
        assert_eq!(store.prune_notes_runs(1).unwrap(), 2);
        assert_eq!(store.prune_notes_runs(1).unwrap(), 0);
    }

    #[test]
    fn prune_notes_runs_survivors_are_the_keep_greatest_names() {
        let keeps = [0usize, 1, 2, 5, 7, 10, 60];
        let mut state = 0x9E3779B97F4A7C15u64;
        for case in 0..200usize {
            let size = (lcg(&mut state) % 26) as usize;
            let keep = keeps[(lcg(&mut state) % keeps.len() as u64) as usize];
            let mut names: Vec<String> = Vec::with_capacity(size);
            for _ in 0..size {
                let month = 1 + lcg(&mut state) % 12;
                let day = 1 + lcg(&mut state) % 28;
                let hour = lcg(&mut state) % 24;
                let minute = lcg(&mut state) % 60;
                let second = lcg(&mut state) % 60;
                let millis = lcg(&mut state) % 1000;
                names.push(format!(
                    "2026-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
                ));
            }
            names.sort();
            names.dedup();
            let dir = TempDir::new("prune-sweep");
            let store = Store::resolve(Some(dir.0.as_path())).unwrap();
            std::fs::create_dir_all(store.notes_root()).unwrap();
            for name in &names {
                seed_run(&store, name, "{\"key\":\"n\"}\n");
            }
            let expected: Vec<String> = if keep == 0 || keep >= names.len() {
                names.clone()
            } else {
                names[names.len() - keep..].to_vec()
            };
            let removed = store.prune_notes_runs(keep).unwrap();
            assert_eq!(
                removed,
                names.len() - expected.len(),
                "case {case} keep {keep} size {}",
                names.len()
            );
            assert_eq!(run_names(&store), expected, "case {case} keep {keep}");
        }
    }
}
