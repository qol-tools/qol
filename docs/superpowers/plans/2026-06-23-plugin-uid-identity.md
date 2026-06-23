# Plugin UID Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make plugin identity an immutable, published `uid` so renames never strand durable/synced state.

**Architecture:** Add a frozen `uid` to each `plugin.toml`. Thread a `PluginUid` newtype through the loader, registry, profile lock, hotkeys, and plugin-config storage so all durable/synced state is uid-keyed (opaque on disk). A PreFlight `FileMigration` (`v3_19_to_v3_20_plugin_uid`) re-keys existing installs from id→uid and coalesces duplicates, driven by local manifests plus a built-in core `legacy_id→uid` table so it works even where a plugin isn't installed.

**Tech Stack:** Rust, `git2` (unrelated), `toml`, `serde`; crates `apps/qol-tray` and `libs/qol-migrations`.

**Spec:** `docs/superpowers/specs/2026-06-23-plugin-uid-identity-design.md`

## Global Constraints

- `uid` is **published and frozen**: authored once into `plugin.toml`, identical bytes on every PC. NEVER generate a uid per-install.
- All durable/synced state is **uid-keyed** (lock entries, hotkey bindings, plugin-config filenames, registry). Opaque on disk; human names resolved via a runtime index.
- `RUSTFLAGS=-D warnings` on **all** platforms. Before adding any `pub`/`use`/`#[cfg(target_os)]` to a shared module, confirm every backend consumes it (`qol-arch-cross-platform`).
- `qol-migrations` is consumed via the workspace path dependency. Do not change to `git`/`branch`/sibling path.
- Migration is a PreFlight `FileMigration`: archive-before-write, idempotent, registered in `PreFlightRegistry::current()` **after** `V3_18ToV3_19DeclaredPluginId`. Bump `qol-migrations` minor version.
- Code is comment-free. Atomic conventional commits, no AI attribution.
- uid format: opaque token (uuidv4), authored as a literal string.

## File Structure

| File | Responsibility |
|---|---|
| `apps/qol-tray/src/plugins/manifest.rs` (or where `PluginManifest`/`PluginId` live) | add `PluginUid` newtype + `uid: Option<PluginUid>` on the `[plugin]` section |
| `apps/qol-tray/src/plugins/identity_index.rs` (new) | `PluginIdentityIndex`: `uid -> { id, name }`, built at load |
| `apps/qol-tray/src/plugins/loader/manifest_loader.rs` | key loaded plugin by uid; populate index |
| `apps/qol-tray/src/plugins/registry/mod.rs` | registry entry keyed by uid |
| `apps/qol-tray/src/features/profile/core/types.rs` | `PluginLockEntry.uid` |
| `apps/qol-tray/src/features/profile/core/plugins_lock.rs` | build/coalesce lock by uid |
| `apps/qol-tray/src/hotkeys/types.rs` | `HotkeyBinding.plugin_uid` |
| `apps/qol-tray/src/features/profile/scope_store.rs` | plugin-config files keyed by uid |
| `apps/qol-tray/src/doctor/checks/...` | `qol doctor` uid↔name decode table |
| `libs/qol-migrations/src/v3_19_to_v3_20_plugin_uid/mod.rs` (new) | the re-key migration + `LEGACY_ID_TO_UID` table |
| `plugins/*/plugin.toml` (~12) | author frozen `uid` |
| `docs/plugin-contract.md` | document `uid` |

## Build / test commands

- qol-tray: `cargo build -p qol-tray --features dev` ; `cargo test -p qol-tray <name>`
- migrations: `cargo test -p qol-migrations <name>`
- before any commit: `cargo fmt` ; `cargo clippy -p <crate> --all-targets -- -D warnings`

---

### Task 1: `PluginUid` newtype + manifest field

**Files:**
- Modify: `apps/qol-tray/src/plugins/manifest.rs` (the module that defines `PluginId` and `PluginManifest`; re-exported at `plugins/mod.rs:22`)
- Test: same file `#[cfg(test)]`

**Interfaces:**
- Produces: `pub struct PluginUid(String)` with `PluginUid::new(impl Into<String>)`, `as_str(&self) -> &str`, `Deserialize`/`Serialize`, `Clone`, `Eq`, `Hash`. `PluginManifest.plugin.uid: Option<PluginUid>`.

- [ ] **Step 1: Failing test** - mirror the existing `PluginId` tests; assert a manifest with `[plugin] uid = "u-123"` parses to `Some(PluginUid("u-123"))` and one without parses to `None`.

```rust
#[test]
fn manifest_parses_optional_uid() {
    let toml = "[plugin]\nid = \"plugin-x\"\nuid = \"u-123\"\nname = \"X\"\ndescription = \"\"\nversion = \"1.0.0\"\n";
    let m = parse_manifest(toml).unwrap();
    assert_eq!(m.plugin.uid.as_ref().map(|u| u.as_str()), Some("u-123"));

    let toml_no_uid = "[plugin]\nid = \"plugin-x\"\nname = \"X\"\ndescription = \"\"\nversion = \"1.0.0\"\n";
    assert_eq!(parse_manifest(toml_no_uid).unwrap().plugin.uid, None);
}
```

- [ ] **Step 2: Run, expect FAIL** - `cargo test -p qol-tray manifest_parses_optional_uid` → fails (no `uid` field).
- [ ] **Step 3: Implement** - add `PluginUid` by copying the `PluginId` newtype block verbatim and renaming; add `#[serde(default)] pub uid: Option<PluginUid>` to the `[plugin]` section struct.
- [ ] **Step 4: Run, expect PASS.** Then `cargo fmt` + `cargo clippy -p qol-tray --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** - `feat(plugins): add frozen PluginUid manifest field`

---

### Task 2: Author frozen uids into core manifests + the legacy table source

**Files:**
- Modify: each `plugins/*/plugin.toml` (launcher, qol-shot, alt-tab, keyremap, lights, os-themes, window-actions, pointz, cli-sessions, removeapp, ide-checkout; `plugin-template` gets a commented placeholder, not a real uid)
- Create: `libs/qol-migrations/src/v3_19_to_v3_20_plugin_uid/legacy_table.rs` with `pub const LEGACY_ID_TO_UID: &[(&str, &str)]`

**Interfaces:**
- Produces: `LEGACY_ID_TO_UID` mapping every current core id **and historical id** to its plugin's uid. MUST include `("plugin-screen-recorder", <qol-shot uid>)` and `("qol-shot", <qol-shot uid>)` pointing at the same uid.

- [ ] **Step 1: Mint uids** - generate one uuidv4 per core plugin (offline, e.g. `uuidgen`). Record the id→uid mapping.
- [ ] **Step 2: Author** - add `uid = "<uuid>"` under `[plugin]` in each core `plugin.toml`.
- [ ] **Step 3: Build the table** - write `LEGACY_ID_TO_UID` from the same mapping; add the screen-recorder→qol-shot historical alias.
- [ ] **Step 4: Test** - a `qol-migrations` test asserting the table has no duplicate keys and that `plugin-screen-recorder` and `qol-shot` map to the same uid.

```rust
#[test]
fn legacy_table_is_unique_and_aliases_screen_recorder() {
    use std::collections::HashMap;
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for (id, uid) in LEGACY_ID_TO_UID { assert!(seen.insert(id, uid).is_none(), "dup id {id}"); }
    assert_eq!(seen.get("plugin-screen-recorder"), seen.get("qol-shot"));
}
```

- [ ] **Step 5: Run, expect PASS.** Commit - `chore(plugins): author frozen uids and legacy id→uid table`

---

### Task 3: `PluginIdentityIndex` + uid-keyed loader

**Files:**
- Create: `apps/qol-tray/src/plugins/identity_index.rs`
- Modify: `apps/qol-tray/src/plugins/loader/manifest_loader.rs`, `apps/qol-tray/src/plugins/mod.rs`

**Interfaces:**
- Consumes: `PluginManifest.plugin.uid` (Task 1).
- Produces: `pub struct PluginIdentityIndex` with `insert(uid: PluginUid, id: PluginId, name: String)`, `display_for(&self, &PluginUid) -> Option<&PluginDisplay>`, `uid_for_legacy_id(&self, &str) -> Option<&PluginUid>`. Loaded `Plugin` exposes `uid(&self) -> &PluginUid` (falls back to a uid derived from the id only when manifest uid is absent during the migration window).

- [ ] **Step 1: Failing test** - build an index from two fake plugins; assert `display_for(uid)` returns the right `{id,name}` and `uid_for_legacy_id(old_id)` resolves once an entry declares it.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** the index struct + populate it in `load_plugin_with_source` after `parse_manifest`. Key the loaded plugin set by uid (replace the `PluginId`-keyed map). Where `manifest.plugin.uid` is `None` (pre-migration), synthesize a transitional uid equal to the id string so nothing breaks before the migration runs.
- [ ] **Step 4: Run, expect PASS** + fmt + clippy.
- [ ] **Step 5: Commit** - `feat(plugins): key loaded plugins by uid via identity index`

---

### Task 4: Lock entries keyed by uid

**Files:**
- Modify: `apps/qol-tray/src/features/profile/core/types.rs` (`PluginLockEntry`), `apps/qol-tray/src/features/profile/core/plugins_lock.rs`

**Interfaces:**
- Consumes: `Plugin::uid` (Task 3).
- Produces: `PluginLockEntry { uid: PluginUid, id: String /* display */, repo_url, version, platforms }`. `build_plugins_lock` keys by uid; the preserved-unsupported path preserves by uid; **coalesce** entries sharing a uid (prefer installed/most-recent metadata).

- [ ] **Step 1: Failing test** - feed `build_plugins_lock` two inputs that resolve to the same uid (a preserved old entry + a loaded new one); assert the result has exactly one entry with that uid.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** uid keying + coalesce in `build_plugins_lock`; keep `platforms`-preservation but dedupe by uid.
- [ ] **Step 4: Run, expect PASS** + fmt + clippy.
- [ ] **Step 5: Commit** - `feat(profile): key plugin lock by uid and coalesce duplicates`

---

### Task 5: Hotkey bindings keyed by uid

**Files:**
- Modify: `apps/qol-tray/src/hotkeys/types.rs` (`HotkeyBinding`), `apps/qol-tray/src/hotkeys/planning.rs`, `apps/qol-tray/src/hotkeys/manager.rs` (render/diff)

**Interfaces:**
- Consumes: `PluginIdentityIndex` (display), `PluginUid`.
- Produces: `HotkeyBinding { plugin_uid: PluginUid, action, key, enabled, id }`. Planning matches available actions by uid; display resolves uid→name via the index.

- [ ] **Step 1: Failing test** - a binding with a uid whose action exists plans/fires; one whose uid is unknown is skipped (not panicked).
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** the field rename + uid matching in `plan_binding`/`apply_diff`.
- [ ] **Step 4: Run, expect PASS** + fmt + clippy.
- [ ] **Step 5: Commit** - `feat(hotkeys): bind hotkeys to plugin uid`

---

### Task 6: Plugin-config files keyed by uid

**Files:**
- Modify: `apps/qol-tray/src/features/profile/scope_store.rs` (config path helpers)

**Interfaces:**
- Produces: plugin-config files named `<uid>.json` under each scope's `plugin-configs/`.

- [ ] **Step 1: Failing test** - the config path helper for a given uid resolves to `<...>/plugin-configs/<uid>.json`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** the helper change.
- [ ] **Step 4: Run, expect PASS** + fmt + clippy.
- [ ] **Step 5: Commit** - `feat(profile): key plugin-config files by uid`

---

### Task 7: Registry keyed by uid

**Files:**
- Modify: `apps/qol-tray/src/plugins/registry/mod.rs`

**Interfaces:**
- Produces: registry entry keyed by uid; lookups by uid; display via index.

- [ ] Steps mirror Task 4 (failing test for uid-keyed lookup → implement → pass → commit).
- [ ] **Commit** - `feat(plugins): key plugin registry by uid`

---

### Task 8: Migration `v3_19_to_v3_20_plugin_uid`

**Files:**
- Create: `libs/qol-migrations/src/v3_19_to_v3_20_plugin_uid/mod.rs` (+ `legacy_table.rs` from Task 2)
- Modify: `libs/qol-migrations/src/lib.rs` (`mod` + `PreFlightRegistry::current()`), `libs/qol-migrations/Cargo.toml` (version bump)
- Create: `libs/qol-migrations/fixtures/v3_19_to_v3_20_plugin_uid/{before,after}/`

**Interfaces:**
- Consumes: `LEGACY_ID_TO_UID` (Task 2), the on-disk shapes from Tasks 4-7.
- Produces: `pub struct V3_19ToV3_20PluginUid` impl `FileMigration` (`name`, `applies`, `migrate`). Mirror `V3_18ToV3_19DeclaredPluginId` structure.

**Algorithm:** build `id→uid` from installed manifests overlaid with `LEGACY_ID_TO_UID`; re-key lock, hotkeys, plugin-config filenames, registry; coalesce entries sharing a uid; leave unmapped ids untouched + log; idempotent when already uid-keyed.

- [ ] **Step 1: Failing test** - `applies()` true on a `before/` fixture containing id-keyed state incl. a preserved `plugin-screen-recorder` lock entry; full `migrate()` round trip yields the `after/` shape where that entry is re-keyed to qol-shot's uid and any qol-shot/screen-recorder pair is coalesced to one.
- [ ] **Step 2: Run, expect FAIL** - `cargo test -p qol-migrations v3_19_to_v3_20`
- [ ] **Step 3: Implement** the migration by copying the `declared_plugin_id` skeleton; use the overlay map; archive-before-write.
- [ ] **Step 4: Register** in `PreFlightRegistry::current()` after `V3_18ToV3_19DeclaredPluginId`; bump `qol-migrations` minor in `Cargo.toml`.
- [ ] **Step 5: Run, expect PASS** + fmt + clippy on `qol-migrations`.
- [ ] **Step 6: Commit** - `feat(migrations): re-key plugin state to uid (v3_19→v3_20)`

---

### Task 9: `qol doctor` uid↔name decode table (one-shot check)

**Files:**
- Modify/Create under `apps/qol-tray/src/doctor/checks/`

**Interfaces:**
- Consumes: `PluginIdentityIndex`.
- Produces: a `DoctorCheck` that reports the `uid → id/name` table for every installed plugin so opaque on-disk files are decodable from one command. (Does NOT prune orphans - that is Project 3.)

- [ ] Failing test → implement → pass → commit `feat(doctor): report uid↔name decode table`

---

### Task 10: uid enforcement gate (OPEN DECISION)

**Open question (confirm with user before implementing):** after the migration window, should a `plugin.toml` lacking `uid` be a **hard reject** at load, or a **soft warn**? Spec leans: soft-warn during the window (transitional uid synthesized), hard-reject after, gated on the post-migration schema version.

- [ ] Once decided: failing test for the chosen behavior → implement gate in `manifest_loader.rs` (extend `validate_manifest_contract`) → pass → commit `feat(plugins): require uid after migration window`

---

### Final delivery

Per `git-trees`: squash the `plugin-uid-identity` worktree into **one** polished conventional commit on the local main clone (`feat(plugins): immutable uid plugin identity + v3_19→v3_20 migration`), unless the user asks for multiple commits. Then `docs/plugin-contract.md` update can ride along or be its own docs commit. Do not push from the worktree.

## Self-Review

- **Spec coverage:** manifest uid (T1), authoring + legacy table (T2), uid-keyed identity/index (T3), lock+coalesce (T4), hotkeys (T5), configs (T6), registry (T7), migration+convergence (T8), doctor decode (T9), enforcement (T10). Cross-PC convergence handled by T8's overlay table + T4 coalesce. ✓
- **Placeholder scan:** uid literal values are authored in T2 (a real action, not a placeholder); template references point at concrete files. ✓
- **Type consistency:** `PluginUid` (T1) used as `plugin_uid`/`uid` fields uniformly in T3-T8; `PluginIdentityIndex` API stable across T3/T5/T9. ✓
