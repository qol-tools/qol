use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub mod lock;
pub mod seal;

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
}

fn is_run_dir_name(name: &OsStr) -> bool {
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

fn newest_run_name(root: &Path) -> Option<String> {
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

pub const BOILERPLATE_MARKERS: [&str; 5] = [
    "[qol session bridge]",
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
        if seen.insert(crate::text::collapse_ws_lower(&unit.text)) {
            out.push(unit);
        }
    }
    out
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
    fn read_units_uses_live_then_snapshot_and_errors_without_runs() {
        let store_dir = TempDir::new("units-live");
        let store = Store::resolve(Some(store_dir.0.as_path())).unwrap();
        assert!(store.read_units().is_err());

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
        let err = format!("{}", store.read_units().unwrap_err());
        assert_eq!(
            err,
            format!("no runs under {}", store.snapshot_root().display())
        );
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
}
