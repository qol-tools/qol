use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct FieldConflict {
    pub file: String,
    pub plugin: Option<String>,
    pub key_path: String,
    pub local: Value,
    pub remote: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileMerge {
    Clean(Value),
    Conflicted {
        merged: Value,
        conflicts: Vec<FieldConflict>,
    },
}

const RELEASE_FIELDS: [&str; 2] = ["version", "platforms"];

pub type ConflictResolver<'a> = dyn Fn(&str, &str) -> Option<bool> + 'a;

pub fn merge_json(
    file: &str,
    plugin: Option<&str>,
    base: &Value,
    local: &Value,
    remote: &Value,
) -> FileMerge {
    merge_json_resolved(file, plugin, base, local, remote, &|_, _| None)
}

pub fn merge_json_resolved(
    file: &str,
    plugin: Option<&str>,
    base: &Value,
    local: &Value,
    remote: &Value,
    resolve: &ConflictResolver<'_>,
) -> FileMerge {
    let mut conflicts = Vec::new();
    let merged = merge_node(
        file,
        plugin,
        "",
        Some(base),
        Some(local),
        Some(remote),
        resolve,
        &mut conflicts,
    )
    .unwrap_or(Value::Null);
    if conflicts.is_empty() {
        FileMerge::Clean(merged)
    } else {
        FileMerge::Conflicted { merged, conflicts }
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_node(
    file: &str,
    plugin: Option<&str>,
    path: &str,
    base: Option<&Value>,
    local: Option<&Value>,
    remote: Option<&Value>,
    resolve: &ConflictResolver<'_>,
    conflicts: &mut Vec<FieldConflict>,
) -> Option<Value> {
    if all_objects(local, remote) {
        let mut out = Map::new();
        for key in union_keys(base, local, remote) {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let merged = merge_node(
                file,
                plugin,
                &child_path,
                base.and_then(Value::as_object).and_then(|m| m.get(&key)),
                local.and_then(Value::as_object).and_then(|m| m.get(&key)),
                remote.and_then(Value::as_object).and_then(|m| m.get(&key)),
                resolve,
                conflicts,
            );
            if let Some(value) = merged {
                out.insert(key, value);
            }
        }
        return Some(Value::Object(out));
    }
    resolve_leaf(file, plugin, path, base, local, remote, resolve, conflicts)
}

#[allow(clippy::too_many_arguments)]
fn resolve_leaf(
    file: &str,
    plugin: Option<&str>,
    path: &str,
    base: Option<&Value>,
    local: Option<&Value>,
    remote: Option<&Value>,
    resolve: &ConflictResolver<'_>,
    conflicts: &mut Vec<FieldConflict>,
) -> Option<Value> {
    if local == remote {
        return local.cloned();
    }
    if base == local {
        return remote.cloned();
    }
    if base == remote {
        return local.cloned();
    }
    match resolve(file, path) {
        Some(true) => return remote.cloned(),
        Some(false) => return local.cloned(),
        None => {}
    }
    conflicts.push(FieldConflict {
        file: file.to_string(),
        plugin: plugin.map(str::to_string),
        key_path: path.to_string(),
        local: local.cloned().unwrap_or(Value::Null),
        remote: remote.cloned().unwrap_or(Value::Null),
    });
    local.cloned()
}

fn all_objects(local: Option<&Value>, remote: Option<&Value>) -> bool {
    matches!(local, Some(Value::Object(_))) && matches!(remote, Some(Value::Object(_)))
}

fn union_keys(base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for value in [base, local, remote].into_iter().flatten() {
        if let Some(map) = value.as_object() {
            for key in map.keys() {
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
    }
    keys
}

pub struct ProfileSnapshot {
    pub files: BTreeMap<String, Value>,
}

pub struct ProfileMerge {
    pub merged: BTreeMap<String, Value>,
    pub conflicts: Vec<FieldConflict>,
}

pub fn merge_profile(
    base: &ProfileSnapshot,
    local: &ProfileSnapshot,
    remote: &ProfileSnapshot,
) -> ProfileMerge {
    merge_profile_with(base, local, remote, &|_, _| None)
}

pub fn merge_profile_with(
    base: &ProfileSnapshot,
    local: &ProfileSnapshot,
    remote: &ProfileSnapshot,
    resolve: &ConflictResolver<'_>,
) -> ProfileMerge {
    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();
    let mut files: Vec<&String> = base
        .files
        .keys()
        .chain(local.files.keys())
        .chain(remote.files.keys())
        .collect();
    files.sort();
    files.dedup();

    for file in files {
        let b = base.files.get(file);
        let l = local.files.get(file);
        let r = remote.files.get(file);
        if file.ends_with("plugins.lock.json") {
            merged.insert(
                file.clone(),
                merge_lock(file, b, l, r, resolve, &mut conflicts),
            );
            continue;
        }
        match (l, r) {
            (Some(l), Some(r)) => {
                let plugin = plugin_id_from_path(file);
                let null = Value::Null;
                match merge_json_resolved(
                    file,
                    plugin.as_deref(),
                    b.unwrap_or(&null),
                    l,
                    r,
                    resolve,
                ) {
                    FileMerge::Clean(v) => {
                        merged.insert(file.clone(), v);
                    }
                    FileMerge::Conflicted {
                        merged: v,
                        conflicts: c,
                    } => {
                        merged.insert(file.clone(), v);
                        conflicts.extend(c);
                    }
                }
            }
            (Some(only), None) | (None, Some(only)) => {
                merged.insert(file.clone(), only.clone());
            }
            (None, None) => {}
        }
    }
    ProfileMerge { merged, conflicts }
}

fn index_lock_entries(snapshot: Option<&Value>) -> BTreeMap<String, Value> {
    let mut indexed = BTreeMap::new();
    for entry in snapshot
        .and_then(|value| value.get("plugins"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        let renamed = match qol_migrations::renamed_plugin_id(id) {
            Some(new_id) => new_id,
            None => id,
        };
        let mut normalized = entry.clone();
        normalized["id"] = Value::String(renamed.to_string());
        let key = normalized
            .get("uid")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| renamed.to_string());
        indexed.insert(key, normalized);
    }
    indexed
}

fn merge_lock(
    file: &str,
    base: Option<&Value>,
    local: Option<&Value>,
    remote: Option<&Value>,
    resolve: &ConflictResolver<'_>,
    conflicts: &mut Vec<FieldConflict>,
) -> Value {
    let base = index_lock_entries(base);
    let local = index_lock_entries(local);
    let remote = index_lock_entries(remote);

    let mut keys: Vec<String> = local.keys().chain(remote.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    let mut entries = Vec::new();
    for key in keys {
        let b = base.get(&key);
        let l = local.get(&key);
        let r = remote.get(&key);
        let chosen = match (l, r) {
            (Some(l), Some(r)) if l == r => Some(l.clone()),
            (Some(l), Some(r)) if b == Some(l) => Some(r.clone()),
            (Some(l), Some(r)) if b == Some(r) => Some(l.clone()),
            (Some(l), Some(r)) => {
                let label = l.get("id").and_then(Value::as_str).unwrap_or(key.as_str());
                match resolve(file, &format!("plugins.{label}")) {
                    Some(true) => Some(r.clone()),
                    Some(false) => Some(l.clone()),
                    None => match newer_version(b, l, r) {
                        Some(newer) => Some(newer.clone()),
                        None => {
                            conflicts.push(FieldConflict {
                                file: file.to_string(),
                                plugin: Some(label.to_string()),
                                key_path: format!("plugins.{label}"),
                                local: l.clone(),
                                remote: r.clone(),
                            });
                            Some(l.clone())
                        }
                    },
                }
            }
            (Some(only), None) | (None, Some(only)) => Some(only.clone()),
            (None, None) => None,
        };
        if let Some(entry) = chosen {
            entries.push(entry);
        }
    }
    serde_json::json!({ "plugins": entries })
}

fn newer_version<'a>(
    base: Option<&Value>,
    local: &'a Value,
    remote: &'a Value,
) -> Option<&'a Value> {
    let version = |entry: &Value| {
        entry
            .get("version")
            .and_then(Value::as_str)
            .and_then(qol_plugin_api::manifest::parse_version)
    };
    if without_release_fields(local) != without_release_fields(remote) {
        return None;
    }
    let (mine, theirs) = (version(local)?, version(remote)?);
    if base
        .and_then(version)
        .is_some_and(|floor| mine < floor || theirs < floor)
    {
        return None;
    }
    match mine.cmp(&theirs) {
        std::cmp::Ordering::Greater => Some(local),
        std::cmp::Ordering::Less => Some(remote),
        std::cmp::Ordering::Equal => None,
    }
}

fn without_release_fields(entry: &Value) -> Value {
    let mut entry = entry.clone();
    if let Some(map) = entry.as_object_mut() {
        for key in RELEASE_FIELDS {
            map.remove(key);
        }
    }
    entry
}

fn plugin_id_from_path(file: &str) -> Option<String> {
    let name = file.rsplit('/').next()?;
    let stem = name.strip_suffix(".json")?;
    file.contains("plugin-configs/").then(|| stem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_platform::Os;
    use serde_json::json;

    fn merged(out: &FileMerge) -> &Value {
        match out {
            FileMerge::Clean(v) => v,
            FileMerge::Conflicted { merged, .. } => merged,
        }
    }
    fn conflicts(out: &FileMerge) -> Vec<&FieldConflict> {
        match out {
            FileMerge::Clean(_) => vec![],
            FileMerge::Conflicted { conflicts, .. } => conflicts.iter().collect(),
        }
    }

    #[test]
    fn three_way_buckets_resolve_without_conflict() {
        let cases = [
            (
                json!({"a": 1}),
                json!({"a": 1}),
                json!({"a": 1}),
                json!({"a": 1}),
                0,
            ),
            (
                json!({"a": 1}),
                json!({"a": 2}),
                json!({"a": 1}),
                json!({"a": 2}),
                0,
            ),
            (
                json!({"a": 1}),
                json!({"a": 1}),
                json!({"a": 3}),
                json!({"a": 3}),
                0,
            ),
            (
                json!({"a": 1}),
                json!({"a": 2}),
                json!({"a": 2}),
                json!({"a": 2}),
                0,
            ),
            (json!({}), json!({"a": 1}), json!({}), json!({"a": 1}), 0),
            (json!({"a": 1}), json!({}), json!({"a": 1}), json!({}), 0),
        ];
        for (base, local, remote, want_merged, want_conflicts) in cases {
            let out = merge_json("f.json", None, &base, &local, &remote);
            assert_eq!(
                merged(&out),
                &want_merged,
                "base={base} local={local} remote={remote}"
            );
            assert_eq!(conflicts(&out).len(), want_conflicts, "base={base}");
        }
    }

    #[test]
    fn both_changed_same_key_is_a_conflict() {
        let out = merge_json(
            "f.json",
            Some("qol-alt-tab"),
            &json!({"opacity": 1.0}),
            &json!({"opacity": 0.8}),
            &json!({"opacity": 0.5}),
        );
        let c = conflicts(&out);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key_path, "opacity");
        assert_eq!(c[0].local, json!(0.8));
        assert_eq!(c[0].remote, json!(0.5));
        assert_eq!(c[0].plugin.as_deref(), Some("qol-alt-tab"));
        assert_eq!(merged(&out), &json!({"opacity": 0.8}));
    }

    #[test]
    fn nested_objects_recurse_and_path_is_dotted() {
        let out = merge_json(
            "f.json",
            None,
            &json!({"win": {"w": 10, "h": 20}}),
            &json!({"win": {"w": 11, "h": 20}}),
            &json!({"win": {"w": 12, "h": 20}}),
        );
        let c = conflicts(&out);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key_path, "win.w");
    }

    #[test]
    fn array_is_a_single_leaf() {
        let out = merge_json(
            "f.json",
            None,
            &json!({"order": [1, 2]}),
            &json!({"order": [1, 2, 3]}),
            &json!({"order": [2, 1]}),
        );
        let c = conflicts(&out);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key_path, "order");
        assert_eq!(c[0].local, json!([1, 2, 3]));
    }

    #[test]
    fn merge_profile_unions_files_and_collects_conflicts_per_file() {
        let snap = |pairs: &[(&str, Value)]| ProfileSnapshot {
            files: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        };
        let base = snap(&[("core/plugin-configs/a.json", json!({"x": 1}))]);
        let local = snap(&[
            ("core/plugin-configs/a.json", json!({"x": 2})),
            ("core/plugin-configs/b.json", json!({"y": 9})),
        ]);
        let remote = snap(&[("core/plugin-configs/a.json", json!({"x": 3}))]);

        let out = merge_profile(&base, &local, &remote);
        assert_eq!(out.conflicts.len(), 1, "x changed on both sides");
        assert_eq!(out.conflicts[0].file, "core/plugin-configs/a.json");
        assert_eq!(out.conflicts[0].key_path, "x");
        assert!(
            out.merged.contains_key("core/plugin-configs/b.json"),
            "local-only file kept"
        );
    }

    #[test]
    fn plugins_lock_uses_union_not_generic_merge() {
        let base = ProfileSnapshot {
            files: BTreeMap::new(),
        };
        let local = ProfileSnapshot {
            files: BTreeMap::from([(
                "core/plugins.lock.json".to_string(),
                json!({"plugins": [{"id": "p-mac", "platforms": [Os::Macos.name()]}]}),
            )]),
        };
        let remote = ProfileSnapshot {
            files: BTreeMap::from([(
                "core/plugins.lock.json".to_string(),
                json!({"plugins": [{"id": "p-linux", "platforms": [Os::Linux.name()]}]}),
            )]),
        };

        let out = merge_profile(&base, &local, &remote);
        assert_eq!(
            out.conflicts.len(),
            0,
            "disjoint plugins are a union, not a clash"
        );
        let lock = &out.merged["core/plugins.lock.json"];
        let ids: Vec<&str> = lock["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&"p-mac") && ids.contains(&"p-linux"),
            "both platform plugins preserved, got {ids:?}"
        );
    }

    fn lock_snapshot(entries: Vec<Value>) -> ProfileSnapshot {
        ProfileSnapshot {
            files: BTreeMap::from([(
                "core/plugins.lock.json".to_string(),
                json!({"plugins": entries}),
            )]),
        }
    }

    fn lock_entry(uid: Option<&str>, id: &str, version: &str) -> Value {
        match uid {
            Some(uid) => json!({"uid": uid, "id": id, "version": version}),
            None => json!({"id": id, "version": version}),
        }
    }

    #[test]
    fn plugins_lock_groups_entries_by_uid_and_keeps_the_renamed_id() {
        let base = lock_snapshot(vec![lock_entry(
            Some("uid-lights"),
            "plugin-lights",
            "0.9.0",
        )]);
        let local = lock_snapshot(vec![lock_entry(
            Some("uid-lights"),
            "plugin-lights",
            "1.0.0",
        )]);
        let remote = lock_snapshot(vec![lock_entry(Some("uid-lights"), "qol-lights", "1.0.0")]);

        let out = merge_profile(&base, &local, &remote);

        let plugins = out.merged["core/plugins.lock.json"]["plugins"]
            .as_array()
            .unwrap();
        assert_eq!(plugins.len(), 1, "one uid stays one entry");
        assert_eq!(plugins[0]["id"], "qol-lights");
        assert_eq!(plugins[0]["uid"], "uid-lights");
        assert_eq!(plugins[0]["version"], "1.0.0");
        assert!(
            out.conflicts.is_empty(),
            "the same state spelled with the old id is not a conflict"
        );
    }

    #[test]
    fn plugins_lock_conflict_uses_the_renamed_id() {
        let base = ProfileSnapshot {
            files: BTreeMap::new(),
        };
        let mut mine = lock_entry(Some("uid-lights"), "plugin-lights", "1.0.0");
        mine["repo_url"] = json!("https://example.com/mine");
        let mut theirs = lock_entry(Some("uid-lights"), "plugin-lights", "1.0.0");
        theirs["repo_url"] = json!("https://example.com/theirs");
        let local = lock_snapshot(vec![mine]);
        let remote = lock_snapshot(vec![theirs]);

        let out = merge_profile(&base, &local, &remote);

        assert_eq!(out.conflicts.len(), 1);
        assert_eq!(out.conflicts[0].key_path, "plugins.qol-lights");
        assert_eq!(out.conflicts[0].plugin.as_deref(), Some("qol-lights"));
        let plugins = out.merged["core/plugins.lock.json"]["plugins"]
            .as_array()
            .unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0]["id"], "qol-lights");
    }

    fn lock_version(out: &ProfileMerge) -> &Value {
        &out.merged["core/plugins.lock.json"]["plugins"][0]["version"]
    }

    #[test]
    fn plugins_lock_keeps_the_newer_version_when_both_sides_upgraded() {
        let base = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.60.0")]);
        let mut newer = lock_entry(Some("uid-rm"), "qol-removeapp", "0.79.2");
        newer["platforms"] = json!(["linux", "macos", "windows"]);
        let older = lock_entry(Some("uid-rm"), "qol-removeapp", "0.67.1");

        let pulled = merge_profile(
            &base,
            &lock_snapshot(vec![older.clone()]),
            &lock_snapshot(vec![newer.clone()]),
        );
        let pushed = merge_profile(
            &base,
            &lock_snapshot(vec![newer]),
            &lock_snapshot(vec![older]),
        );

        assert!(pulled.conflicts.is_empty());
        assert!(pushed.conflicts.is_empty());
        assert_eq!(lock_version(&pulled), "0.79.2");
        assert_eq!(lock_version(&pushed), "0.79.2");
        assert_eq!(
            pulled.merged["core/plugins.lock.json"]["plugins"][0]["platforms"],
            json!(["linux", "macos", "windows"])
        );
    }

    #[test]
    fn plugins_lock_honours_an_explicit_pick_over_the_newer_version() {
        let base = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.60.0")]);
        let local = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.67.1")]);
        let remote = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.79.2")]);

        let out = merge_profile_with(&base, &local, &remote, &|_, key| {
            (key == "plugins.qol-removeapp").then_some(false)
        });

        assert!(out.conflicts.is_empty());
        assert_eq!(lock_version(&out), "0.67.1");
    }

    #[test]
    fn plugins_lock_still_asks_when_the_older_side_also_changed_its_source() {
        let base = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.60.0")]);
        let mut older = lock_entry(Some("uid-rm"), "qol-removeapp", "0.67.1");
        older["repo_url"] = json!("https://example.com/fork");
        let local = lock_snapshot(vec![older]);
        let remote = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.79.2")]);

        let out = merge_profile(&base, &local, &remote);

        assert_eq!(out.conflicts.len(), 1);
        assert_eq!(lock_version(&out), "0.67.1");
    }

    #[test]
    fn plugins_lock_still_asks_when_one_side_rolled_back() {
        let base = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.80.0")]);
        let local = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.70.0")]);
        let remote = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.81.0")]);

        let out = merge_profile(&base, &local, &remote);

        assert_eq!(out.conflicts.len(), 1);
    }

    #[test]
    fn plugins_lock_still_asks_when_versions_cannot_be_ordered() {
        let base = ProfileSnapshot {
            files: BTreeMap::new(),
        };
        let local = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "dev")]);
        let remote = lock_snapshot(vec![lock_entry(Some("uid-rm"), "qol-removeapp", "0.79.2")]);

        let out = merge_profile(&base, &local, &remote);

        assert_eq!(out.conflicts.len(), 1);
        assert_eq!(out.conflicts[0].key_path, "plugins.qol-removeapp");
    }

    #[test]
    fn plugins_lock_normalizes_ids_without_uid_and_keeps_distinct_uids() {
        let base = ProfileSnapshot {
            files: BTreeMap::new(),
        };
        let local = lock_snapshot(vec![
            lock_entry(None, "plugin-lights", "1.0.0"),
            lock_entry(Some("uid-other"), "plugin-alt-tab", "2.0.0"),
        ]);
        let remote = lock_snapshot(vec![lock_entry(None, "qol-lights", "1.0.0")]);

        let out = merge_profile(&base, &local, &remote);

        assert!(out.conflicts.is_empty());
        let plugins = out.merged["core/plugins.lock.json"]["plugins"]
            .as_array()
            .unwrap();
        assert_eq!(plugins.len(), 2);
        assert!(plugins
            .iter()
            .any(|entry| entry["id"] == "qol-lights" && entry.get("uid").is_none()));
        assert!(plugins
            .iter()
            .any(|entry| entry["id"] == "qol-alt-tab" && entry["uid"] == "uid-other"));
    }
}
