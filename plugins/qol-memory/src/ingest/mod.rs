pub mod redact;
pub mod state;
pub mod transcript;

pub use redact::redact;

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use qol_agent_homes::Registry;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::store::lock::DistillLock;
use crate::store::Store;

const DEFAULT_IGNORE: [&str; 5] = [
    "**/*secret*",
    "**/*token*",
    "**/.env",
    "**/.env.*",
    "**/memory/",
];

const MAX_DEPTH: usize = 8;

struct IgnoreRule(String, Option<Regex>);

pub const SUPPORTED_SOURCES: [&str; 2] = ["claude", "pi"];

pub struct IngestRoot {
    pub path: PathBuf,
    pub source: &'static str,
    pub agent_home: String,
}

pub struct IngestRoots {
    pub roots: Vec<IngestRoot>,
}

impl IngestRoots {
    pub fn resolve() -> IngestRoots {
        IngestRoots::from_registry(&Registry::load())
    }

    pub fn from_registry(registry: &Registry) -> IngestRoots {
        let mut roots: Vec<IngestRoot> = registry
            .transcript_roots()
            .into_iter()
            .filter(|(home, _)| SUPPORTED_SOURCES.contains(&home.harness.id()))
            .map(|(home, path)| IngestRoot {
                path,
                source: home.harness.id(),
                agent_home: home.id,
            })
            .collect();
        for source in ["pi", "claude"] {
            let Some(path) = env_root(source) else {
                continue;
            };
            let harness = match source {
                "pi" => qol_agent_homes::Harness::Pi,
                _ => qol_agent_homes::Harness::Claude,
            };
            let agent_home = registry.current(harness).id;
            roots.retain(|root| root.source != source);
            roots.push(IngestRoot {
                path,
                source,
                agent_home,
            });
        }
        IngestRoots { roots }
    }

    pub fn source_of(&self, path: &Path) -> Option<(&'static str, &str)> {
        self.roots
            .iter()
            .find(|root| path.starts_with(&root.path))
            .map(|root| (root.source, root.agent_home.as_str()))
    }
}

fn env_root(source: &str) -> Option<PathBuf> {
    let name = match source {
        "pi" => "QOL_MEMORY_PI_DIR",
        "claude" => "QOL_MEMORY_CLAUDE_DIR",
        _ => return None,
    };
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub struct KeySet {
    keys: HashSet<String>,
    fingerprint: (u64, u64),
}

impl KeySet {
    pub fn load(store: &Store) -> anyhow::Result<KeySet> {
        let fingerprint = units_fingerprint(store);
        let mut keys = HashSet::new();
        if store.units_path().exists() {
            let layer = store.read_units()?;
            for unit in layer.items {
                keys.insert(unit.key);
            }
        }
        Ok(KeySet { keys, fingerprint })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

fn units_fingerprint(store: &Store) -> (u64, u64) {
    match std::fs::metadata(store.units_path()) {
        Ok(metadata) => (metadata.len(), mtime_millis(&metadata)),
        Err(_) => (0, 0),
    }
}

pub const ASSISTANT_KIND: &str = "assistant";
pub const COMPACTION_KIND: &str = "compaction";
pub const PARSER_VERSION: u32 = 2;

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IngestReport {
    pub files: usize,
    pub appended: usize,
    pub duplicates: usize,
    pub reparsed: usize,
    pub compactions: usize,
}

pub fn unit_key(source: &str, file: &str, ts: Option<&str>, text: &str) -> String {
    let joined = [source, file, ts.unwrap_or(""), text].join("|");
    let digest = Sha256::digest(joined.as_bytes());
    let mut key = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

pub const CAPTURE_SOURCE: &str = "agent";
pub const CAPTURE_KIND: &str = "capture";

pub fn capture_unit(text: &str, cwd: &str, ts: &str) -> serde_json::Value {
    serde_json::json!({
        "key": unit_key(CAPTURE_SOURCE, cwd, None, text),
        "source": CAPTURE_SOURCE,
        "cwd": cwd,
        "kind": CAPTURE_KIND,
        "ts": ts,
        "text": text
    })
}

pub fn is_ignored(roots: &IngestRoots, path: &Path) -> bool {
    let lossy = path.to_string_lossy();
    let raw: &str = lossy.as_ref();
    let mut rels: Vec<String> = vec![raw.to_string()];
    for root in &roots.roots {
        let root_lossy = root.path.to_string_lossy();
        let root_str: &str = root_lossy.as_ref();
        if !root_str.is_empty() && raw.starts_with(root_str) {
            rels.push(format!("~{}{}", root.source, &raw[root_str.len()..]));
        }
    }
    let normalized: Vec<String> = rels.iter().map(|rel| rel.replace('\\', "/")).collect();
    ignore_rules().iter().any(|rule| match rule {
        IgnoreRule(_, Some(built)) => {
            rels.iter().any(|rel| built.is_match(rel))
                || normalized.iter().any(|rel| built.is_match(rel))
        }
        IgnoreRule(text, None) => {
            rels.iter().any(|rel| rel.contains(text))
                || normalized.iter().any(|rel| rel.contains(text))
        }
    })
}

fn ignore_rules() -> &'static Vec<IgnoreRule> {
    static RULES: OnceLock<Vec<IgnoreRule>> = OnceLock::new();
    RULES.get_or_init(load_ignore_rules)
}

fn load_ignore_rules() -> Vec<IgnoreRule> {
    let mut rules: Vec<IgnoreRule> = DEFAULT_IGNORE
        .iter()
        .map(|text| IgnoreRule((*text).to_string(), compile_rule(text)))
        .collect();
    if let Ok(store) = Store::resolve(None) {
        if let Ok(text) = std::fs::read_to_string(store.root().join("ignore")) {
            for line in text.split('\n') {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    rules.push(IgnoreRule(trimmed.to_string(), compile_rule(trimmed)));
                }
            }
        }
    }
    rules
}

fn compile_rule(rule: &str) -> Option<Regex> {
    if !rule.contains('*') {
        return None;
    }
    let pattern = format!(
        "^{}$",
        rule.split('*')
            .map(escape_regex)
            .collect::<Vec<_>>()
            .join(".*")
    );
    Regex::new(&pattern).ok()
}

fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ".*+?^${}()|[]\\".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn append_units(
    store: &Store,
    units: &[serde_json::Value],
    keys: &mut KeySet,
) -> anyhow::Result<usize> {
    append_units_inner(store, units, keys).map(|(appended, _)| appended)
}

fn append_units_inner(
    store: &Store,
    units: &[serde_json::Value],
    keys: &mut KeySet,
) -> anyhow::Result<(usize, usize)> {
    let candidates: Vec<&serde_json::Value> = units
        .iter()
        .filter(|unit| {
            unit.is_object()
                && unit
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        })
        .collect();
    if candidates.is_empty() {
        return Ok((0, 0));
    }
    let _lock = DistillLock::acquire_wait(store, "append", Duration::from_secs(2))?;
    if units_fingerprint(store) != keys.fingerprint {
        *keys = KeySet::load(store)?;
    }
    let mut pending: Vec<&serde_json::Value> = Vec::new();
    let mut accepted: HashSet<&str> = HashSet::new();
    for unit in &candidates {
        let Some(key) = unit.get("key").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if keys.contains(key) || accepted.contains(key) {
            continue;
        }
        accepted.insert(key);
        pending.push(*unit);
    }
    if pending.is_empty() {
        return Ok((0, 0));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store.units_path())?;
    for unit in &pending {
        writeln!(file, "{}", serde_json::to_string(unit)?)?;
    }
    file.flush()?;
    for key in &accepted {
        keys.keys.insert((*key).to_string());
    }
    keys.fingerprint = units_fingerprint(store);
    let compactions = pending
        .iter()
        .filter(|unit| {
            unit.get("kind").and_then(serde_json::Value::as_str) == Some(COMPACTION_KIND)
        })
        .count();
    Ok((pending.len(), compactions))
}

pub fn ingest_paths(
    store: &Store,
    roots: &IngestRoots,
    paths: &[PathBuf],
    keys: &mut KeySet,
) -> anyhow::Result<IngestReport> {
    let mut report = IngestReport::default();
    let mut persisted = state::IngestState::load(store);
    for path in paths {
        let Some((source, agent_home)) = roots.source_of(path) else {
            continue;
        };
        if is_ignored(roots, path) {
            continue;
        }
        let metadata = match std::fs::metadata(path) {
            Ok(found) => found,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        let size = metadata.len();
        let mtime_ms = mtime_millis(&metadata);
        let inode = state::inode_of(&metadata);
        let head = state::head_of(path);
        let cursor = match persisted.get(path) {
            Some(previous)
                if previous.parser == PARSER_VERSION
                    && size >= previous.offset
                    && inode == previous.inode
                    && head.starts_with(&previous.head) =>
            {
                transcript::ParseCursor {
                    offset: previous.offset,
                    session: previous.session.clone(),
                    cwd: previous.cwd.clone(),
                }
            }
            Some(_) => {
                report.reparsed += 1;
                transcript::ParseCursor::default()
            }
            None => transcript::ParseCursor::default(),
        };
        let parsed = transcript::parse_file(path, source, agent_home, cursor)?;
        let (appended, compactions) = append_units_inner(store, &parsed.units, keys)?;
        report.files += 1;
        report.appended += appended;
        report.compactions += compactions;
        report.duplicates += parsed.units.len().saturating_sub(appended);
        persisted.set(
            path,
            state::FileState {
                offset: parsed.cursor.offset,
                size,
                mtime_ms,
                inode,
                head,
                session: parsed.cursor.session,
                cwd: parsed.cursor.cwd,
                parser: PARSER_VERSION,
            },
        );
    }
    persisted.save(store)?;
    Ok(report)
}

pub fn walk_roots(roots: &IngestRoots) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in &roots.roots {
        paths.extend(walk_root(roots, &root.path, 0));
    }
    paths
}

pub fn ingest_all(
    store: &Store,
    roots: &IngestRoots,
    keys: &mut KeySet,
) -> anyhow::Result<IngestReport> {
    ingest_paths(store, roots, &walk_roots(roots), keys)
}

fn walk_root(roots: &IngestRoots, dir: &Path, depth: usize) -> Vec<PathBuf> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    names.sort_by(|left, right| left.0.cmp(&right.0));
    let mut out = Vec::new();
    for (_, path) in names {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if is_ignored(roots, &path) {
                continue;
            }
            out.extend(walk_root(roots, &path, depth + 1));
        } else if is_jsonl_name(&path) && !is_ignored(roots, &path) {
            out.push(path);
        }
    }
    out
}

fn is_jsonl_name(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".jsonl"))
}

fn mtime_millis(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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
                "qol-memory-ingest-{}-{}-{}",
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

    fn store_in(dir: &TempDir) -> Store {
        let root = dir.0.join("store");
        std::fs::create_dir_all(&root).unwrap();
        Store::resolve(Some(root.as_path())).unwrap()
    }

    fn roots_in(dir: &TempDir) -> IngestRoots {
        let pi = dir.0.join("pi");
        let claude = dir.0.join("claude");
        std::fs::create_dir_all(&pi).unwrap();
        std::fs::create_dir_all(&claude).unwrap();
        IngestRoots {
            roots: vec![
                IngestRoot {
                    path: pi,
                    source: "pi",
                    agent_home: "test-pi".to_string(),
                },
                IngestRoot {
                    path: claude,
                    source: "claude",
                    agent_home: "test-claude".to_string(),
                },
            ],
        }
    }

    fn root_dir(roots: &IngestRoots, source: &str) -> PathBuf {
        roots
            .roots
            .iter()
            .find(|root| root.source == source)
            .unwrap()
            .path
            .clone()
    }

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap()
    }

    fn clear_transcript_env_vars() {
        std::env::remove_var("QOL_MEMORY_PI_DIR");
        std::env::remove_var("QOL_MEMORY_CLAUDE_DIR");
    }

    #[test]
    fn unit_key_matches_snapshot_formula() {
        let expected = {
            let joined = ["pi", "a.jsonl", "2026-01-01T00:00:00.000Z", "hello"].join("|");
            let digest = Sha256::digest(joined.as_bytes());
            let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            hex[..16].to_string()
        };
        assert_eq!(
            unit_key("pi", "a.jsonl", Some("2026-01-01T00:00:00.000Z"), "hello"),
            expected
        );
    }

    #[test]
    fn null_ts_joins_as_empty() {
        assert_eq!(
            unit_key("pi", "a.jsonl", None, "hello"),
            unit_key("pi", "a.jsonl", Some(""), "hello")
        );
    }

    #[test]
    fn capture_unit_key_ignores_ts_and_depends_on_cwd_and_text() {
        let text = "the clipboard ring survives tray restarts";
        let first = capture_unit(text, "/repo", "2026-08-01T09:00:00.000Z");
        let second = capture_unit(text, "/repo", "2027-01-02T03:04:05.000Z");
        assert_eq!(first["key"], second["key"]);
        assert_ne!(
            first["key"],
            capture_unit("another fact entirely", "/repo", "2026-08-01T09:00:00.000Z")["key"]
        );
        assert_ne!(
            first["key"],
            capture_unit(text, "/elsewhere", "2026-08-01T09:00:00.000Z")["key"]
        );
        assert_eq!(first["source"], "agent");
        assert_eq!(first["kind"], "capture");
        assert_eq!(first["ts"], "2026-08-01T09:00:00.000Z");
        assert!(first.get("file").is_none());
        assert!(first.get("session").is_none());
    }

    #[test]
    fn ignore_rules_match_snapshot_semantics() {
        let dir = TempDir::new("ignore");
        let roots = roots_in(&dir);
        let pi = root_dir(&roots, "pi");
        let claude = root_dir(&roots, "claude");
        assert!(is_ignored(&roots, &pi.join("sub").join("token-file.jsonl")));
        assert!(is_ignored(&roots, &claude.join("api-secret-keys.jsonl")));
        assert!(is_ignored(&roots, &claude.join("deep").join(".env")));
        assert!(is_ignored(&roots, &claude.join(".env.local")));
        assert!(!is_ignored(&roots, &pi.join("plain.jsonl")));
        assert!(!is_ignored(&roots, &pi.join("c.jsonl")));
    }

    #[test]
    fn append_units_skips_duplicate_keys_and_holds_lock() {
        let dir = TempDir::new("append");
        let store = store_in(&dir);
        let mut keys = KeySet::load(&store).unwrap();
        let units = [
            serde_json::json!({"key": "k1"}),
            serde_json::json!({"key": "k2"}),
            serde_json::json!({"no-key": true}),
            serde_json::json!({"key": 7}),
        ];
        assert_eq!(append_units(&store, &units, &mut keys).unwrap(), 2);
        assert_eq!(append_units(&store, &units, &mut keys).unwrap(), 0);
        let raw = std::fs::read_to_string(store.units_path()).unwrap();
        assert_eq!(raw.lines().count(), 2);
        assert!(keys.contains("k1"));
        assert!(keys.contains("k2"));
        let held = DistillLock::acquire(&store, "test").unwrap().unwrap();
        assert!(append_units(&store, &[json_unit("k3")], &mut keys).is_err());
        drop(held);
        assert_eq!(
            append_units(&store, &[json_unit("k3")], &mut keys).unwrap(),
            1
        );
    }

    fn json_unit(key: &str) -> serde_json::Value {
        serde_json::json!({"key": key})
    }

    #[test]
    fn append_units_dedupes_within_one_batch() {
        let dir = TempDir::new("batch-dedupe");
        let store = store_in(&dir);
        let mut keys = KeySet::load(&store).unwrap();
        let units = [
            json_unit("same-key"),
            json_unit("same-key"),
            json_unit("same-key"),
        ];
        let appended = append_units(&store, &units, &mut keys).unwrap();
        assert_eq!(appended, 1);
        let raw = std::fs::read_to_string(store.units_path()).unwrap();
        assert_eq!(raw.lines().count(), 1);
        assert!(keys.contains("same-key"));
    }

    #[test]
    fn append_units_reloads_keys_after_external_rewrite() {
        let dir = TempDir::new("external-rewrite");
        let store = store_in(&dir);
        let mut keys = KeySet::load(&store).unwrap();
        let seed = [json_unit("seed-key")];
        assert_eq!(append_units(&store, &seed, &mut keys).unwrap(), 1);
        let seed_line = std::fs::read_to_string(store.units_path()).unwrap();
        let external = format!(
            "{}{{\"key\":\"merge-key\",\"kind\":\"user\",\"text\":\"landed by the merge step\"}}\n",
            seed_line
        );
        std::fs::write(store.units_path(), external).unwrap();
        let merge = [json_unit("merge-key")];
        assert_eq!(append_units(&store, &merge, &mut keys).unwrap(), 0);
        assert!(keys.contains("merge-key"));
        let raw = std::fs::read_to_string(store.units_path()).unwrap();
        assert_eq!(raw.lines().count(), 2);
    }

    #[test]
    fn offset_resume_only_parses_new_lines() {
        let dir = TempDir::new("resume");
        let store = store_in(&dir);
        let roots = roots_in(&dir);
        let path = root_dir(&roots, "pi").join("a.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session","id":"s1","cwd":"/w"}"#,
                "\n",
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"first question"}],"timestamp":1787819945554}}"#,
                "\n"
            ),
        )
        .unwrap();
        let mut keys = KeySet::load(&store).unwrap();
        let first = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(first.files, 1);
        assert_eq!(first.appended, 1);
        assert_eq!(first.reparsed, 0);

        append_text(
            &path,
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"second"#,
        );
        let paths = std::slice::from_ref(&path);
        let partial = ingest_paths(&store, &roots, paths, &mut keys).unwrap();
        assert_eq!(partial.appended, 0);
        assert_eq!(partial.reparsed, 0);
        let offset_before = state::IngestState::load(&store).get(&path).unwrap().offset;

        append_text(
            &path,
            &format!("{}\n", r#" question"}],"timestamp":1787819945600}}"#),
        );
        let third = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(third.appended, 1);
        assert_eq!(third.reparsed, 0);
        let persisted = state::IngestState::load(&store).get(&path).unwrap().offset;
        assert_ne!(persisted, offset_before);
        assert_eq!(persisted, std::fs::metadata(&path).unwrap().len());
        let raw = std::fs::read_to_string(store.units_path()).unwrap();
        assert_eq!(raw.lines().count(), 2);
    }

    fn append_text(path: &Path, text: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        write!(file, "{text}").unwrap();
    }

    #[test]
    fn shrunk_file_reparses_from_zero() {
        let dir = TempDir::new("shrunk");
        let store = store_in(&dir);
        let roots = roots_in(&dir);
        let path = root_dir(&roots, "claude").join("a.jsonl");
        let line = |text: &str| {
            format!("{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}},\"sessionId\":\"sx\",\"cwd\":\"/wx\",\"timestamp\":\"2026-02-01T00:00:00.000Z\"}}\n")
        };
        std::fs::write(
            &path,
            format!(
                "{}{}{}",
                line("alpha shrink note"),
                line("beta shrink note"),
                line("gamma shrink note")
            ),
        )
        .unwrap();
        let mut keys = KeySet::load(&store).unwrap();
        let first = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(first.appended, 3);
        assert_eq!(first.reparsed, 0);

        std::fs::write(&path, line("alpha shrink note")).unwrap();
        let second = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(second.reparsed, 1);
        assert_eq!(second.appended, 0);
        assert_eq!(second.duplicates, 1);

        std::fs::remove_file(&path).unwrap();
        std::fs::write(
            &path,
            format!(
                "{}{}{}{}",
                line("delta fresh note"),
                line("epsilon fresh note"),
                line("zeta fresh note"),
                line("eta fresh note")
            ),
        )
        .unwrap();
        let third = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(third.reparsed, 1);
        assert_eq!(third.appended, 4);
    }

    #[test]
    fn older_parser_state_reparses_from_zero() {
        let dir = TempDir::new("parser-version");
        let store = store_in(&dir);
        let roots = roots_in(&dir);
        let path = root_dir(&roots, "claude").join("a.jsonl");
        let line = |text: &str| {
            format!("{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}},\"sessionId\":\"sp\",\"cwd\":\"/wp\",\"timestamp\":\"2026-03-01T00:00:00.000Z\"}}\n")
        };
        std::fs::write(&path, line("alpha parser note")).unwrap();
        let mut keys = KeySet::load(&store).unwrap();
        let first = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(first.appended, 1);
        assert_eq!(first.reparsed, 0);
        let state_path = store.ingest_state_path();
        let state_text = std::fs::read_to_string(&state_path).unwrap();
        assert!(state_text.contains(&format!("\"parser\": {PARSER_VERSION}")));
        std::fs::write(
            &state_path,
            state_text.replace(&format!("\"parser\": {PARSER_VERSION}"), "\"parser\": 1"),
        )
        .unwrap();
        let second = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(second.reparsed, 1);
        assert_eq!(second.appended, 0);
        assert_eq!(second.duplicates, 1);
        let saved = std::fs::read_to_string(&state_path).unwrap();
        assert!(saved.contains(&format!("\"parser\": {PARSER_VERSION}")));
    }

    #[test]
    fn compactions_counts_appended_compaction_units() {
        let dir = TempDir::new("compactions-count");
        let store = store_in(&dir);
        let roots = roots_in(&dir);
        let path = root_dir(&roots, "claude").join("a.jsonl");
        let compact = r#"{"type":"user","isCompactSummary":true,"message":{"content":[{"type":"text","text":"This session is being continued from a previous conversation"}]},"timestamp":"2026-04-01T00:00:00.000Z"}"#;
        let user = r#"{"type":"user","message":{"content":[{"type":"text","text":"a plain question"}]},"timestamp":"2026-04-01T00:00:01.000Z"}"#;
        std::fs::write(&path, format!("{compact}\n{user}\n")).unwrap();
        let mut keys = KeySet::load(&store).unwrap();
        let first = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(first.appended, 2);
        assert_eq!(first.compactions, 1);
        std::fs::write(&path, format!("{compact}\n")).unwrap();
        let shrunk = ingest_paths(&store, &roots, std::slice::from_ref(&path), &mut keys).unwrap();
        assert_eq!(shrunk.reparsed, 1);
        assert_eq!(shrunk.appended, 0);
        assert_eq!(shrunk.duplicates, 1);
        assert_eq!(shrunk.compactions, 0);
    }

    #[test]
    fn from_registry_without_declared_homes_yields_one_root_per_supported_harness() {
        let _guard = env_guard();
        clear_transcript_env_vars();
        let user_home = PathBuf::from("/qol-memory-fake-home");
        let registry = qol_agent_homes::Registry::load_from(None, &user_home, &|_| None);
        let roots = IngestRoots::from_registry(&registry);
        assert_eq!(roots.roots.len(), 2);
        for source in SUPPORTED_SOURCES {
            assert!(roots.roots.iter().any(|root| root.source == source));
        }
        let claude = roots
            .roots
            .iter()
            .find(|root| root.source == "claude")
            .unwrap();
        assert_eq!(claude.path, user_home.join(".claude").join("projects"));
        assert_eq!(
            claude.agent_home,
            user_home.join(".claude").to_string_lossy().into_owned()
        );
    }

    #[test]
    fn two_declared_claude_homes_are_both_walked_and_sourced() {
        let _guard = env_guard();
        clear_transcript_env_vars();
        let dir = TempDir::new("two-homes");
        let file = dir.0.join("agents.toml");
        std::fs::write(
            &file,
            concat!(
                "[[home]]\n",
                "harness = \"claude\"\n",
                "path = \"~/claude-one\"\n",
                "\n",
                "[[home]]\n",
                "harness = \"claude\"\n",
                "path = \"~/claude-two\"\n",
            ),
        )
        .unwrap();
        let registry = qol_agent_homes::Registry::load_from(Some(&file), &dir.0, &|_| None);
        let roots = IngestRoots::from_registry(&registry);
        let claude_roots: Vec<&IngestRoot> = roots
            .roots
            .iter()
            .filter(|root| root.source == "claude")
            .collect();
        assert_eq!(claude_roots.len(), 3);
        let first = dir.0.join("claude-one").join("projects");
        let second = dir.0.join("claude-two").join("projects");
        assert!(claude_roots.iter().any(|root| root.path == first));
        assert!(claude_roots.iter().any(|root| root.path == second));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("a.jsonl"), "").unwrap();
        std::fs::write(second.join("b.jsonl"), "").unwrap();
        assert_eq!(
            walk_roots(&roots),
            vec![first.join("a.jsonl"), second.join("b.jsonl")]
        );
        let first_home = dir.0.join("claude-one").to_string_lossy().into_owned();
        let second_home = dir.0.join("claude-two").to_string_lossy().into_owned();
        assert_eq!(
            roots.source_of(&first.join("a.jsonl")),
            Some(("claude", first_home.as_str()))
        );
        assert_eq!(
            roots.source_of(&second.join("b.jsonl")),
            Some(("claude", second_home.as_str()))
        );
    }

    #[test]
    fn env_var_replaces_that_harness_roots_with_current_home() {
        let _guard = env_guard();
        clear_transcript_env_vars();
        let user_home = PathBuf::from("/qol-memory-env-home");
        let registry = qol_agent_homes::Registry::load_from(None, &user_home, &|_| None);
        std::env::set_var("QOL_MEMORY_CLAUDE_DIR", "/env-home/.claude/projects");
        let roots = IngestRoots::from_registry(&registry);
        std::env::remove_var("QOL_MEMORY_CLAUDE_DIR");
        let claude_roots: Vec<&IngestRoot> = roots
            .roots
            .iter()
            .filter(|root| root.source == "claude")
            .collect();
        assert_eq!(claude_roots.len(), 1);
        assert_eq!(
            claude_roots[0].path,
            PathBuf::from("/env-home/.claude/projects")
        );
        assert_eq!(claude_roots[0].agent_home, "/qol-memory-env-home/.claude");
        let pi = roots.roots.iter().find(|root| root.source == "pi").unwrap();
        assert_eq!(pi.agent_home, "/qol-memory-env-home/.pi/agent");
    }
}
