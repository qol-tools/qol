# State & Lifecycle (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate dev from prod state, move ephemeral runtime data to `/tmp/qol-tray/`, add lifecycle cleanup, and migrate legacy file locations.

**Architecture:** Dev files move into a `dev/` subdirectory gated behind `#[cfg(feature = "dev")]`. PID tracking switches from a single shared file to per-plugin files in `/tmp/qol-tray/pids/`. Startup wipes `/tmp/qol-tray/` for crash recovery. Migration handles the transition from old file locations.

**Tech Stack:** Rust, tokio (async fs ops), tempfile (already in dev-dependencies)

**Spec:** `docs/superpowers/specs/2026-03-21-state-logging-design.md` (Phase 1)

**Spec deviations:**
- Staging dirs remain in `plugins/` (not `/tmp/qol-tray/staging/`) because `tokio::fs::rename()` fails across filesystems (EXDEV). Cleanup contract still applies: clean on success AND failure, clean stale on startup. `init_runtime_dirs` does NOT create a `staging/` subdirectory.
- Plugin cache moves to `/tmp/qol-tray/cache/` as specified. This means a GitHub API call on every boot to repopulate. Acceptable because the cache has a TTL anyway and the call is lightweight with a 2s timeout.

**Ordering note:** Task 5 (migration) MUST be deployed together with Task 2 (dev dir restructure). If Task 2 is deployed alone, dev-links would be read from `dev/links.json` while the old `dev-links.json` still exists at root. Task 5's migration moves the old files to the new location. Commit both in the same release.

---

### Task 1: Runtime Directory Paths

Add `/tmp/qol-tray/` path constants and init/wipe helpers.

**Files:**
- Modify: `src/paths.rs`
- Test: `src/paths.rs` (existing test module)

- [ ] **Step 1: Write tests for runtime paths and init_runtime_dirs**

Add to the existing `tests` module in `src/paths.rs`:

```rust
#[test]
fn runtime_dir_is_under_tmp() {
    let dir = runtime_dir();
    assert!(
        dir.starts_with("/tmp"),
        "runtime dir {:?} should be under /tmp",
        dir
    );
    assert!(dir.ends_with("qol-tray"));
}

#[test]
fn runtime_subdirs_have_correct_suffixes() {
    let cases = [
        (runtime_pids_dir(), "pids"),
        (runtime_cache_dir(), "cache"),
    ];
    for (path, suffix) in cases {
        assert!(
            path.ends_with(suffix),
            "path {:?} should end with {}",
            path,
            suffix
        );
    }
}

#[test]
fn init_runtime_dirs_creates_fresh_structure() {
    let test_dir = PathBuf::from("/tmp/qol-tray-test-init");
    let pids = test_dir.join("pids");
    let cache = test_dir.join("cache");

    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&pids).unwrap();
    std::fs::write(pids.join("stale.pid"), "999").unwrap();

    init_runtime_dirs_at(&test_dir).unwrap();

    assert!(pids.is_dir(), "pids dir should exist");
    assert!(cache.is_dir(), "cache dir should exist");
    assert!(
        !pids.join("stale.pid").exists(),
        "stale files should be wiped"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib paths::tests -- --nocapture`
Expected: FAIL — functions not found

- [ ] **Step 3: Implement runtime path functions**

Add to `src/paths.rs` before the `open_url` function:

```rust
const RUNTIME_DIR: &str = "/tmp/qol-tray";

pub fn runtime_dir() -> PathBuf {
    PathBuf::from(RUNTIME_DIR)
}

pub fn runtime_pids_dir() -> PathBuf {
    runtime_dir().join("pids")
}

pub fn runtime_cache_dir() -> PathBuf {
    runtime_dir().join("cache")
}

pub fn init_runtime_dirs() -> Result<()> {
    init_runtime_dirs_at(&runtime_dir())
}

fn init_runtime_dirs_at(base: &Path) -> Result<()> {
    if base.exists() {
        fs::remove_dir_all(base)
            .with_context(|| format!("Failed to wipe runtime dir {}", base.display()))?;
    }
    for subdir in ["pids", "cache"] {
        fs::create_dir_all(base.join(subdir))
            .with_context(|| format!("Failed to create runtime subdir {}", subdir))?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib paths::tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/paths.rs
git commit -m "feat: add runtime directory paths under /tmp/qol-tray"
```

---

### Task 2: Dev Directory Restructure

Move dev file constants to `dev/` subdirectory. Gate plugin log controls behind `#[cfg(feature = "dev")]`.

**Files:**
- Modify: `src/dev/linking/store.rs` — path constants, remove comment on line 38-39
- Modify: `src/dev/build/types.rs` — `DEV_BUILD_STATE_FILE` constant
- Modify: `src/dev/build/fingerprint_store.rs` — temp file path
- Modify: `src/logging/control.rs` — path constant, feature gate functions
- Modify: `src/logging/mod.rs` — feature gate re-exports
- Modify: `src/plugins/daemon_lifecycle/spawn.rs` — gate `configure_log_relay`
- Test: existing tests in `src/dev/linking.rs`, `src/logging/control.rs`

- [ ] **Step 1: Update dev-links path constants**

In `src/dev/linking/store.rs`, change:

```rust
fn dev_links_path(config_dir: &Path) -> PathBuf {
    config_dir.join("dev/links.json")
}

fn temp_dev_links_path(config_dir: &Path) -> PathBuf {
    config_dir.join("dev/.links.json.tmp")
}
```

In `save_dev_links`, add `dev/` dir creation before writing:

```rust
fn save_dev_links(config_dir: &Path, links: &HashMap<String, PathBuf>) -> Result<(), String> {
    let path = dev_links_path(config_dir);
    let tmp_path = temp_dev_links_path(config_dir);
    let dev_dir = config_dir.join("dev");
    std::fs::create_dir_all(&dev_dir)
        .map_err(|e| format!("Failed to create dev directory: {}", e))?;
    let content = serde_json::to_string_pretty(links)
        .map_err(|e| format!("Failed to serialize dev-links: {}", e))?;
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write dev-links temp file: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to finalize dev/links.json: {}", e))
}
```

Also remove the comment on lines 38-39 of `create_link` (CLAUDE.md: no comments).

- [ ] **Step 2: Update build fingerprint constants**

In `src/dev/build/types.rs`, change:

```rust
pub(crate) const DEV_BUILD_STATE_FILE: &str = "dev/build-fingerprints.json";
```

In `src/dev/build/fingerprint_store.rs`, update `save_build_fingerprints`:

```rust
pub fn save_build_fingerprints(
    config_dir: &Path,
    fingerprints: &HashMap<String, String>,
) -> Result<(), String> {
    let dev_dir = config_dir.join("dev");
    std::fs::create_dir_all(&dev_dir).map_err(|e| {
        format!("Failed to create dev directory {}: {}", dev_dir.display(), e)
    })?;
    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let tmp_path = config_dir.join("dev/.build-fingerprints.tmp");
    let state = BuildFingerprintState {
        fingerprints: fingerprints.clone(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialize build fingerprints: {}", e))?;
    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write build fingerprint temp file: {}", e))?;
    std::fs::rename(&tmp_path, &state_path)
        .map_err(|e| format!("Failed to finalize build fingerprint file: {}", e))
}
```

- [ ] **Step 3: Gate plugin log controls behind dev feature**

In `src/logging/control.rs`, add `#[cfg(feature = "dev")]` to:

- `LOG_CONTROL_STATE_FILE` constant (line 5)
- `PluginLogControlFile` struct (line 15-19)
- `load_all_plugin_controls` (line 21)
- `save_all_plugin_controls` (line 32)
- `save_controls_file` (line 42) — shared helper, but only called by dev-gated functions; gate to avoid dead code warning in prod
- `load_plugin_control` (line 63)
- `load_plugin_control_from_shared_config` (line 69)
- `upsert_plugin_control` (line 76)
- `upsert_control_entry` (line 86) — only called by gated functions
- `normalize_patterns` (line 99) — only called by gated functions
- The existing test `upsert_plugin_control_roundtrip_and_clear` (line 162) — add `#[cfg(feature = "dev")]` to the test

Keep ungated: `LogControl` struct, `matches_any_pattern` (used by relay in both modes).

Update `CORE_LOG_CONTROL_STATE_FILE` path:

```rust
#[cfg(feature = "dev")]
const CORE_LOG_CONTROL_STATE_FILE: &str = "dev/core-log-controls.json";
```

- [ ] **Step 4: Update logging/mod.rs re-exports**

In `src/logging/mod.rs`, gate the plugin control re-exports:

```rust
#[cfg(feature = "dev")]
pub use control::{
    load_all_plugin_controls, load_plugin_control, load_plugin_control_from_shared_config,
    save_all_plugin_controls, upsert_plugin_control,
};

pub use control::LogControl;
```

`LogControl` stays ungated. The five plugin control functions are gated.

- [ ] **Step 5: Gate configure_log_relay in spawn.rs**

In `src/plugins/daemon_lifecycle/spawn.rs`, gate both the call site AND the function definition:

In `spawn_daemon` (line 18), replace:
```rust
let relay_patterns = configure_log_relay(plugin, &mut command);
```
with:
```rust
#[cfg(feature = "dev")]
let relay_patterns = configure_log_relay(plugin, &mut command);
#[cfg(not(feature = "dev"))]
let relay_patterns: Vec<String> = {
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    Vec::new()
};
```

Gate the entire `configure_log_relay` function definition:
```rust
#[cfg(feature = "dev")]
fn configure_log_relay(plugin: &Plugin, command: &mut Command) -> Vec<String> {
    ...
}
```

- [ ] **Step 6: Run tests to verify**

Run: `cargo test --lib --features dev -- --nocapture` (dev tests)
Run: `cargo test --lib -- --nocapture` (prod tests — verify compilation, no dead code)
Expected: PASS for both

- [ ] **Step 7: Commit**

```bash
git add src/dev/linking/store.rs src/dev/build/types.rs src/dev/build/fingerprint_store.rs src/logging/control.rs src/logging/mod.rs src/plugins/daemon_lifecycle/spawn.rs
git commit -m "refactor: move dev state files to dev/ subdirectory"
```

---

### Task 3: PID Tracking Overhaul

Replace single `.daemon-pids` file with per-plugin PID files in `/tmp/qol-tray/pids/`.

**Files:**
- Modify: `src/plugins/daemon_tracker/mod.rs` — rewrite save/load/kill
- Modify: `src/plugins/manager/loading.rs` — update finalize_load
- Modify: `src/plugins/manager/runtime.rs` — update persist_daemon_pids, stop_all_plugins
- Test: new tests in `src/plugins/daemon_tracker/mod.rs`

- [ ] **Step 1: Write tests for per-plugin PID files**

Add a test module to `src/plugins/daemon_tracker/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_and_load_pid_roundtrip() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);

        let pid_file = tmp.path().join("foo.pid");
        assert!(pid_file.exists());

        let content = std::fs::read_to_string(&pid_file).unwrap();
        assert_eq!(content.trim(), "12345");
    }

    #[test]
    fn remove_plugin_pid_deletes_file() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);
        remove_plugin_pid(tmp.path(), "foo");

        assert!(!tmp.path().join("foo.pid").exists());
    }

    #[test]
    fn remove_plugin_pid_noop_when_missing() {
        let tmp = TempDir::new().unwrap();
        remove_plugin_pid(tmp.path(), "nonexistent");
    }

    #[test]
    fn list_tracked_pids_returns_all_entries() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 111);
        save_plugin_pid(tmp.path(), "b", 222);

        let mut pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        pids.sort_by_key(|(id, _)| id.clone());

        assert_eq!(pids.len(), 2);
        assert_eq!(pids[0], ("a".to_string(), 111));
        assert_eq!(pids[1], ("b".to_string(), 222));
    }

    #[test]
    fn list_tracked_pids_skips_corrupt_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("bad.pid"), "not-a-number").unwrap();
        save_plugin_pid(tmp.path(), "good", 42);

        let pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], ("good".to_string(), 42));
    }

    #[test]
    fn clear_all_pids_removes_all_pid_files() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 1);
        save_plugin_pid(tmp.path(), "b", 2);
        clear_all_pids(tmp.path());

        assert!(list_tracked_pids(tmp.path()).next().is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib plugins::daemon_tracker::tests -- --nocapture`
Expected: FAIL — functions not found

- [ ] **Step 3: Implement per-plugin PID functions**

Add new functions to `src/plugins/daemon_tracker/mod.rs`. Use `u32` consistently (matching `Plugin::daemon_pid()` return type):

```rust
pub fn save_plugin_pid(pids_dir: &Path, plugin_id: &str, pid: u32) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::write(&path, pid.to_string());
}

pub fn remove_plugin_pid(pids_dir: &Path, plugin_id: &str) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::remove_file(&path);
}

pub fn list_tracked_pids(pids_dir: &Path) -> impl Iterator<Item = (String, u32)> {
    std::fs::read_dir(pids_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "pid" {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let pid: u32 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
            Some((id, pid))
        })
}

pub fn clear_all_pids(pids_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(pids_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "pid") {
            let _ = std::fs::remove_file(&path);
        }
    }
}
```

- [ ] **Step 4: Update callers to use per-plugin PID tracking**

In `src/plugins/manager/loading.rs`, replace `finalize_load`:

```rust
fn finalize_load(manager: &mut PluginManager, loaded: LoadedPlugins) {
    register_plugins(manager, loaded.plugins);
    runtime::persist_daemon_pids(manager);
    runtime::sync_ignore_pids(manager);
}
```

In `src/plugins/manager/runtime.rs`, replace `persist_daemon_pids`:

```rust
pub(super) fn persist_daemon_pids(manager: &PluginManager) {
    let pids_dir = crate::paths::runtime_pids_dir();
    for plugin in manager.plugins.values() {
        if let Some(pid) = plugin.daemon_pid() {
            super::super::daemon_tracker::save_plugin_pid(&pids_dir, &plugin.id, pid);
        }
    }
}
```

Replace `stop_all_plugins`:

```rust
fn stop_all_plugins(manager: &mut PluginManager) {
    kill_all_plugin_processes();
    stop_plugin_daemons(manager);
    manager.plugins.clear();
    super::super::daemon_tracker::clear_all_pids(&crate::paths::runtime_pids_dir());
    super::super::daemon_tracker::kill_orphan_daemons();
}
```

- [ ] **Step 5: Update kill_from_pid_files to use new location**

In `src/plugins/daemon_tracker/mod.rs`, update `kill_from_pid_files` to scan `/tmp/qol-tray/pids/` plus legacy:

```rust
#[cfg(unix)]
pub(crate) fn kill_from_pid_files() {
    let roots = ManagedRoots::load();
    let pids_dir = crate::paths::runtime_pids_dir();
    for (_, pid) in list_tracked_pids(&pids_dir) {
        kill_pid_if_managed(&(pid as i32).to_string(), &roots);
    }
    clear_all_pids(&pids_dir);

    if let Some(legacy) = legacy_daemon_pids_path() {
        if legacy.exists() {
            process_pid_file(&legacy, &roots);
        }
    }
}
```

Rename `daemon_pids_path()` to `legacy_daemon_pids_path()` to signal its transitional role.

- [ ] **Step 6: Run tests to verify**

Run: `cargo test --lib plugins::daemon_tracker -- --nocapture`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/plugins/daemon_tracker/mod.rs src/plugins/manager/loading.rs src/plugins/manager/runtime.rs
git commit -m "refactor: per-plugin PID files in /tmp/qol-tray/pids"
```

---

### Task 4: Plugin Cache Relocation

Move `.plugin-cache.json` from config dir to `/tmp/qol-tray/cache/`.

**Files:**
- Modify: `src/paths.rs` — update `plugin_cache_path()`
- Test: update existing paths test

- [ ] **Step 1: Update plugin_cache_path and verify callers**

In `src/paths.rs`, change:

```rust
pub fn plugin_cache_path() -> Result<PathBuf> {
    Ok(runtime_cache_dir().join("plugin-cache.json"))
}
```

Verify callers handle the return type correctly. `src/features/plugin_store/github/cache.rs:8-10` calls `paths::plugin_cache_path().ok()` — still works (always returns Some now).

- [ ] **Step 2: Update paths test**

Remove `(plugin_cache_path(), ".plugin-cache.json")` from `paths_have_correct_suffixes`. Add:

```rust
#[test]
fn plugin_cache_path_is_under_runtime() {
    let path = plugin_cache_path().unwrap();
    assert!(
        path.starts_with("/tmp/qol-tray"),
        "cache path {:?} should be under /tmp/qol-tray",
        path
    );
}
```

- [ ] **Step 3: Run tests to verify**

Run: `cargo test --lib paths::tests -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/paths.rs
git commit -m "refactor: move plugin cache to /tmp/qol-tray/cache"
```

---

### Task 5: Migration Logic

Handle transition from old file locations to new ones on first run after upgrade.

**Files:**
- Create: `src/migration.rs`
- Modify: `src/lib.rs` — add `pub mod migration`

- [ ] **Step 1: Create migration module with implementation and tests**

Create `src/migration.rs`:

```rust
use anyhow::Result;
use std::path::Path;

pub fn run_migration(config_dir: &Path) -> Result<()> {
    migrate_dev_files(config_dir);
    clean_legacy_ephemeral(config_dir);
    clean_stale_staging(config_dir);
    Ok(())
}

fn migrate_dev_files(config_dir: &Path) {
    let dev_dir = config_dir.join("dev");
    let migrations = [
        ("dev-links.json", "links.json"),
        ("dev-build-fingerprints.json", "build-fingerprints.json"),
        ("dev-core-log-controls.json", "core-log-controls.json"),
        ("dev-plugin-log-controls.json", "plugin-log-controls.json"),
    ];

    let any_exists = migrations
        .iter()
        .any(|(old, _)| config_dir.join(old).exists());

    if !any_exists {
        return;
    }

    let _ = std::fs::create_dir_all(&dev_dir);

    for (old_name, new_name) in migrations {
        let old = config_dir.join(old_name);
        let new = dev_dir.join(new_name);
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

fn clean_legacy_ephemeral(config_dir: &Path) {
    for name in [".daemon-pids", ".plugin-cache.json"] {
        let _ = std::fs::remove_file(config_dir.join(name));
    }
}

fn clean_stale_staging(config_dir: &Path) {
    let plugins_dir = config_dir.join("plugins");
    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_stale_staging_dir(&name_str) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn is_stale_staging_dir(name: &str) -> bool {
    name.starts_with('.')
        && (name.contains(".installing.")
            || name.contains(".updating.")
            || name.contains(".backup."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrate_dev_files_moves_to_dev_subdir() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join("dev-links.json"), "{}").unwrap();
        std::fs::write(cfg.join("dev-build-fingerprints.json"), "{}").unwrap();

        run_migration(cfg).unwrap();

        assert!(!cfg.join("dev-links.json").exists());
        assert!(cfg.join("dev/links.json").exists());
        assert!(!cfg.join("dev-build-fingerprints.json").exists());
        assert!(cfg.join("dev/build-fingerprints.json").exists());
    }

    #[test]
    fn migrate_dev_files_skips_when_target_exists() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::create_dir_all(cfg.join("dev")).unwrap();
        std::fs::write(cfg.join("dev-links.json"), r#"{"old": true}"#).unwrap();
        std::fs::write(cfg.join("dev/links.json"), r#"{"new": true}"#).unwrap();

        run_migration(cfg).unwrap();

        let content = std::fs::read_to_string(cfg.join("dev/links.json")).unwrap();
        assert!(content.contains("new"), "should not overwrite existing");
    }

    #[test]
    fn migrate_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join("dev-links.json"), "{}").unwrap();

        run_migration(cfg).unwrap();
        run_migration(cfg).unwrap();

        assert!(cfg.join("dev/links.json").exists());
    }

    #[test]
    fn clean_legacy_ephemeral_removes_old_files() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join(".daemon-pids"), "123").unwrap();
        std::fs::write(cfg.join(".plugin-cache.json"), "{}").unwrap();

        run_migration(cfg).unwrap();

        assert!(!cfg.join(".daemon-pids").exists());
        assert!(!cfg.join(".plugin-cache.json").exists());
    }

    #[test]
    fn clean_stale_staging_removes_orphan_dirs() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        let plugins = cfg.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();

        std::fs::create_dir_all(plugins.join(".foo.installing.123.456")).unwrap();
        std::fs::create_dir_all(plugins.join(".bar.updating.789.012")).unwrap();
        std::fs::create_dir_all(plugins.join(".baz.backup.111.222")).unwrap();
        std::fs::create_dir_all(plugins.join("real-plugin")).unwrap();

        run_migration(cfg).unwrap();

        assert!(!plugins.join(".foo.installing.123.456").exists());
        assert!(!plugins.join(".bar.updating.789.012").exists());
        assert!(!plugins.join(".baz.backup.111.222").exists());
        assert!(plugins.join("real-plugin").exists());
    }

    #[test]
    fn is_stale_staging_dir_cases() {
        let cases = [
            (".foo.installing.123.456", true),
            (".bar.updating.789.012", true),
            (".baz.backup.111.222", true),
            ("real-plugin", false),
            (".hidden-but-not-staging", false),
            (".foo.installing", false),
        ];
        for (name, expected) in cases {
            assert_eq!(
                is_stale_staging_dir(name),
                expected,
                "is_stale_staging_dir({:?})",
                name
            );
        }
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add `pub mod migration;` to `src/lib.rs`.

- [ ] **Step 3: Run tests to verify**

Run: `cargo test --lib migration -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/migration.rs src/lib.rs
git commit -m "feat: add state migration for dev dir restructure and stale cleanup"
```

---

### Task 6: Wire Into Startup Sequence

Insert runtime init, migration, and wipe into the startup path in `main.rs`.

**Files:**
- Modify: `src/main.rs` — add init calls after instance check

- [ ] **Step 1: Add startup calls**

In `src/main.rs`, after the `is_already_running()` check (line 32) and before `doctor::auto_fix_startup()` (line 34), add:

```rust
if let Err(e) = qol_tray::paths::init_runtime_dirs() {
    log::error!("Failed to initialize runtime directories: {}", e);
}

if let Ok(config_dir) = qol_tray::paths::shared_config_dir() {
    if let Err(e) = qol_tray::migration::run_migration(&config_dir) {
        log::error!("Migration failed: {}", e);
    }
}
```

Startup sequence:
1. Instance check (existing)
2. `init_runtime_dirs()` — wipe and recreate `/tmp/qol-tray/` (NEW)
3. `run_migration()` — move dev files, clean legacy ephemeral, clean stale staging (NEW)
4. Doctor fixes (existing)
5. Plugin loading (existing)

- [ ] **Step 2: Verify shutdown cleanup**

The shutdown path `tray::platform::mod.rs:158` -> `manager.shutdown()` -> `runtime::shutdown()` -> `stop_all_plugins()` already calls `clear_all_pids` (from Task 3). No additional changes needed.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire runtime init and migration into startup sequence"
```

---

### Task 7: Update dev_registry Migration Path

The `migrate_symlinks_to_registry` in `dev_registry.rs` writes to the old `dev-links.json` path. Update it to use the new `dev/links.json` path.

**Files:**
- Modify: `src/plugins/manager/dev_registry.rs`

- [ ] **Step 1: Update symlink migration target path**

In `src/plugins/manager/dev_registry.rs`, line 23, change:

```rust
let dev_links_path = config_dir.join("dev/links.json");
```

Update `write_dev_links` to create `dev/` dir:

```rust
#[cfg(feature = "dev")]
fn write_dev_links(dev_links_path: &Path, migrated: &HashMap<String, PathBuf>) {
    if let Some(parent) = dev_links_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(content) = serde_json::to_string_pretty(migrated) else {
        return;
    };
    let _ = std::fs::write(dev_links_path, content);
    log::info!("Migrated {} symlinks to dev/links.json", migrated.len());
}
```

- [ ] **Step 2: Run dev-links tests**

Run: `cargo test --lib --features dev dev::linking -- --nocapture`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/plugins/manager/dev_registry.rs
git commit -m "refactor: update symlink migration to use dev/links.json path"
```

---

### Task 8: Remove Legacy PID Functions

Clean up old bulk PID functions now that all callers use per-plugin PID tracking.

**Files:**
- Modify: `src/plugins/daemon_tracker/mod.rs` — remove old functions

- [ ] **Step 1: Verify no remaining callers of old API**

Search for `save_daemon_pids` usage. All callers should have been migrated in Task 3.

- [ ] **Step 2: Remove old functions**

Remove from `src/plugins/daemon_tracker/mod.rs`:
- `save_daemon_pids(pids: &[u32])` — replaced by `save_plugin_pid`
- `daemon_pid_files()` — scanned installs dirs, no longer needed

Rename `daemon_pids_path()` to `legacy_daemon_pids_path()` and keep it only for the legacy fallback in `kill_from_pid_files`.

- [ ] **Step 3: Run full test suite**

Run: `cargo test --lib -- --nocapture`
Run: `cargo test --lib --features dev -- --nocapture`
Expected: PASS for both

- [ ] **Step 4: Commit**

```bash
git add src/plugins/daemon_tracker/mod.rs
git commit -m "refactor: remove legacy bulk PID tracking functions"
```
