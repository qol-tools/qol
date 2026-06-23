# Profile Sync Conflict Resolver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

## Status (2026-06-23) — detection/merge core landed, all green & warning-free on `main`

Done (TDD, committed):
- Task 1 `d6604fad` — field-level 3-way merge engine (`sync/merge.rs`)
- Task 2 `5f5d969a` — full-document merge + plugin-lock union (`merge_lock`)
- Task 3 `ebe469e5` — merge-base, tree-snapshot, oid git helpers (`sync/git_repo.rs`)
- Task 5 (core) `2c640cf1` — reconcile orchestration `sync/reconcile.rs` (diverged repo → merged doc + `FieldConflict`s), with `mergeable_path` predicate

**RESUME AT — Task 5 wiring:** call `reconcile(&repo)` from `do_pull`'s `Diverged`
branch in `service.rs`; write merged files back via `repo_path.join(rel)`; persist
`ResolvableConflict`s into `SyncStateFile.conflicts`; set incident `Conflict` /
health `Attention`; when `conflicts.is_empty()` write+commit+push and go Healthy.
Then: Task 4 (blame dates — `chrono::DateTime::from_timestamp(secs,0).to_rfc3339()`,
fallback to tip-commit time), Task 6 `resolve_conflicts`, Task 7 routes (+delete
`acknowledge`), Task 8 pull-before-push, Tasks 9-11 Preact resolver dive, Task 12
e2e + full clippy stack.

Facts discovered during execution (supersede plan draft where they differ):
- Allowlist is `ProfileScopeStore::is_sync_allowlisted(rel)`; "mergeable" = that
  AND `.json` AND no `sync/` component (excludes backups). See `reconcile::mergeable_path`.
- On-disk paths carry the active-profile segment (`default/core/...`), not bare `core/`.
- `chrono` is available; `now_rfc3339` lives in `sync/state.rs`.
- No symmetric two-lock union existed; `merge.rs::merge_lock` is the new one.
- `FieldConflict`/`FileMerge` derive `PartialEq` only (serde_json::Value isn't `Eq`).
- Intermediate commits may carry `dead_code` until a symbol is consumed downstream;
  `cargo test --lib` stays clean. Task 12's `clippy -D warnings` is the gate.

---

**Goal:** Give qol-tray a UI path out of profile-sync divergence: a field-level 3-way merge that auto-resolves non-conflicting changes and a keyboard-first resolver dive that walks the user through genuine clashes one field at a time.

**Architecture:** A pure Rust merge engine (`merge.rs`) takes three profile snapshots (merge-base, local, remote) and returns a merged document plus a list of `FieldConflict`s. The sync service runs it on divergence: clean → write+commit+push; conflicts → persist them and surface a `profile-sync-conflicts` world-canvas dive that posts the user's per-field picks back. Both sides are snapshotted to a backup before any write.

**Tech Stack:** Rust (git2, serde_json, tokio, thiserror/anyhow), Preact + htm frontend (no JSX, no build step), token-based CSS.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-06-23-profile-sync-conflict-resolver-design.md`.
- `RUSTFLAGS=-D warnings` on all three OSes; no `dead_code`/`unused_imports`. Check every backend symbol against what each platform consumes (`qol-arch-cross-platform`).
- No code comments. Conventional commits, short imperative subject, no AI attribution, no `Co-Authored-By`.
- Frontend is htm tagged templates from `lib/html.js`; no JSX. Keyboard-first is a hard rule. Token CSS per `ui/styles/STYLE_GUIDE.md`; compose `Surface`/`ListRow` (`qol-tray-ui-systems`).
- Synced files = the promote allowlist only (`manifest.json`, `core/*`, `os/<bucket>/*`); backups are never merged (`promote.rs`).
- `plugins.lock.json` uses existing union semantics, not generic JSON merge (`qol-tray-feature-profile`).
- Last-edited dates are display-only and never auto-resolve a clash.
- All paths below are under `apps/qol-tray/`.

---

### Task 1: Merge engine — leaf + object 3-way over a single JSON file

**Files:**
- Create: `src/features/profile/sync/merge.rs`
- Modify: `src/features/profile/sync/mod.rs` (add `mod merge;` and re-export `FieldConflict`, `FileMerge`, `merge_json`)
- Test: inline `#[cfg(test)]` in `merge.rs`

**Interfaces:**
- Produces:
  - `struct FieldConflict { file: String, plugin: Option<String>, key_path: String, local: serde_json::Value, remote: serde_json::Value }`
  - `enum FileMerge { Clean(serde_json::Value), Conflicted { merged: serde_json::Value, conflicts: Vec<FieldConflict> } }`
  - `fn merge_json(file: &str, plugin: Option<&str>, base: &Value, local: &Value, remote: &Value) -> FileMerge`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(out: &FileMerge) -> &Value {
        match out { FileMerge::Clean(v) => v, FileMerge::Conflicted { merged, .. } => merged }
    }
    fn conflicts(out: &FileMerge) -> Vec<&FieldConflict> {
        match out { FileMerge::Clean(_) => vec![], FileMerge::Conflicted { conflicts, .. } => conflicts.iter().collect() }
    }

    #[test]
    fn three_way_buckets_resolve_without_conflict() {
        let cases = [
            (json!({"a": 1}), json!({"a": 1}), json!({"a": 1}), json!({"a": 1}), 0),
            (json!({"a": 1}), json!({"a": 2}), json!({"a": 1}), json!({"a": 2}), 0),
            (json!({"a": 1}), json!({"a": 1}), json!({"a": 3}), json!({"a": 3}), 0),
            (json!({"a": 1}), json!({"a": 2}), json!({"a": 2}), json!({"a": 2}), 0),
            (json!({}),       json!({"a": 1}), json!({}),       json!({"a": 1}), 0),
            (json!({"a": 1}), json!({}),       json!({"a": 1}), json!({}),       0),
        ];
        for (base, local, remote, want_merged, want_conflicts) in cases {
            let out = merge_json("f.json", None, &base, &local, &remote);
            assert_eq!(merged(&out), &want_merged, "base={base} local={local} remote={remote}");
            assert_eq!(conflicts(&out).len(), want_conflicts, "base={base}");
        }
    }

    #[test]
    fn both_changed_same_key_is_a_conflict() {
        let out = merge_json("f.json", Some("plugin-alt-tab"),
            &json!({"opacity": 1.0}), &json!({"opacity": 0.8}), &json!({"opacity": 0.5}));
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
        let out = merge_json("f.json", None,
            &json!({"win": {"w": 10, "h": 20}}),
            &json!({"win": {"w": 11, "h": 20}}),
            &json!({"win": {"w": 12, "h": 20}}));
        let c = conflicts(&out);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key_path, "win.w");
    }

    #[test]
    fn array_is_a_single_leaf() {
        let out = merge_json("f.json", None,
            &json!({"order": [1, 2]}), &json!({"order": [1, 2, 3]}), &json!({"order": [2, 1]}));
        let c = conflicts(&out);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].key_path, "order");
        assert_eq!(c[0].local, json!([1, 2, 3]));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-tray sync::merge -- --nocapture`
Expected: FAIL — `cannot find function merge_json` / unresolved `merge`.

- [ ] **Step 3: Implement the engine**

```rust
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
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
    Conflicted { merged: Value, conflicts: Vec<FieldConflict> },
}

pub fn merge_json(file: &str, plugin: Option<&str>, base: &Value, local: &Value, remote: &Value) -> FileMerge {
    let mut conflicts = Vec::new();
    let merged = merge_node(file, plugin, "", Some(base), Some(local), Some(remote), &mut conflicts)
        .unwrap_or(Value::Null);
    if conflicts.is_empty() {
        FileMerge::Clean(merged)
    } else {
        FileMerge::Conflicted { merged, conflicts }
    }
}

fn merge_node(
    file: &str, plugin: Option<&str>, path: &str,
    base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>,
    conflicts: &mut Vec<FieldConflict>,
) -> Option<Value> {
    if all_objects(local, remote) {
        let mut out = Map::new();
        for key in union_keys(base, local, remote) {
            let child_path = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
            let merged = merge_node(
                file, plugin, &child_path,
                base.and_then(|v| v.as_object()).and_then(|m| m.get(&key)),
                local.and_then(|v| v.as_object()).and_then(|m| m.get(&key)),
                remote.and_then(|v| v.as_object()).and_then(|m| m.get(&key)),
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
    file: &str, plugin: Option<&str>, path: &str,
    base: Option<&Value>, local: Option<&Value>, remote: Option<&Value>,
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
    for v in [base, local, remote].into_iter().flatten() {
        if let Some(map) = v.as_object() {
            for k in map.keys() {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    keys
}
```

Add to `mod.rs`: `mod merge;` and `pub(crate) use merge::{merge_json, FieldConflict, FileMerge};`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qol-tray sync::merge -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/merge.rs apps/qol-tray/src/features/profile/sync/mod.rs
git commit -m "feat(profile-sync): add field-level 3-way JSON merge engine"
```

---

### Task 2: Merge the whole profile document + lock union

**Files:**
- Modify: `src/features/profile/sync/merge.rs`
- Test: inline `#[cfg(test)]` in `merge.rs`

**Interfaces:**
- Consumes: `merge_json`, `FieldConflict`, `FileMerge` (Task 1); the existing lock-union helper in `core/mod.rs` (reuse it; if it is not callable from here, the implementer exposes a `pub(crate)` wrapper rather than duplicating the rule).
- Produces:
  - `struct ProfileSnapshot { files: std::collections::BTreeMap<String, Value> }` (relative path → parsed JSON)
  - `struct ProfileMerge { merged: BTreeMap<String, Value>, conflicts: Vec<FieldConflict> }`
  - `fn merge_profile(base: &ProfileSnapshot, local: &ProfileSnapshot, remote: &ProfileSnapshot) -> ProfileMerge`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn merge_profile_unions_files_and_collects_conflicts_per_file() {
    let snap = |pairs: &[(&str, Value)]| ProfileSnapshot {
        files: pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
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
    assert!(out.merged.contains_key("core/plugin-configs/b.json"), "local-only file kept");
}

#[test]
fn plugins_lock_uses_union_not_generic_merge() {
    let base = ProfileSnapshot { files: Default::default() };
    let local = ProfileSnapshot { files: [(
        "core/plugins.lock.json".to_string(),
        json!({"plugins": [{"id": "p-mac", "platforms": ["macos"]}]}),
    )].into() };
    let remote = ProfileSnapshot { files: [(
        "core/plugins.lock.json".to_string(),
        json!({"plugins": [{"id": "p-linux", "platforms": ["linux"]}]}),
    )].into() };

    let out = merge_profile(&base, &local, &remote);
    let lock = &out.merged["core/plugins.lock.json"];
    let ids: Vec<&str> = lock["plugins"].as_array().unwrap()
        .iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"p-mac") && ids.contains(&"p-linux"), "both platform plugins preserved");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray sync::merge::tests::merge_profile -- --nocapture`
Expected: FAIL — `merge_profile` not found.

- [ ] **Step 3: Implement**

```rust
use std::collections::BTreeMap;

pub struct ProfileSnapshot {
    pub files: BTreeMap<String, Value>,
}

pub struct ProfileMerge {
    pub merged: BTreeMap<String, Value>,
    pub conflicts: Vec<FieldConflict>,
}

pub fn merge_profile(base: &ProfileSnapshot, local: &ProfileSnapshot, remote: &ProfileSnapshot) -> ProfileMerge {
    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();
    let mut files: Vec<&String> = base.files.keys().chain(local.files.keys()).chain(remote.files.keys()).collect();
    files.sort();
    files.dedup();

    for file in files {
        let b = base.files.get(file);
        let l = local.files.get(file);
        let r = remote.files.get(file);
        if file.ends_with("plugins.lock.json") {
            merged.insert(file.clone(), union_lock(l, r));
            continue;
        }
        match (l, r) {
            (Some(l), Some(r)) => {
                let plugin = plugin_id_from_path(file);
                let null = Value::Null;
                match merge_json(file, plugin.as_deref(), b.unwrap_or(&null), l, r) {
                    FileMerge::Clean(v) => { merged.insert(file.clone(), v); }
                    FileMerge::Conflicted { merged: v, conflicts: c } => {
                        merged.insert(file.clone(), v);
                        conflicts.extend(c);
                    }
                }
            }
            (Some(only), None) | (None, Some(only)) => { merged.insert(file.clone(), only.clone()); }
            (None, None) => {}
        }
    }
    ProfileMerge { merged, conflicts }
}

fn plugin_id_from_path(file: &str) -> Option<String> {
    let name = file.rsplit('/').next()?;
    let stem = name.strip_suffix(".json")?;
    file.contains("plugin-configs/").then(|| stem.to_string())
}
```

For `union_lock(local, remote) -> Value`: call the existing lock reconciliation from `core/mod.rs`. Read that function first; wrap it so it takes the two parsed `Value` locks and returns the unioned `Value`. Do not re-implement the rule.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p qol-tray sync::merge -- --nocapture`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/merge.rs
git commit -m "feat(profile-sync): merge full profile document, union the lock"
```

---

### Task 3: Git helpers — merge-base, blob-at-commit, tracked-file list

**Files:**
- Modify: `src/features/profile/sync/git_repo.rs`
- Test: extend the existing `#[cfg(test)]` block (see `push_then_pull_roundtrips_through_bare_origin`)

**Interfaces:**
- Produces on `GitRepo`:
  - `fn merge_base_with_remote(&self) -> Result<Option<git2::Oid>>`
  - `fn snapshot_at(&self, oid: git2::Oid, allowlist_prefixes: &[&str]) -> Result<BTreeMap<String, Value>>` (relative path → parsed JSON, only allowlisted `.json` files)
  - `fn local_oid(&self) -> Result<Option<git2::Oid>>` and `fn remote_oid(&self) -> Result<git2::Oid>` (extract the inline logic already in `pull`)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn snapshot_at_reads_allowlisted_json_from_a_commit() {
    let (repo, _tmp) = init_repo_with_file(
        "core/plugin-configs/a.json", r#"{"x": 1}"#);
    let oid = repo.local_oid().unwrap().unwrap();
    let snap = repo.snapshot_at(oid, &["core/", "os/", "manifest.json"]).unwrap();
    assert_eq!(snap.get("core/plugin-configs/a.json"), Some(&serde_json::json!({"x": 1})));
}
```

(Reuse/extend the existing test harness that builds a bare origin and a working clone. If `init_repo_with_file` does not exist, add it next to the existing helpers, mirroring their setup.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray sync::git_repo -- --nocapture`
Expected: FAIL — methods not found.

- [ ] **Step 3: Implement**

```rust
pub fn merge_base_with_remote(&self) -> Result<Option<git2::Oid>> {
    let repo = self.open_repo()?;
    let Some(local) = self.local_oid()? else { return Ok(None) };
    let remote = self.remote_oid()?;
    Ok(repo.merge_base(local, remote).ok())
}

pub fn snapshot_at(&self, oid: git2::Oid, allow: &[&str]) -> Result<std::collections::BTreeMap<String, Value>> {
    let repo = self.open_repo()?;
    let tree = repo.find_commit(oid)?.tree()?;
    let mut out = std::collections::BTreeMap::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
        let name = entry.name().unwrap_or("");
        let path = format!("{dir}{name}");
        if path.ends_with(".json") && allow.iter().any(|p| path.starts_with(p) || path == *p) {
            if let Ok(obj) = entry.to_object(&repo) {
                if let Some(blob) = obj.as_blob() {
                    if let Ok(value) = serde_json::from_slice::<Value>(blob.content()) {
                        out.insert(path, value);
                    }
                }
            }
        }
        git2::TreeWalkResult::Ok
    })?;
    Ok(out)
}
```

Extract `local_oid`/`remote_oid` from the existing `pull` body (lines ~88–98) and have `pull` call them, so there is one source of truth.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p qol-tray sync::git_repo -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/git_repo.rs
git commit -m "feat(profile-sync): add merge-base and tree-snapshot git helpers"
```

---

### Task 4: Last-edited blame helper (display-only)

**Files:**
- Modify: `src/features/profile/sync/git_repo.rs`
- Test: extend `#[cfg(test)]`

**Interfaces:**
- Produces: `fn field_edited_at(&self, oid: git2::Oid, file: &str, key: &str) -> Result<Option<String>>` — RFC3339 commit time of the commit that last touched the pretty-printed line for `key` in `file` at `oid`; `None`/tip-time fallback when blame can't isolate a line.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn field_edited_at_returns_a_timestamp_for_a_known_key() {
    let (repo, _tmp) = init_repo_with_file("core/plugin-configs/a.json", "{\n  \"x\": 1\n}\n");
    let oid = repo.local_oid().unwrap().unwrap();
    let ts = repo.field_edited_at(oid, "core/plugin-configs/a.json", "x").unwrap();
    assert!(ts.is_some(), "blame should resolve a commit time for key x");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray sync::git_repo::tests::field_edited_at -- --nocapture`
Expected: FAIL — method not found.

- [ ] **Step 3: Implement** — find the line index of `"<key>":` in the blob at `oid`, `repo.blame_file` with `.newest_commit(oid)`, read the hunk for that line, convert `commit.time()` to RFC3339. On any miss, return the tip commit's time.

```rust
pub fn field_edited_at(&self, oid: git2::Oid, file: &str, key: &str) -> Result<Option<String>> {
    let repo = self.open_repo()?;
    let blob_text = self.snapshot_text_at(oid, file)?;
    let needle = format!("\"{key}\":");
    let line_no = blob_text.lines().position(|l| l.trim_start().starts_with(&needle));
    let mut opts = git2::BlameOptions::new();
    opts.newest_commit(oid);
    let path = std::path::Path::new(file);
    let when = match (line_no, repo.blame_file(path, Some(&mut opts))) {
        (Some(idx), Ok(blame)) => blame
            .get_line(idx + 1)
            .map(|h| h.final_signature().when()),
        _ => None,
    };
    let when = when.or_else(|| repo.find_commit(oid).ok().map(|c| c.time()));
    Ok(when.map(git_time_to_rfc3339))
}
```

(`snapshot_text_at` reads the raw blob string; add it beside `snapshot_at`. `git_time_to_rfc3339` converts `git2::Time` using the offset — small free function with a unit test on a fixed epoch.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p qol-tray sync::git_repo -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/git_repo.rs
git commit -m "feat(profile-sync): blame per-field last-edited time with tip fallback"
```

---

### Task 5: Reconcile on pull — clean auto-apply, else persist conflicts

**Files:**
- Modify: `src/features/profile/sync/service.rs` (`do_pull`, ~226–281)
- Modify: `src/features/profile/sync/state.rs` (add `conflicts: Vec<ResolvableConflict>` to `SyncStateFile`)
- Modify: `src/features/profile/sync/types.rs` (add `ResolvableConflict` — `FieldConflict` plus display dates)
- Test: `tests/profile_feature.rs`

**Interfaces:**
- Produces: `struct ResolvableConflict { file, plugin, key_path, local: Value, remote: Value, local_edited: Option<String>, remote_edited: Option<String> }`; serializes camelCase for the UI.
- Consumes: `merge_profile` (T2), `merge_base_with_remote`/`snapshot_at`/`field_edited_at` (T3/T4).

- [ ] **Step 1: Write the failing flow test** in `tests/profile_feature.rs`:

```rust
#[test]
fn diverged_pull_surfaces_field_conflicts_and_auto_merges_the_rest() {
    let env = SyncTestEnv::new();              // bare origin + two clones, mirror existing helpers
    env.machine_a().set_field("plugin-alt-tab", "opacity", json!(0.8));
    env.machine_a().set_field("plugin-launcher", "max_results", json!(8));
    env.machine_a().push();
    env.machine_b().set_field("plugin-alt-tab", "opacity", json!(0.5)); // clashes
    env.machine_b().set_field("plugin-lights", "theme", json!("cool")); // independent
    env.machine_b().commit_local();

    let status = env.machine_b().pull();       // remote is ahead AND b has local commits

    assert_eq!(status.health, SyncHealth::Attention);
    assert_eq!(status.conflict_count, 1);
    let c = env.machine_b().conflicts();
    assert_eq!(c[0].key_path, "opacity");
    // independent changes already merged into the working tree, not pending:
    assert_eq!(env.machine_b().field("plugin-launcher", "max_results"), json!(8));
    assert_eq!(env.machine_b().field("plugin-lights", "theme"), json!("cool"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray --test profile_feature diverged_pull_surfaces -- --nocapture`
Expected: FAIL — `conflict_count`/`conflicts()` absent; divergence still bare.

- [ ] **Step 3: Implement** — in `do_pull`, when `pull` returns `Diverged`:

```rust
let base_oid = repo.merge_base_with_remote()?;
let allow: &[&str] = &["manifest.json", "core/", "os/"];
let base = base_oid.map(|o| repo.snapshot_at(o, allow)).transpose()?.unwrap_or_default();
let local = repo.snapshot_at(local_oid, allow)?;
let remote = repo.snapshot_at(remote_oid, allow)?;
let result = merge_profile(&into_snapshot(base), &into_snapshot(local), &into_snapshot(remote));
if result.conflicts.is_empty() {
    write_merged_profile(&repo_path, &result.merged)?;
    repo.commit_all("merge remote", &SignatureSpec::default_for_app())?;
    repo.push(Some(&token))?;
    // clear incident, health Healthy
} else {
    // write merged (auto-resolved fields applied now), keep conflicts pending
    write_merged_profile(&repo_path, &result.merged)?;
    let resolvable = decorate_with_edit_times(&repo, local_oid, remote_oid, result.conflicts)?;
    // state.conflicts = resolvable; incident kind = Conflict; health Attention
}
```

`write_merged_profile` writes each merged file back with `serde_json::to_string_pretty` (so blame lines stay stable). `into_snapshot` wraps the `BTreeMap` in `ProfileSnapshot`. `decorate_with_edit_times` calls `field_edited_at` per conflict for each side. Add `conflict_count` to `SyncStatus` in `build_status`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p qol-tray --test profile_feature diverged_pull_surfaces -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/
git commit -m "feat(profile-sync): reconcile divergence via field merge on pull"
```

---

### Task 6: Apply resolved conflicts — backup, write, commit, push

**Files:**
- Modify: `src/features/profile/sync/service.rs`
- Test: `tests/profile_feature.rs`

**Interfaces:**
- Produces: `async fn resolve_conflicts(&self, choices: Vec<ConflictChoice>) -> Result<SyncActionResult>` where `struct ConflictChoice { file: String, key_path: String, side: Side }`, `enum Side { Mine, Remote }`.

- [ ] **Step 1: Write the failing flow test**

```rust
#[test]
fn resolving_conflicts_backs_up_both_sides_then_pushes() {
    let env = /* diverged state from Task 5 setup */;
    let before = env.machine_b().backup_count();
    env.machine_b().resolve(&[("plugin-alt-tab", "opacity", Side::Remote)]);
    assert_eq!(env.machine_b().field("plugin-alt-tab", "opacity"), json!(0.5));
    assert_eq!(env.machine_b().backup_count(), before + 1, "conflict snapshot written");
    assert!(env.machine_b().latest_backup_name().ends_with("-conflict.json"));
    let status = env.machine_b().status();
    assert_eq!(status.health, SyncHealth::Healthy);
    assert_eq!(status.conflict_count, 0);
    assert_eq!(env.origin().field("plugin-alt-tab", "opacity"), json!(0.5), "pushed");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray --test profile_feature resolving_conflicts -- --nocapture`
Expected: FAIL — `resolve()` path absent.

- [ ] **Step 3: Implement** — apply each `ConflictChoice` onto the pending merged document (override the auto-pick at `key_path` with the chosen side's value), snapshot both sides to `sync/backups/<ts>-conflict.json` (reuse existing backup writer), write merged, `commit_all`, `push`, clear `state.conflicts`/incident, health Healthy. Use the same `now_rfc3339`/timestamp source the codebase already uses (no `Date::now` in libs that forbid it — service is allowed).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p qol-tray --test profile_feature resolving_conflicts -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/
git commit -m "feat(profile-sync): apply per-field conflict choices and push"
```

---

### Task 7: HTTP — drop acknowledge, add conflicts endpoints

**Files:**
- Modify: `src/features/profile/http/sync.rs`, `src/features/profile/http/mod.rs`
- Modify: `src/features/profile/sync/service.rs` (delete `acknowledge_incident`)
- Test: `tests/profile_feature.rs`

**Interfaces:**
- Produces routes: `GET /sync/conflicts` → `Vec<ResolvableConflict>`; `POST /sync/conflicts/resolve` (body `{ choices: Vec<ConflictChoice> }`) → `SyncStatus`.
- Removes: `POST /sync/acknowledge` and `acknowledge_sync` handler.

- [ ] **Step 1: Write the failing test** — assert `GET /sync/conflicts` returns the pending list and `POST /sync/conflicts/resolve` flips health to Healthy; assert the acknowledge route is gone (`router has no /sync/acknowledge`, or a request 404s).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray --test profile_feature conflicts_endpoints -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Implement** — add the two handlers mirroring the existing `sync::*` handler shape; register in `http/mod.rs` route slice; delete `acknowledge_sync` + its route + `acknowledge_incident`. Verify no dangling references (`rg acknowledge apps/qol-tray/src apps/qol-tray/ui`).

- [ ] **Step 4: Run to verify pass + no dead symbols**

Run: `cargo test -p qol-tray --test profile_feature conflicts_endpoints -- --nocapture && cargo clippy -p qol-tray --all-targets --all-features -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/
git commit -m "feat(profile-sync): replace acknowledge with conflicts resolve endpoints"
```

---

### Task 8: Pull-before-push so clashes stay one field

**Files:**
- Modify: `src/features/profile/sync/service.rs` (`auto_push_if_dirty`, `manual_push`)
- Test: `tests/profile_feature.rs`

**Interfaces:** consumes `do_pull` reconcile (T5). No new public surface.

- [ ] **Step 1: Write the failing test** — machine A pushes; machine B (one local field change, non-clashing) calls push; assert B fast-forward-merges remote first and pushes cleanly with `conflict_count == 0` (no incident). Then a clashing variant asserts push aborts into a pending conflict instead of `NotFastForward`.

- [ ] **Step 2–4:** Run (FAIL → implement: push path runs a fetch+reconcile first; if conflicts result, return the conflict status instead of pushing → PASS).

Run: `cargo test -p qol-tray --test profile_feature pull_before_push -- --nocapture`

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/features/profile/sync/service.rs
git commit -m "fix(profile-sync): reconcile before push to keep clashes small"
```

---

### Task 9: Frontend API + conflict types

**Files:**
- Modify: `ui/views/profile/actions.js`
- Test: `node --check ui/views/profile/actions.js`

**Interfaces:**
- Produces: `fetchConflicts()` → array of `{ file, plugin, keyPath, local, remote, localEdited, remoteEdited }`; `resolveConflicts(choices)` where each choice is `{ file, keyPath, side: 'mine' | 'remote' }` → status.

- [ ] **Step 1:** Add the two functions next to the existing sync action calls, matching the established fetch/error pattern in `actions.js`.

```js
export async function fetchConflicts() {
  return apiGet('/sync/conflicts');
}
export async function resolveConflicts(choices) {
  return apiPost('/sync/conflicts/resolve', { choices });
}
```

(Use the actual `apiGet`/`apiPost` helpers already in that file — read it first and mirror them.)

- [ ] **Step 2:** `node --check ui/views/profile/actions.js` → no error.

- [ ] **Step 3: Commit**

```bash
git add apps/qol-tray/ui/views/profile/actions.js
git commit -m "feat(profile-sync-ui): add conflicts fetch/resolve API calls"
```

---

### Task 10: Resolver dive — stepper, sides, in-context diff, confirm

**Files:**
- Create: `ui/views/profile/conflict-resolver/view.js` (dive view)
- Create: `ui/views/profile/conflict-resolver/use-resolver.js` (state: current index, picks, derived counts)
- Create: `ui/styles/profile-conflicts.css` (token-based)
- Modify: dive registration sites per `qol-tray-page-creation` (register `profile-sync-conflicts` as a single-page dive; add the `data-dive-target` source on the Profile sync section)
- Test: `node --check` on each created JS file; manual run via the in-app Recompile button

**Interfaces:** consumes `fetchConflicts`/`resolveConflicts` (T9). Layout/flow per the approved prototype `.superpowers/brainstorm/45908-1782236566/content/conflict-resolver.html` (reference only — rebuild in token CSS + `Surface`/`ListRow`).

- [ ] **Step 1:** Read `qol-tray-page-creation` and an existing single-page dive (e.g. `profile-backup-detail`) to copy the registration contract exactly. List the 3–4 registration sites in the task notes before editing.

- [ ] **Step 2:** Build `use-resolver.js` — load conflicts on mount, hold `index` and `picks[]` (one `'mine'|'remote'|null` per conflict), expose `pick(side)`, `next()`, `prev()`, `allPicked`, and `summary()` (`{ keptMine, tookRemote }`).

- [ ] **Step 3:** Build `view.js` — `Surface` containing: header with `current / total`; the two selectable side panels (value + last-edited); the in-context config diff (conflicting key both ways, chosen side marked, auto-merged lines labelled); footer prev/next; and a confirm sub-view (tally + Apply → `resolveConflicts`). Pure render from state; no imperative DOM.

- [ ] **Step 4:** `profile-conflicts.css` using existing tokens (no raw hex; mirror `STYLE_GUIDE.md`).

- [ ] **Step 5:** `node --check` each file; build UI; click in via Recompile and step through a synthesized 2-conflict state.

- [ ] **Step 6: Commit**

```bash
git add apps/qol-tray/ui/views/profile/conflict-resolver/ apps/qol-tray/ui/styles/profile-conflicts.css apps/qol-tray/ui/<registration files>
git commit -m "feat(profile-sync-ui): conflict resolver dive (stepper + diff + confirm)"
```

---

### Task 11: Keyboard routing + entry from the sync section

**Files:**
- Modify: `ui/views/profile/conflict-resolver/view.js` (or its key-router), following `ui/views/profile/key-router.js`
- Modify: `ui/views/profile/view.js` (when `incident.kind === 'conflict'`, the primary action dives to `profile-sync-conflicts` instead of showing the dead Pull/Push/Acknowledge row)
- Test: `node --check`; manual keyboard walk-through

**Interfaces:** consumes the dive (T10). Keyboard map: ←/→ pick mine/remote, n/p (and ↑/↓) move, enter advance / on last+all-picked go to confirm, enter on confirm applies, esc leaves (incident stays pending).

- [ ] **Step 1:** Route keys through the existing profile key-router rather than a bare `addEventListener` (`preact-conventions`, `qol-tray-ui-systems`).
- [ ] **Step 2:** In `view.js`, branch the sync section: `kind === 'conflict'` → "Resolve N conflicts" dive entry; the legacy Pull/Push/Acknowledge row is removed for that state.
- [ ] **Step 3:** `node --check`; build; verify the whole flow is reachable with the keyboard only.
- [ ] **Step 4: Commit**

```bash
git add apps/qol-tray/ui/views/profile/
git commit -m "feat(profile-sync-ui): keyboard nav and conflict entry point"
```

---

### Task 12: End-to-end verification + cleanup

**Files:**
- Test: `tests/profile_feature.rs` (full round trip)

- [ ] **Step 1:** Add one end-to-end test: diverge → pull → resolve mixed (some mine, some remote) → assert merged document, backup written, push fast-forwards, both machines converge on a subsequent pull, health Healthy on both.

- [ ] **Step 2: Run the full verification stack** (per `qol-tray-feature-profile`):

```bash
cd apps/qol-tray
make build && make test && cargo build --features dev
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Expected: all green, no warnings on any path.

- [ ] **Step 3:** `rg -n acknowledge apps/qol-tray` → no functional references remain (only this plan/spec).

- [ ] **Step 4: Commit**

```bash
git add apps/qol-tray/tests/profile_feature.rs
git commit -m "test(profile-sync): end-to-end conflict resolve round trip"
```

---

## Self-Review

- **Spec coverage:** merge engine (T1–2), git/blame helpers + last-edited decision (T3–4), reconcile-on-pull + auto-merge (T5), apply+backup+push (T6), acknowledge removal + endpoints + conflict transport decision (T7), pull-before-push prevention (T8), resolver dive in qol-tray style + entry (T9–11), edge cases via tests + e2e (T5/6/12). All spec sections map to a task.
- **Placeholders:** engine/git/flow code is concrete; UI tasks name exact files and reference the precise existing patterns to copy (page-creation, key-router, STYLE_GUIDE) rather than inventing unseen component APIs — the one honest dependency is reading those patterns first, called out as Step 1 in T10/T11.
- **Type consistency:** `FieldConflict`/`FileMerge`/`merge_json` (T1) → `ProfileSnapshot`/`ProfileMerge`/`merge_profile` (T2) → `ResolvableConflict`/`ConflictChoice`/`Side` (T5–6) → camelCase `keyPath`/`side` at the JS boundary (T9). Names are stable across tasks.
