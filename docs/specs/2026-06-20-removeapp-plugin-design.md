# removeapp - app uninstaller plugin (design)

Date: 2026-06-20
Status: draft - design approved, pending implementation plan

## Summary

`removeapp` is a qol-tray plugin that uninstalls an app *and its leftovers* (the
support/cache/preference files an app scatters across the OS), the way AppCleaner
does. The removal engine is **OS-agnostic** behind a platform strategy; **macOS is
iteration 1**, with Linux/Windows as later iterations. It replaces a lost personal
shell script (`~/uninstall-app.sh`, fronted by `~/.local/bin/removeapp`) that
pointed at a now-deleted target.

It is built as a **headless core with a UI wrapped around it**:

- a **core engine** that discovers apps, resolves a target's leftovers, and
  removes them (Trash by default, hard-delete on opt-in) - pure, testable, no UI;
- a **headless CLI** (`removeapp scan|remove`) over that core - scriptable and
  standalone, which restores the original `removeapp <app>` terminal use;
- a **gpui picker UI** (`removeapp open`) that links the same core in-process.

## Scope

### In scope (v1, macOS as iteration 1)
- Remove an app bundle + its standard `~/Library` leftovers (macOS impl).
- Headless CLI: `scan` (report) and `remove` (act), JSON output.
- gpui keyboard-first picker, lifetime-bound to qol-tray (cli-sessions pattern).
- Move-to-Trash by default (reversible); hard-delete only via explicit opt-in.
- Guardrails refusing system / OS-signed / managed-security apps.
- OS-agnostic engine behind the strategy pattern; macOS is the implemented
  backend, Linux/Windows compile as typed-`Err` until their iteration lands.

### Out of scope (deferred)
- Login-item and `/var/db/receipts` cleanup (later iteration).
- Running-process detection / "quit before removing" (later iteration).
- Functional Linux/Windows backends (later iterations; the engine is ready).
- The `qol new --kind ...` generator. That is **sub-project 2**, its own spec,
  extracted once removeapp gives us a second concrete `window`-kind plugin to
  template alongside cli-sessions.

## Plugin lifecycle: "window" kind

removeapp is the **window** lifecycle (lazy-launched on demand, single-instance,
lives while open, dies with host), the same pattern cli-sessions uses. It never
auto-starts - only `daemon` plugins do. This needs **no qol-tray contract
change** - it is entirely expressible plugin-side:

- To qol-tray it is a plain **runtime plugin**: `[runtime]` with
  `actions = { open = ["open"] }`, `[capabilities] gpui = true`, no `[daemon]`.
- On `open`, qol-tray spawns `removeapp open` fresh and (because the action is a
  runtime-only `open`) does **not** dedupe the spawn.
- The binary self-manages a **singleton**: `main.rs` calls
  `qol_plugin_daemon`'s `send_action("open")` on its own socket; if an instance
  answers, the new process focuses it and exits, otherwise it becomes the
  long-lived one.
- The long-lived process arms `qol_runtime::spawn_host_death_watchdog` (via the
  `QOL_TRAY_STATE_SOCKET` env qol-tray injects), so it exits on socket EOF or
  `getppid() == 1`.

## Architecture

New plugin crate `plugins/removeapp` (package `plugin-removeapp`, binary
`removeapp`, lib `plugin_removeapp`), same layout as `plugin-cli-sessions`. One
crate, three layers; the core never knows about gpui, and the UI and CLI never
re-implement removal logic.

```
core engine (lib)  ──>  AppPlatform strategy  ──>  platform/{macos,linux,windows}
      ▲                                                   macos: real
      │                                                   linux/windows: typed Err
  ┌───┴───────────────┐
  │                   │
headless CLI       gpui UI
(scan / remove)    (open)
```

### Layer 1: core engine (`src/core/`)

Pure domain logic over a platform strategy. No I/O choices baked in beyond what
the strategy exposes.

Domain types:

```rust
pub struct InstalledApp {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,            // the .app bundle
}

pub enum LeftoverKind {
    AppBundle,
    ApplicationSupport,
    Caches,
    Preferences,
    Containers,
    GroupContainers,
    SavedState,
    Logs,
    HttpStorages,
    WebKit,
    LaunchAgent,
}

pub struct Leftover {
    pub path: PathBuf,
    pub kind: LeftoverKind,
    pub size_bytes: u64,
}

pub struct RemovalPlan {
    pub app: InstalledApp,
    pub items: Vec<Leftover>,    // includes the bundle itself
    pub total_bytes: u64,
}

pub struct RemovalOutcome {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}
```

Core API (delegates to the strategy):

```rust
pub enum Disposal { Trash, Delete }   // Trash is the default; Delete is opt-in

pub fn installed_apps() -> Result<Vec<InstalledApp>>;
pub fn search(query: &str) -> Result<Vec<InstalledApp>>;     // ranked (UI picker)
pub fn resolve_unique(query: &str) -> Result<InstalledApp>;  // Err on 0 or >1 (CLI)
pub fn plan(app: &InstalledApp) -> Result<RemovalPlan>;
pub fn remove(plan: &RemovalPlan, how: Disposal) -> Result<RemovalOutcome>;
pub fn is_protected(app: &InstalledApp) -> bool;             // guardrail
```

Two resolution paths, because the consequences differ: the UI shows a ranked
`search` list and a human picks, but the CLI must never *guess* its way into a
destructive `remove`. `resolve_unique` errors on an ambiguous query and prints
the candidates instead of acting. `remove` refuses a protected app (returns a
typed error) before touching the filesystem. Disposal is **Trash by default**;
`Disposal::Delete` (hard delete) is reachable only through an explicit opt-in
(`--force` / a UI toggle). `RemovalOutcome` reports per-path success/failure so
partial failures surface instead of aborting silently.

### Layer 2: platform strategy (`src/core/platform/`)

The qol-arch-code strategy pattern. All OS-specific knowledge - where leftovers
live, what counts as protected, how to remove - lives here. Zero `#[cfg]` in the
core logic.

```rust
pub trait AppPlatform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>>;
    fn scan(&self, app: &InstalledApp) -> Result<RemovalPlan>;
    fn remove_paths(&self, paths: &[PathBuf], how: Disposal) -> Result<RemovalOutcome>;
    fn is_protected(&self, app: &InstalledApp) -> bool;
}
```

- `platform/mod.rs` - trait + cfg-aliased `pub use <os>::Platform`.
- `platform/macos.rs` - real impl. Enumerates `/Applications`, `~/Applications`,
  `/System/Applications` (read-only, for protection checks); reads `bundle_id`
  from `Info.plist`; collects leftovers by bundle-id and name across the
  `~/Library` locations in `LeftoverKind`. `remove_paths` Trashes via NSFileManager
  `trashItemAtURL:` through objc2 (native, no new dep - objc2 is already a
  workspace macOS dep), or hard-deletes via `remove_dir_all` when
  `Disposal::Delete`.
  **Testability:** constructed with overridable roots
  (`MacosPlatform::with_roots(home, applications_dirs)`) so tests point it at a
  tempdir; the default reads real dirs.
- `platform/linux.rs`, `platform/windows.rs` - stub `Platform` returning typed
  `Err("removeapp: not implemented on <os>")` for every method, until each OS's
  iteration replaces the stub with a real impl.

Protection rule (`is_protected`), macOS:
- path under `/System` or `/Library/Apple`, or
- app is Apple-system-signed (codesign team is Apple), or
- `bundle_id` matches the managed-security denylist (Microsoft Defender
  `com.microsoft.wdav*`, Intune `com.microsoft.intune*`, Heimdal, etc.), or
- the bundle is not writable by the current user.

### Layer 3a: headless CLI (`src/main.rs` + `src/cli/`)

```
removeapp scan  <query>                        # resolve, print RemovalPlan as JSON
removeapp remove <query> [--dry-run] [--yes] [--force]
removeapp open                                 # launch the gpui UI (default action)
```

- `scan` - `resolve_unique` the app (ambiguous query → error listing candidates,
  no guess), print the plan (items + per-item size + total) as JSON. No mutation.
- `remove` - `resolve_unique`, plan, refuse if protected, then prompt `y/N`
  (skipped with `--yes`) and move to Trash. `--force` hard-deletes instead of
  Trashing (still subject to guardrails + confirm). `--dry-run` prints the plan
  and exits without touching anything. Output is JSON (`RemovalOutcome`).
- `open` - the activation path. Singleton via `send_action`, else run the UI.

`main.rs` stays thin: parse subcommand, dispatch to `cli::` or `ui::`. It is a
wrapper over already-tested core functions, so it gets no unit tests of its own.

### Layer 3b: gpui UI (`src/ui/`)

Keyboard-first picker, lifetime-bound to qol-tray:

1. Open → list installed apps (`core::installed_apps`) with fuzzy filter
   (`qol-search`) and icons (`qol-app-icon` by bundle-id).
2. Select an app → `core::plan(app)` → render the leftover list + total size.
3. Confirm (Enter) → `core::remove(plan, Disposal::Trash)` → show outcome, close.
   A clearly-marked toggle switches that confirm to `Disposal::Delete` (permanent).
4. Esc cancels. Protected apps render as non-removable with the reason.

The UI calls the **same core** as the CLI - it never shells out to the binary.

## Manifest (`plugin.toml`)

```toml
[plugin]
name = "Remove App"
description = "Uninstall an app and its leftovers"
version = "0.1.0"
author = "KMRH47"
platforms = ["macos"]          # compiles everywhere; only macOS is functional in v1

[runtime]
command = "removeapp"
actions = { open = ["open"] }

[capabilities]
gpui = true

[menu]
label = "Remove App"
items = [
    { type = "action", id = "open", label = "Open", action = "run" },
]

[[shortcuts]]
id = "open"
name = "Remove App"
action = "open"
export_to_launcher = true

[[dependencies.binaries]]
name = "removeapp"
repo = "qol-tools/plugin-removeapp"
pattern = "removeapp-{os}-{arch}"
```

`platforms = ["macos"]` so qol-tray does not offer a non-functional plugin on
Linux/Windows, even though the binary compiles there (typed-`Err` stubs). Each
later OS iteration adds its name here once its backend is real.

## Shared libraries used

- `qol-plugin-daemon` - singleton socket + `send_action` (focus-existing-on-open).
- `qol-runtime` - host-death watchdog (`spawn_host_death_watchdog`).
- `qol-gpui` - gpui window/keepalive helpers (as cli-sessions uses).
- `qol-search` - fuzzy app filtering in the picker.
- `qol-app-icon` - app icons by bundle-id in the picker.

No new direct dependency is added without checking `qol-shared-libs` first.

## qol-tray integration

- **Launcher app (free).** The `[[shortcuts]] export_to_launcher = true` block
  makes qol-tray materialize a real OS application for `open` -
  `~/Applications/QoL/Remove App.app` on macOS (Spotlight / Raycast discover it),
  a `.desktop` entry under `~/.local/share/applications/` on Linux. Launching it
  runs `qol-courier exec shortcut open`, which forwards to the running tray's
  `/api/shortcuts/open/execute` endpoint and routes back to the plugin's `open`
  action. Same path cli-sessions uses; no extra code in removeapp.
- **Persistence.** removeapp persists nothing in v1. There is no general plugin
  data dir - the only blessed per-plugin store is `config.json` via a
  `qol-config.toml` (scoped Core / Os / Device; Core+Os sync through the GitHub
  profile, Device stays local). If a later iteration adds a user-editable
  protect-list or extra scan locations, it lives there, **Os-scoped** (app layout
  is OS-specific). Transient runtime state, if any, lives under `/tmp/qol-tray`
  (wiped on startup), never hand-rolled dotfiles.

## Safety model

- **Trash by default, delete is opt-in.** Default disposal moves items to the
  Trash (recovery = "restore from Trash"). Hard delete (`remove_dir_all`) happens
  only when the user explicitly asks - `--force` on the CLI, a marked toggle in
  the UI.
- **Preview + explicit confirm.** UI shows the full plan before acting; CLI
  `remove` prompts unless `--yes`, and `--dry-run` is act-free.
- **Guardrails (both dispositions).** `is_protected` blocks system, OS-signed, and
  managed-security apps before any filesystem mutation, including under `--force`.
  Relevant on this managed work machine (Intune / Defender / Heimdal present).
- **Partial-failure honesty.** `RemovalOutcome.failed` carries per-path errors;
  nothing is swallowed.

## Testing

- **core leftover matching** - table-driven: given a bundle-id + name and a fake
  `~/Library` tree (tempdir via `MacosPlatform::with_roots`), the expected set of
  `Leftover` paths and `total_bytes` are produced. Cases cover present/absent
  locations and name-vs-bundle-id matches.
- **protection rules** - table-driven: `/System` paths, denylisted bundle-ids,
  and non-writable bundles are refused; ordinary user apps are not.
- **disposal** - against a tempdir root: default Trash moves sources out and lists
  them in `removed` with nothing hard-deleted; `Disposal::Delete` removes them;
  both refuse a protected app (guardrails hold even with delete / `--force`).
- **contract test** - `validate_plugin_contract` parses `plugin.toml` and calls
  `manifest.validate()`.
- No tests for `main.rs` dispatch (thin wrapper over tested core).

Verification gate (qol-arch-code matrix) before "done":
`cargo fmt --check`, `cargo clippy --all-targets --all-features --keep-going -- -D warnings`,
`cargo build`, `cargo test`.

## Open decisions (locked unless flagged)

| Decision | v1 choice |
|---|---|
| Removal mechanism | Trash by default; hard delete via opt-in (`--force` / UI toggle) |
| Platforms (manifest) | `["macos"]` (iteration 1) |
| Platforms (code) | OS-agnostic engine; Linux/Windows typed-`Err` until their iteration |
| Leftover scope | bundle + `~/Library` locations in `LeftoverKind` + writable `/Library/Launch*` |
| Guardrails | refuse system / OS-signed / managed-security (even under `--force`) |
| CLI confirm | prompt unless `--yes`; `--dry-run` is act-free |
| CLI resolution | `resolve_unique`; ambiguous query errors with candidates |
| Trash impl | objc2 NSFileManager `trashItemAtURL:` (delete: `remove_dir_all`) |
| Launcher app | materialized free via `export_to_launcher`; Spotlight-discoverable |

## Follow-on: sub-project 2 (generator)

`qol new <name> --kind command|daemon|window` scaffolds a plugin from
`plugin-template`, deterministically applying today's manual Customize Checklist
and the kind-specific wiring. The three kinds, by when they start and end:

- `command` - starts when invoked, exits when its task finishes.
- `daemon` - starts automatically at qol-tray startup (the only auto-start kind),
  exits when qol-tray exits.
- `window` - starts when invoked (hotkey / menu / launcher, never on boot), exits
  when you close it.

Two layers: (1) mechanical scaffold (copy + rename + metadata, kind-agnostic) and
(2) kind skeletons (command from plugin-template, daemon from alt-tab/keyremap,
window from cli-sessions + removeapp). The window skeleton is extracted *after*
removeapp ships, so it is templated from two real examples, not one. Separate spec.
