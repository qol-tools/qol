use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::store::Store;

pub const MIN_KEY_PREFIX: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Supersede,
    Forget,
    Restore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub ts: String,
    pub op: Op,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_home: Option<String>,
}

impl Record {
    fn new(op: Op, key: &str, ts: &str, agent_home: &str) -> Record {
        Record {
            ts: ts.to_owned(),
            op,
            key: key.to_owned(),
            by: None,
            reason: None,
            agent_home: Some(agent_home.to_owned()),
        }
    }

    fn is_well_formed(&self) -> bool {
        !self.key.is_empty()
            && match self.op {
                Op::Supersede => self
                    .by
                    .as_deref()
                    .is_some_and(|by| !by.is_empty() && by != self.key),
                Op::Forget | Op::Restore => true,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Superseded { by: String, ts: String },
    Forgotten { reason: String, ts: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub records: usize,
    pub superseded: usize,
    pub forgotten: usize,
    pub unreadable: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Lifecycle {
    states: HashMap<String, State>,
    records: usize,
    unreadable: usize,
    read_error: Option<String>,
}

impl Lifecycle {
    pub fn load(store: &Store) -> Lifecycle {
        match std::fs::read(store.lifecycle_path()) {
            Ok(raw) => Lifecycle::parse(&String::from_utf8_lossy(&raw)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Lifecycle::default(),
            Err(error) => {
                log::warn!("qol-memory lifecycle log unreadable: {error}");
                Lifecycle {
                    read_error: Some(error.to_string()),
                    ..Lifecycle::default()
                }
            }
        }
    }

    pub fn parse(text: &str) -> Lifecycle {
        let mut lifecycle = Lifecycle::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str::<Record>(line) {
                Ok(record) if record.is_well_formed() => lifecycle.apply(record),
                _ => lifecycle.unreadable += 1,
            }
        }
        lifecycle
    }

    fn apply(&mut self, record: Record) {
        self.records += 1;
        match record.op {
            Op::Supersede => {
                let by = record.by.unwrap_or_default();
                self.states.remove(&by);
                self.states
                    .insert(record.key, State::Superseded { by, ts: record.ts });
            }
            Op::Forget => {
                self.states.insert(
                    record.key,
                    State::Forgotten {
                        reason: record.reason.unwrap_or_default(),
                        ts: record.ts,
                    },
                );
            }
            Op::Restore => {
                self.states.remove(&record.key);
            }
        }
    }

    pub fn state(&self, key: &str) -> Option<&State> {
        self.states.get(key)
    }

    pub fn is_live(&self, key: &str) -> bool {
        !self.states.contains_key(key)
    }

    pub fn is_forgotten(&self, key: &str) -> bool {
        matches!(self.states.get(key), Some(State::Forgotten { .. }))
    }

    pub fn live_replacement(&self, key: &str) -> Option<&str> {
        let mut current = match self.states.get(key)? {
            State::Superseded { by, .. } => by.as_str(),
            State::Forgotten { .. } => return None,
        };
        for _ in 0..=self.states.len() {
            match self.states.get(current) {
                None => return Some(current),
                Some(State::Superseded { by, .. }) => current = by,
                Some(State::Forgotten { .. }) => return None,
            }
        }
        None
    }

    pub fn non_live(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn read_error(&self) -> Option<&str> {
        self.read_error.as_deref()
    }

    pub fn counts(&self) -> Counts {
        let superseded = self
            .states
            .values()
            .filter(|state| matches!(state, State::Superseded { .. }))
            .count();
        Counts {
            records: self.records,
            superseded,
            forgotten: self.states.len() - superseded,
            unreadable: self.unreadable,
        }
    }
}

pub struct Catalog {
    keys: Vec<String>,
}

impl Catalog {
    pub fn new(keys: impl IntoIterator<Item = String>) -> Catalog {
        let mut keys: Vec<String> = keys.into_iter().collect();
        keys.sort();
        keys.dedup();
        Catalog { keys }
    }

    pub fn resolve(&self, raw: &str) -> Result<Vec<String>> {
        let mut resolved: Vec<String> = Vec::new();
        let mut problems = Vec::new();
        for token in raw
            .split(|character: char| character.is_whitespace() || character == ',')
            .filter(|token| !token.is_empty())
        {
            let wanted = token.to_ascii_lowercase();
            if wanted.len() < MIN_KEY_PREFIX || !wanted.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                problems.push(format!(
                    "`{token}` is not a key; pass the key or its first {MIN_KEY_PREFIX} or more characters"
                ));
                continue;
            }
            let start = self
                .keys
                .partition_point(|key| key.as_str() < wanted.as_str());
            let matches: Vec<&String> = self.keys[start..]
                .iter()
                .take_while(|key| key.starts_with(wanted.as_str()))
                .collect();
            let chosen = matches
                .iter()
                .find(|key| key.as_str() == wanted)
                .or_else(|| matches.first().filter(|_| matches.len() == 1));
            match chosen {
                Some(key) => {
                    if !resolved.contains(*key) {
                        resolved.push((*key).clone());
                    }
                }
                None if matches.is_empty() => {
                    problems.push(format!("no fact you can see has key `{token}`"))
                }
                None => problems.push(format!(
                    "`{token}` matches {} keys ({}); pass more characters",
                    matches.len(),
                    matches
                        .iter()
                        .take(4)
                        .map(|key| key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        if !problems.is_empty() {
            bail!("nothing changed: {}", problems.join("; "));
        }
        if resolved.is_empty() {
            bail!("nothing changed: no keys given");
        }
        Ok(resolved)
    }
}

pub fn forget(store: &Store, keys: &[String], reason: &str, agent_home: &str) -> Result<()> {
    let ts = crate::text::now_iso();
    let records: Vec<Record> = keys
        .iter()
        .map(|key| Record {
            reason: Some(reason.to_owned()),
            ..Record::new(Op::Forget, key, &ts, agent_home)
        })
        .collect();
    append(store, &records)
}

pub fn supersede(store: &Store, keys: &[String], by: &str, agent_home: &str) -> Result<()> {
    if keys.iter().any(|key| key == by) {
        bail!("nothing changed: the new fact is the same capture as `{by}`");
    }
    let ts = crate::text::now_iso();
    let records: Vec<Record> = keys
        .iter()
        .map(|key| Record {
            by: Some(by.to_owned()),
            ..Record::new(Op::Supersede, key, &ts, agent_home)
        })
        .collect();
    append(store, &records)
}

pub struct Replaced {
    pub appended: usize,
    pub unit: serde_json::Value,
    pub key: String,
}

pub fn supersede_with_capture(
    store: &Store,
    keys: &[String],
    text: &str,
    cwd: &str,
    agent_home: &str,
    known: &mut crate::ingest::KeySet,
) -> Result<Replaced> {
    let mut unit = crate::ingest::capture_unit(text, cwd, &crate::text::now_iso());
    if let Some(fields) = unit.as_object_mut() {
        fields.insert("agent_home".to_owned(), serde_json::json!(agent_home));
    }
    let key = unit
        .get("key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if keys.contains(&key) {
        bail!("nothing changed: the new fact is the same capture as `{key}`");
    }
    let appended = crate::ingest::append_units(store, std::slice::from_ref(&unit), known)?;
    supersede(store, keys, &key, agent_home)?;
    Ok(Replaced {
        appended,
        unit,
        key,
    })
}

pub fn restore(store: &Store, key: &str, agent_home: &str) -> Result<()> {
    append(
        store,
        &[Record::new(
            Op::Restore,
            key,
            &crate::text::now_iso(),
            agent_home,
        )],
    )
}

fn append(store: &Store, records: &[Record]) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let mut payload = String::new();
    for record in records {
        payload.push_str(
            &serde_json::to_string(record).context("failed to encode a lifecycle record")?,
        );
        payload.push('\n');
    }
    std::fs::create_dir_all(store.root())
        .with_context(|| format!("failed to create {}", store.root().display()))?;
    let path = store.lifecycle_path();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    if ends_mid_line(&mut file).with_context(|| format!("failed to read {}", path.display()))? {
        payload.insert(0, '\n');
    }
    file.write_all(payload.as_bytes())
        .with_context(|| format!("failed to append to {}", path.display()))
}

fn ends_mid_line(file: &mut std::fs::File) -> std::io::Result<bool> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(len - 1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-lifecycle-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn store(&self) -> Store {
            Store::resolve(Some(&self.0)).unwrap()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn line(op: &str, key: &str, by: Option<&str>) -> String {
        let mut record =
            serde_json::json!({"ts": "2026-10-20T10:00:00.000Z", "op": op, "key": key});
        if let Some(by) = by {
            record["by"] = serde_json::json!(by);
        }
        record.to_string()
    }

    fn replay(lines: &[String]) -> Lifecycle {
        Lifecycle::parse(&lines.join("\n"))
    }

    #[test]
    fn replay_applies_the_latest_line_for_each_key() {
        let cases: Vec<(&str, Vec<String>, &str, Option<&str>)> = vec![
            ("empty log", vec![], "a", None),
            (
                "supersede",
                vec![line("supersede", "a", Some("b"))],
                "a",
                Some("superseded"),
            ),
            (
                "forget",
                vec![line("forget", "a", None)],
                "a",
                Some("forgotten"),
            ),
            (
                "restore after forget",
                vec![line("forget", "a", None), line("restore", "a", None)],
                "a",
                None,
            ),
            (
                "forget after supersede",
                vec![line("supersede", "a", Some("b")), line("forget", "a", None)],
                "a",
                Some("forgotten"),
            ),
            (
                "supersede makes the replacement live",
                vec![line("forget", "b", None), line("supersede", "a", Some("b"))],
                "b",
                None,
            ),
            (
                "forgetting a replacement keeps the old fact replaced",
                vec![line("supersede", "a", Some("b")), line("forget", "b", None)],
                "a",
                Some("superseded"),
            ),
        ];
        for (name, lines, key, expected) in cases {
            let lifecycle = replay(&lines);
            let state = lifecycle.state(key).map(|state| match state {
                State::Superseded { .. } => "superseded",
                State::Forgotten { .. } => "forgotten",
            });
            assert_eq!(state, expected, "{name}");
            assert_eq!(lifecycle.is_live(key), expected.is_none(), "{name}");
        }
    }

    #[test]
    fn live_replacement_follows_the_chain_to_a_live_key() {
        let cases: Vec<(&str, Vec<String>, Option<&str>)> = vec![
            ("live key has none", vec![], None),
            (
                "direct replacement",
                vec![line("supersede", "a", Some("b"))],
                Some("b"),
            ),
            (
                "chain",
                vec![
                    line("supersede", "a", Some("b")),
                    line("supersede", "b", Some("c")),
                ],
                Some("c"),
            ),
            (
                "chain ending at a forgotten key",
                vec![
                    line("supersede", "a", Some("b")),
                    line("supersede", "b", Some("c")),
                    line("forget", "c", None),
                ],
                None,
            ),
            (
                "superseding the replacement back makes the first key live",
                vec![
                    line("supersede", "a", Some("b")),
                    line("supersede", "b", Some("a")),
                ],
                None,
            ),
            (
                "forgotten key has none",
                vec![line("forget", "a", None)],
                None,
            ),
        ];
        for (name, lines, expected) in cases {
            assert_eq!(replay(&lines).live_replacement("a"), expected, "{name}");
        }
    }

    #[test]
    fn live_replacement_stops_on_a_hand_edited_cycle() {
        let mut lifecycle = Lifecycle::default();
        for (key, by) in [("a", "b"), ("b", "c"), ("c", "a")] {
            lifecycle.states.insert(
                key.to_owned(),
                State::Superseded {
                    by: by.to_owned(),
                    ts: String::new(),
                },
            );
        }
        assert_eq!(lifecycle.live_replacement("a"), None);
    }

    #[test]
    fn unreadable_lines_are_counted_and_skipped() {
        let lines = vec![
            line("forget", "a", None),
            "{not json".to_owned(),
            line("supersede", "b", None),
            line("supersede", "c", Some("c")),
            r#"{"ts":"t","op":"rename","key":"d"}"#.to_owned(),
            line("forget", "e", None),
        ];
        let counts = replay(&lines).counts();
        assert_eq!(
            counts,
            Counts {
                records: 2,
                superseded: 0,
                forgotten: 2,
                unreadable: 4,
            }
        );
    }

    #[test]
    fn catalog_resolves_full_keys_and_unique_prefixes() {
        let catalog = Catalog::new(
            [
                "72ed4718aaaaaaaa",
                "72ed4718bbbbbbbb",
                "af919076cccccccc",
                "2e0e3996dddddddd",
            ]
            .map(str::to_owned),
        );
        let cases: Vec<(&str, std::result::Result<Vec<&str>, &str>)> = vec![
            ("af919076", Ok(vec!["af919076cccccccc"])),
            ("AF919076", Ok(vec!["af919076cccccccc"])),
            ("72ed4718aaaaaaaa", Ok(vec!["72ed4718aaaaaaaa"])),
            (
                "af919076, 2e0e3996dddddddd af919076",
                Ok(vec!["af919076cccccccc", "2e0e3996dddddddd"]),
            ),
            ("72ed4718", Err("matches 2 keys")),
            ("af91907", Err("is not a key")),
            ("zz919076", Err("is not a key")),
            ("deadbeef", Err("no fact you can see has key `deadbeef`")),
            ("af919076 deadbeef", Err("no fact you can see")),
            ("  , ", Err("no keys given")),
        ];
        for (raw, expected) in cases {
            match (catalog.resolve(raw), expected) {
                (Ok(keys), Ok(want)) => assert_eq!(keys, want, "{raw}"),
                (Err(error), Err(want)) => {
                    let message = format!("{error:#}");
                    assert!(message.starts_with("nothing changed: "), "{raw}: {message}");
                    assert!(message.contains(want), "{raw}: {message}");
                }
                (got, want) => panic!("{raw}: got {got:?}, want {want:?}"),
            }
        }
    }

    #[test]
    fn operations_append_records_that_replay_to_the_same_states() {
        let dir = TempDir::new("ops");
        let store = dir.store();
        supersede(&store, &["a".to_owned(), "b".to_owned()], "c", "home").unwrap();
        forget(&store, &["d".to_owned()], "wrong path", "home").unwrap();
        restore(&store, "d", "home").unwrap();
        forget(&store, &["e".to_owned()], "stale", "home").unwrap();
        let lifecycle = Lifecycle::load(&store);
        assert_eq!(lifecycle.live_replacement("a"), Some("c"));
        assert_eq!(lifecycle.live_replacement("b"), Some("c"));
        assert!(lifecycle.is_live("d"));
        assert!(
            matches!(lifecycle.state("e"), Some(State::Forgotten { reason, .. }) if reason == "stale")
        );
        assert_eq!(lifecycle.counts().records, 5);
    }

    #[test]
    fn supersede_refuses_a_replacement_that_is_one_of_the_keys() {
        let dir = TempDir::new("self");
        let store = dir.store();
        let error = supersede(&store, &["a".to_owned()], "a", "home").unwrap_err();
        assert!(format!("{error:#}").contains("same capture"));
        assert!(!store.lifecycle_path().exists());
    }

    #[test]
    fn append_starts_a_new_line_after_a_torn_write() {
        let dir = TempDir::new("torn");
        let store = dir.store();
        std::fs::write(
            store.lifecycle_path(),
            line("forget", "a", None) + "\n{\"ts\":",
        )
        .unwrap();
        forget(&store, &["b".to_owned()], "stale", "home").unwrap();
        let lifecycle = Lifecycle::load(&store);
        assert!(lifecycle.is_forgotten("a"));
        assert!(lifecycle.is_forgotten("b"));
        assert_eq!(lifecycle.counts().unreadable, 1);
    }

    #[test]
    fn missing_log_loads_empty() {
        let dir = TempDir::new("missing");
        let lifecycle = Lifecycle::load(&dir.store());
        assert!(lifecycle.is_empty());
        assert_eq!(lifecycle.read_error(), None);
        assert_eq!(lifecycle.counts(), Counts::default());
    }
}
