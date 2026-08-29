use std::collections::HashSet;

use anyhow::{anyhow, Result};
use qol_fs::atomic_write;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::lock::DistillLock;
use crate::store::{Store, Unit};

pub mod sections;

const DECISION_CLS: &str = "decision";
const DECISION_SOURCE_KIND: &str = "decision-deter";
const BUSY_MESSAGE: &str = "qol-memory: distill busy";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DistillReport {
    pub run: Option<String>,
    pub unchanged: bool,
    pub compactions: usize,
    pub carried: usize,
    pub added: usize,
    pub dropped: usize,
}

pub fn run(store: &Store) -> Result<DistillReport> {
    let started_at = crate::text::now_iso();
    let units = store.read_units()?;
    let compaction_units: Vec<&Unit> = units
        .items
        .iter()
        .filter(|unit| crate::store::is_compaction_unit(unit))
        .collect();
    let mut new_notes: Vec<Value> = Vec::new();
    for unit in &compaction_units {
        for text in sections::claim_lines(&unit.text) {
            new_notes.push(new_note(unit, &text));
        }
    }
    let existing = store.read_notes()?;
    let carried: Vec<Value> = existing
        .items
        .iter()
        .filter(|note| {
            note.cls == DECISION_CLS && note.source_kind.as_deref() != Some(DECISION_SOURCE_KIND)
        })
        .filter_map(|note| serde_json::to_value(note).ok())
        .collect();
    let dropped = existing
        .items
        .iter()
        .filter(|note| note.cls != DECISION_CLS)
        .count();

    let mut merged: Vec<Value> = Vec::with_capacity(carried.len() + new_notes.len());
    let mut seen: HashSet<String> = HashSet::new();
    let mut added = 0usize;
    for note in &carried {
        if insert_key(&mut seen, note) {
            merged.push(note.clone());
        }
    }
    for note in &new_notes {
        if insert_key(&mut seen, note) {
            merged.push(note.clone());
            added += 1;
        }
    }
    merged.sort_by_key(sort_key);

    if dropped == 0 {
        if let Some(newest) = &existing.run {
            let newest_keys: HashSet<String> =
                existing.items.iter().map(|note| note.key.clone()).collect();
            if newest_keys == seen {
                return Ok(DistillReport {
                    run: Some(newest.clone()),
                    unchanged: true,
                    compactions: compaction_units.len(),
                    carried: carried.len(),
                    added: 0,
                    dropped: 0,
                });
            }
        }
    }

    let _lock = DistillLock::acquire(store, "distill")?.ok_or_else(|| anyhow!(BUSY_MESSAGE))?;
    let name = crate::text::now_iso();
    let notes_root = store.notes_root();
    let tmp = notes_root.join(format!(".tmp-{name}"));
    std::fs::create_dir_all(&tmp)?;
    let mut body = String::new();
    for note in &merged {
        body.push_str(&serde_json::to_string(note)?);
        body.push('\n');
    }
    atomic_write(&tmp.join("notes.jsonl"), body.as_bytes())?;
    let report = json!({
        "name": "qol-memory notes (deterministic distill)",
        "schemaVersion": 2,
        "started_at": started_at,
        "finished_at": crate::text::now_iso(),
        "status": "pass",
        "inputs": {
            "compactions": compaction_units.len(),
            "carried": carried.len(),
        },
        "stats": {
            "added": added,
            "carried": carried.len(),
            "dropped": dropped,
        },
        "commands": ["qol-memory distill"],
    });
    atomic_write(
        &tmp.join("report.json"),
        serde_json::to_string_pretty(&report)?.as_bytes(),
    )?;
    std::fs::rename(&tmp, notes_root.join(&name))?;
    Ok(DistillReport {
        run: Some(name),
        unchanged: false,
        compactions: compaction_units.len(),
        carried: carried.len(),
        added,
        dropped,
    })
}

pub fn is_busy(error: &anyhow::Error) -> bool {
    error.to_string() == BUSY_MESSAGE
}

pub fn note_key(text: &str) -> String {
    let digest = Sha256::digest(format!("{DECISION_CLS}|{}", normalize(text)).as_bytes());
    let mut key = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

fn normalize(text: &str) -> String {
    let mapped: String = text
        .to_lowercase()
        .chars()
        .map(|ch| match ch {
            '`' | '"' | '\'' | '(' | ')' | ',' | ';' | ':' => ' ',
            other => other,
        })
        .collect();
    crate::text::collapse_ws(&mapped)
}

fn new_note(unit: &Unit, text: &str) -> Value {
    let mut note = serde_json::Map::new();
    note.insert("key".to_string(), json!(note_key(text)));
    note.insert("cls".to_string(), json!(DECISION_CLS));
    note.insert("text".to_string(), json!(text));
    note.insert("source_key".to_string(), json!(unit.key));
    note.insert("source_ts".to_string(), json!(unit.ts));
    note.insert("source_kind".to_string(), json!(DECISION_SOURCE_KIND));
    if let Some(session) = &unit.session {
        note.insert("session".to_string(), json!(session));
    }
    Value::Object(note)
}

fn insert_key(seen: &mut HashSet<String>, note: &Value) -> bool {
    let key = note
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    seen.insert(key)
}

fn sort_key(note: &Value) -> (i64, String) {
    (
        crate::text::parse_iso_millis(note.get("source_ts").and_then(Value::as_str)),
        note.get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-distill-{}-{}-{}",
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

    fn write_units(store: &Store) {
        let unit = json!({
            "key": "cu-1",
            "source": "pi",
            "session": "sess-distill-1",
            "kind": "compaction",
            "ts": "2026-08-20T10:00:00.000Z",
            "text": "## Key Decisions\n- Ship the deterministic distill before the daemon restart ships\n"
        });
        std::fs::write(store.units_path(), format!("{unit}\n")).unwrap();
    }

    fn seed_notes(store: &Store) {
        let run = store.notes_root().join("2026-08-01T09-00-00-000Z");
        std::fs::create_dir_all(&run).unwrap();
        let decision = json!({
            "key": "carried0000000001",
            "cls": "decision",
            "text": "Carried decision note from the earlier JS run",
            "source_key": "old-unit",
            "source_ts": "2026-08-01T09:00:00.000Z",
            "source_kind": "decision"
        });
        let stale = json!({
            "key": "stale00000000001",
            "cls": "decision",
            "text": "Stale deterministic note the old parser version wrote",
            "source_key": "old-unit",
            "source_ts": "2026-08-01T09:00:00.000Z",
            "source_kind": "decision-deter"
        });
        let path_note = json!({
            "key": "pathnote00000001",
            "cls": "path",
            "text": "src/daemon/mod.rs holds the supervision loop",
            "source_key": "old-unit",
            "source_ts": "2026-08-01T09:00:00.000Z",
            "source_kind": "path"
        });
        let decision_line = serde_json::to_string(&decision).unwrap();
        let stale_line = serde_json::to_string(&stale).unwrap();
        let path_line = serde_json::to_string(&path_note).unwrap();
        let body = format!("{decision_line}\n{stale_line}\n{path_line}\n");
        std::fs::write(run.join("notes.jsonl"), body).unwrap();
    }

    #[test]
    fn note_key_matches_the_js_note_key_formula() {
        assert_eq!(note_key("Pick the daemon restart path"), "b3129b85eda05e9f");
        assert_eq!(
            note_key("Use `cargo` (build), then: done"),
            "5e326d77c0c00be7"
        );
    }

    #[test]
    fn run_distills_compactions_carries_decisions_and_drops_other_classes() {
        let dir = TempDir::new("run");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        write_units(&store);
        seed_notes(&store);

        let first = run(&store).unwrap();
        assert!(!first.unchanged);
        assert_eq!(first.compactions, 1);
        assert_eq!(first.carried, 1);
        assert_eq!(first.dropped, 1);
        assert_eq!(first.added, 1);
        let run_name = first.run.clone().unwrap();

        let notes = store.read_notes().unwrap();
        assert_eq!(notes.run.as_deref(), Some(run_name.as_str()));
        assert_eq!(notes.items.len(), 2);
        assert!(notes.items.iter().all(|note| note.cls == "decision"));
        assert!(notes
            .items
            .iter()
            .any(|note| note.key == "carried0000000001"));
        assert!(notes
            .items
            .iter()
            .any(|note| note.text.contains("deterministic distill")));
        assert!(notes
            .items
            .iter()
            .all(|note| note.key != "stale00000000001"));

        let raw_path = store.notes_root().join(&run_name).join("notes.jsonl");
        let raw = std::fs::read_to_string(raw_path).unwrap();
        assert!(raw.contains("\"session\":\"sess-distill-1\""));
        assert!(raw.contains("\"source_kind\":\"decision-deter\""));
        let report_path = store.notes_root().join(&run_name).join("report.json");
        let report_text = std::fs::read_to_string(report_path).unwrap();
        let report: serde_json::Value = serde_json::from_str(&report_text).unwrap();
        assert_eq!(report["name"], "qol-memory notes (deterministic distill)");
        assert_eq!(report["schemaVersion"], 2);
        assert_eq!(report["status"], "pass");
        assert_eq!(report["inputs"]["compactions"], 1);
        assert_eq!(report["inputs"]["carried"], 1);
        assert_eq!(report["stats"]["added"], 1);
        assert_eq!(report["stats"]["dropped"], 1);
        assert_eq!(report["commands"][0], "qol-memory distill");

        let second = run(&store).unwrap();
        assert!(second.unchanged);
        assert_eq!(second.run.as_deref(), Some(run_name.as_str()));
        assert_eq!(second.compactions, 1);
        assert_eq!(second.carried, 1);
        assert_eq!(second.added, 0);
        assert_eq!(second.dropped, 0);
        assert_eq!(
            store.read_notes().unwrap().run.as_deref(),
            Some(run_name.as_str())
        );
    }

    #[test]
    fn run_reports_busy_when_another_writer_holds_the_lock() {
        let dir = TempDir::new("busy");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        write_units(&store);
        let _held = DistillLock::acquire(&store, "test").unwrap().unwrap();
        let error = run(&store).unwrap_err();
        assert!(is_busy(&error));
        assert_eq!(error.to_string(), "qol-memory: distill busy");
    }
}
