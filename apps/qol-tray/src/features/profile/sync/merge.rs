use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FieldConflict {
    pub(crate) file: String,
    pub(crate) plugin: Option<String>,
    pub(crate) key_path: String,
    pub(crate) local: Value,
    pub(crate) remote: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FileMerge {
    Clean(Value),
    Conflicted {
        merged: Value,
        conflicts: Vec<FieldConflict>,
    },
}

pub(crate) fn merge_json(
    file: &str,
    plugin: Option<&str>,
    base: &Value,
    local: &Value,
    remote: &Value,
) -> FileMerge {
    let mut conflicts = Vec::new();
    let merged = merge_node(
        file,
        plugin,
        "",
        Some(base),
        Some(local),
        Some(remote),
        &mut conflicts,
    )
    .unwrap_or(Value::Null);
    if conflicts.is_empty() {
        FileMerge::Clean(merged)
    } else {
        FileMerge::Conflicted { merged, conflicts }
    }
}

fn merge_node(
    file: &str,
    plugin: Option<&str>,
    path: &str,
    base: Option<&Value>,
    local: Option<&Value>,
    remote: Option<&Value>,
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
                conflicts,
            );
            if let Some(value) = merged {
                out.insert(key, value);
            }
        }
        return Some(Value::Object(out));
    }
    resolve_leaf(file, plugin, path, base, local, remote, conflicts)
}

fn resolve_leaf(
    file: &str,
    plugin: Option<&str>,
    path: &str,
    base: Option<&Value>,
    local: Option<&Value>,
    remote: Option<&Value>,
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

#[cfg(test)]
mod tests {
    use super::*;
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
            (json!({"a": 1}), json!({"a": 1}), json!({"a": 1}), json!({"a": 1}), 0),
            (json!({"a": 1}), json!({"a": 2}), json!({"a": 1}), json!({"a": 2}), 0),
            (json!({"a": 1}), json!({"a": 1}), json!({"a": 3}), json!({"a": 3}), 0),
            (json!({"a": 1}), json!({"a": 2}), json!({"a": 2}), json!({"a": 2}), 0),
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
            Some("plugin-alt-tab"),
            &json!({"opacity": 1.0}),
            &json!({"opacity": 0.8}),
            &json!({"opacity": 0.5}),
        );
        let c = conflicts(&out);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key_path, "opacity");
        assert_eq!(c[0].local, json!(0.8));
        assert_eq!(c[0].remote, json!(0.5));
        assert_eq!(c[0].plugin.as_deref(), Some("plugin-alt-tab"));
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
}
