# Plugin UID Identity - Design

Date: 2026-06-23
Status: Draft (awaiting review)
Scope: Project 1 of 3 (see "Relationship to sibling work")

## Problem

Durable, cross-PC plugin state is keyed on the plugin's **mutable** human id
(`plugin.toml` `[plugin] id`). When a plugin is renamed across a release
(observed: `plugin-screen-recorder` -> `qol-shot`), every stored reference goes
stale and there is no stable anchor to repair it from.

Concrete failure reproduced on the user's machine:

- `profile/plugins.lock.json` (the synced "installed plugins" record) still
  pins `plugin-screen-recorder` (v1.3.1, `platforms = ["linux"]`).
- `plugin-screen-recorder` is absent from `plugin-registry.json` (the live id is
  now `qol-shot`) and is installed nowhere locally.
- The active profile's hotkeys do **not** reference it - so this is **not**
  hotkey drift; it is a stale profile record.

Why it persists and spreads (production, multi-PC):

1. The existing `v3_18_to_v3_19_declared_plugin_id` migration only remaps ids it
   can derive from a **locally installed** plugin's manifest. On a machine where
   the plugin is not installed, nothing remaps it.
2. `build_plugins_lock`
   (`apps/qol-tray/src/features/profile/core/plugins_lock.rs`) deliberately
   **preserves** entries whose `platforms` are unsupported on the current OS
   (cross-PC recovery). A linux-only entry therefore lives forever on macOS.
3. Sync circulates the dead id to every PC. Any hotkey bound to it silently
   stops firing (the plan/diff cycle drops unknown bindings but never prunes
   storage).

Root cause: **identity == a renamable string.** Fixing propagation (chasing the
string across files and peers) is brittle. We fix the identity model instead.

## Goals

- A plugin's durable identity is **immutable**. Renaming the human id, folder,
  command, or repo never invalidates stored state and never recurs as a dead id
  across PCs.
- All durable / synced plugin state keys on the immutable identity.
- Existing installs migrate deterministically and identically on every PC,
  including machines where a given plugin is not installed.
- Mixed-version fleets (one PC upgraded, another not) converge without flapping.

## Non-goals

- The live git-config monitor (Project 2) and the doctor live-monitor vector are
  out of scope. Defined separately.
- Governance of uids for third-party / external plugin sources. The model must
  not preclude it, but this project covers **core monorepo plugins only**.
- Changing what plugins *do* or their menu/runtime contracts.

## Identity model: frozen published UID

- `plugin.toml` gains `[plugin] uid = "<opaque token>"`, authored **once** and
  **frozen forever**. It is *published* identity: the same bytes ship to every
  PC, so sync treats the same plugin identically everywhere. It is never
  generated per-install (a per-install uid would split one plugin into N across
  PCs).
- `id`, `name`, the install folder name, `runtime.command`, and `repo_url`
  become **mutable labels**. None of them is identity.
- UID format: an opaque, collision-resistant token (e.g. a uuidv4 or ULID),
  authored as a literal. Opaque on purpose - a human-meaningful slug invites
  "fixing" it later, which would break the freeze invariant.

### On-disk form: UID-keyed everywhere

Durable / synced state keys on uid, with no human label in the key:

| Artifact | Before | After |
|---|---|---|
| `plugins.lock.json` entry | keyed by `id` | keyed by `uid` (field `uid`; `id` retained as a display label only) |
| `hotkeys.json` binding | `plugin_id: String` | `plugin_uid: Uid` (display label derived at render time) |
| `plugin-configs/<id>.json` | filename = id | filename = `<uid>.json` |
| `plugin-registry.json` entry | keyed by `id` | keyed by `uid` |

Consequence: on-disk files are **opaque** when read directly (you cannot tell
which plugin `plugin-configs/<uid>.json` is by name alone). Accepted trade-off
for permanent rename-safety. To offset it:

- A runtime `PluginIdentityIndex` provides `uid -> { id, name }` for every
  loaded plugin, built from manifests at load. All human-facing surfaces (UI,
  logs, doctor output) resolve display names through it.
- `qol doctor` (one-shot check) prints the `uid <-> id/name` table so the opaque
  on-disk state is always decodable from a single command.

## Components and changes

1. **Manifest schema** (`apps/qol-tray/src/plugins/.../manifest`): add required
   `uid`. Loader rejects a manifest without a uid (after migration window;
   during the window, absence is tolerated and backfilled - see Migration).
   Update `docs/plugin-contract.md`.

2. **Identity type**: introduce a `PluginUid` newtype (mirroring the existing
   `PluginId`). Identity comparisons everywhere (resolver, loader, lock,
   hotkeys, configs) switch from `PluginId`/`String` to `PluginUid`.
   `PluginId` survives only as a display label.

3. **Loader / resolver** (`apps/qol-tray/src/plugins/`): scan dirs as today
   (folder name is a label), read each `plugin.toml`'s `uid`, key the loaded set
   and the registry by uid. Build the `PluginIdentityIndex`.

4. **Profile lock** (`features/profile/core/plugins_lock.rs`):
   `PluginLockEntry` keyed by `uid`; `id` kept as a display field.
   `build_plugins_lock` keys/coalesces by uid; the platform-preservation path
   preserves by uid. `sync_plugins_lock_from_plugins` writes uids.

5. **Hotkeys** (`apps/qol-tray/src/hotkeys/`): `HotkeyBinding.plugin_uid`.
   Planning / diff match on uid. Render resolves uid -> name via the index.

6. **Plugin configs** (`features/profile/scope_store.rs`): config files named
   and looked up by uid.

7. **Doctor**: the one-shot reconcile check (separate spec) flags only uids that
   match no plugin's uid anywhere = genuine orphans. Renamed-but-alive plugins
   can no longer be mistaken for orphans.

## Migration: `v3_19_to_v3_20_plugin_uid` (PreFlight, FileMigration)

Re-keys all on-disk plugin state from `id` to `uid`, identically on every PC.

Inputs to the `id -> uid` map, in priority order:

1. **Locally installed manifests**: read `uid` from each installed
   `plugin.toml`. Authoritative for plugins present on this machine.
2. **Built-in core table**: a `const LEGACY_ID_TO_UID: &[(&str, &str)]` shipped
   inside the migration, covering every core monorepo plugin **and its
   historical ids**. This is what lets a machine re-key an entry for a plugin it
   does not have installed (the screen-recorder-on-macOS case). It includes
   `("plugin-screen-recorder", <qol-shot uid>)` and `("qol-shot", <qol-shot uid>)`.

Algorithm (per profile, archive-before-write per crate convention):

1. Build `id -> uid` from (local manifests) overlaid with (built-in table).
2. For lock, hotkeys, plugin-config filenames, registry: replace each `id` key
   with its `uid`. Entries whose id maps to no uid are left untouched (a later
   doctor pass classifies true orphans) and logged.
3. **Coalesce**: when two entries resolve to the same uid (e.g. a preserved
   `plugin-screen-recorder` lock entry and a `qol-shot` entry), merge into one,
   preferring the installed/most-recent metadata. Never merge two distinct uids.
4. Idempotent: an entry already keyed by a known uid is a no-op.

Cross-PC convergence:

- Every PC runs the same migration with the same built-in table, so each
  produces uid-keyed state independently and pushes uids.
- A not-yet-upgraded peer can still push an old id. Because the upgraded PC's
  lock-rebuild and coalesce are uid-aware and the built-in table maps the old id
  to the same uid, the resurrected old-id entry collapses into the uid entry on
  the next rebuild rather than flapping. Convergence does not require all PCs to
  upgrade simultaneously.
- Sliding window: the built-in `LEGACY_ID_TO_UID` table is retained for the
  supported upgrade window, then the migration (and table) is pruned in the same
  commit that introduces the next breaking migration, per
  `qol-tray-data-migrations`.

Register in `PreFlightRegistry::current()` after
`V3_18ToV3_19DeclaredPluginId`. Bump `qol-migrations` minor version.

## One-time authoring task

Author a frozen `uid` into every core `plugins/*/plugin.toml` (~12 plugins;
`plugin-template` gets a placeholder note, not a real uid). The same uids
populate the migration's built-in table. This is a single commit and must land
before the migration ships.

## Testing

- `fixtures/v3_19_to_v3_20_plugin_uid/before|after/` covering: a normal install,
  a renamed plugin present locally, a renamed plugin **absent** locally (the
  screen-recorder case), a dual-entry that must coalesce, and an already-migrated
  (idempotent) profile.
- `applies()` true/false paths; full `migrate()` round trip asserting resulting
  config-dir shape.
- Loader/resolver tests: identity resolves by uid; display name resolves via the
  index; a stored old id no longer matches (it has been re-keyed) and a
  not-installed uid is preserved, not dropped.
- Hotkey planning matches on uid.

## Risks / open questions

- **Opaque on-disk state** hurts manual debugging. Mitigated by the doctor
  uid<->name table and the runtime index. Confirmed acceptable by user.
- **Required-uid enforcement timing**: rejecting uid-less manifests must not fire
  until the migration window has backfilled. Gate enforcement behind the
  post-migration schema version.
- **Third-party plugins** will eventually need uid governance (uniqueness, trust).
  Out of scope here; the newtype + index leave room for it.
- **`repo_url` as a secondary anchor**: not used as identity; renaming a repo is
  also just a label change under this model.

## Relationship to sibling work

This session surfaced three independent projects. They share the doctor
`framework` plumbing but nothing else, and must stay separated:

- **Project 1 (this spec)**: plugin uid identity + `v3_19_to_v3_20` migration.
- **Project 2**: live git-config `.git/config` upstream-strip monitor, a new
  doctor **`monitors`** vector (event-driven, run by `qol dev`'s loop), distinct
  from one-shot **`checks`**. Separate spec.
- **Project 3**: doctor one-shot reconcile **check** for genuine orphan uids.
  Falls out of Project 1; separate spec.
