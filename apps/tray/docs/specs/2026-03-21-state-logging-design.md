# State Architecture & Production Logging

## Problem

qol-tray shares a single config directory between dev and prod modes. Dev sessions modify state files (dev-links, build fingerprints, log controls) that the installed prod binary reads on next boot, causing:

- Plugin resolution pointing to dev paths instead of installed plugins
- Hotkeys bound to plugins that resolve to wrong binaries
- Self-update failures with no visibility into root cause
- Stale staging directories from failed installs accumulating indefinitely
- Ephemeral runtime state (PIDs, sockets) stored in the config dir with no lifecycle management

There is no production logging. Errors are written to stderr and lost.

## Design

### Phase 1: State & Lifecycle

#### Directory Layout

```
~/.config/qol-tray/                    # PERSISTENT - user owns this
  hotkeys.json                         # user-configured hotkeys
  plugin-configs.json                  # plugin settings
  .github-token                        # GitHub API auth
  shortcuts.json                       # app shortcuts
  task-runner.json                     # task runner config
  suppressed-errors.json               # app-managed rate limiter state (see App State below)
  plugins/                             # installed plugin files (replaceable)
    plugin-launcher/
    plugin-alt-tab/
    ...
  dev/                                 # dev-mode only, prod never reads
    links.json
    build-fingerprints.json
    core-log-controls.json
    plugin-log-controls.json

/tmp/qol-tray/                         # EPHEMERAL - process owns this
  pids/                                # one file per daemon: {plugin-id}.pid
  staging/                             # install/update working dirs
  cache/                               # plugin metadata cache
```

#### State Categories

| Category | Examples | Rule |
|---|---|---|
| User state | hotkeys.json, plugin-configs.json, .github-token, shortcuts.json, task-runner.json | Never delete. User owns. Survives all upgrades. |
| App state | suppressed-errors.json | Machine-generated, UI-managed. Persists across restarts. Not user-editable directly. |
| Plugin files | plugins/*/ (binaries, plugin.toml, ui/) | Replaceable on install/update. Not user-editable. |
| Dev state | dev/links.json, dev/build-fingerprints.json, dev/*-log-controls.json | Only read when `#[cfg(feature = "dev")]`. Prod binary ignores entirely. |
| Ephemeral | /tmp/qol-tray/* (PIDs, staging, cache) | Created and destroyed by the process. OS cleans on reboot as safety net. |

#### Dev-Links Gating

Dev-links are loaded in `plugins/paths.rs::resolve_dev_link_path` and `dev/linking/store.rs::load_dev_links`, both gated behind `#[cfg(feature = "dev")]`. The prod binary has no code path that reads from the `dev/` directory. This is a compile-time gate, not a runtime check.

#### Daemon Socket Paths

Plugin daemons declare their socket path in `plugin.toml` via `daemon.socket`. qol-tray passes this to the daemon via `QOL_TRAY_DAEMON_SOCKET` env var. Socket paths remain plugin-declared and are not managed by qol-tray's directory structure. Plugins are responsible for creating and cleaning up their own sockets. qol-tray only checks reachability.

The main qol-tray state socket (`/tmp/qol-tray-state.sock`, defined in `src/paths.rs`) remains at its current path. It serves as the instance lock: if bind fails, another instance is already running.

#### PID Tracking

Transition from single `.daemon-pids` file to per-plugin PID files under `/tmp/qol-tray/pids/`:

- File naming: `pids/{plugin-id}.pid` (e.g., `pids/plugin-launcher.pid`)
- Content: single PID as text (one PID per file)
- On daemon start: write PID file
- On daemon stop: delete PID file
- `kill_orphan_daemons()` adapts to scan the `pids/` directory, read each file, verify the PID's exe is under a managed plugin path (preserving the `ManagedRoots` safety check), then kill if orphaned
- Startup wipe of `/tmp/qol-tray/` makes this safe: stale PID files from crashes are deleted before orphan scanning

#### Instance Exclusion and Startup Wipe Order

On startup, the sequence is:

1. Existing instance detection (reuse current logic — socket bind + user notification, no duplication)
2. Wipe `/tmp/qol-tray/` directory (safe because step 1 guarantees we are the only instance)
3. Recreate `/tmp/qol-tray/` subdirectories (pids/, staging/, cache/)
4. Continue normal startup (migration, plugin loading, daemon spawn)

This ensures the wipe never destroys a live instance's state.

#### Migration

On first run after upgrade:

1. Create `dev/` subdirectory if it does not exist
2. If `dev-links.json` exists in config root, move to `dev/links.json`
3. Same for `dev-build-fingerprints.json` -> `dev/build-fingerprints.json`, `dev-core-log-controls.json` -> `dev/core-log-controls.json`, `dev-plugin-log-controls.json` -> `dev/plugin-log-controls.json`
4. If `.daemon-pids` exists in config root, delete it (PIDs now live in `/tmp/qol-tray/pids/`)
5. If `.plugin-cache.json` exists in config root, delete it (cache now in `/tmp/qol-tray/cache/`)
6. Delete any `.plugin-*.installing.*` staging dirs in `plugins/` (stale leftovers)

Old file locations are silently ignored after migration. Migration is idempotent.

Phase 1 scope includes updating all internal file-naming constants that reference the old paths: `dev/linking/store.rs` (dev-links path), `dev/build/types.rs` (build fingerprints path), `dev/build/fingerprint_store.rs` (temp file path for atomic writes), `logging/control.rs` (log control paths). Additionally, `LOG_CONTROL_STATE_FILE` and its associated public functions (`load_all_plugin_controls`, `save_all_plugin_controls`, etc.) in `control.rs` are currently not behind `#[cfg(feature = "dev")]` — Phase 1 gates them to match the other dev-only controls.

#### Ephemeral Lifecycle Contract

Every transition that creates ephemeral state owns its cleanup. Zero leaks.

| Transition | Creates | Cleans |
|---|---|---|
| Startup | `/tmp/qol-tray/` dirs (fresh) | Wipe entire `/tmp/qol-tray/` first, after instance lock acquired |
| Daemon start | `pids/{id}.pid` | -- |
| Daemon stop | -- | `pids/{id}.pid` |
| Plugin update | `staging/{id}.{timestamp}/` | Staging dir on success AND failure. Old daemon PID before restart. |
| Plugin uninstall | -- | Daemon PID. Plugin dir. |
| Shutdown | -- | All PID files. Belt-and-suspenders (OS cleans /tmp anyway). |

---

### Phase 2: Production Logging

#### Architecture

```
src/logging/
  mod.rs              # public API: init_logger(), log_error!()
  filter.rs           # existing dev FilterableLogger (unchanged)
  control.rs          # existing dev log controls (unchanged, paths updated for dev/ subfolder)
  rate_limiter.rs     # NEW - deduplication + suppression
  writer.rs           # NEW - file writer with rotation
  platform/
    linux.rs          # log dir via base_data_dir() / "logs"
    macos.rs          # ~/Library/Logs/qol-tray/
    windows.rs        # %LOCALAPPDATA%/qol-tray/logs/
```

Log directory on Linux reuses the existing `base_data_dir()` function from `src/paths.rs` (`~/.local/share/qol-tray/logs/`), avoiding duplicated path logic. `base_data_dir()` is currently private — Phase 2 promotes it to `pub(crate)`.

#### Log File Format

File naming: `qol-tray-YYYY-MM-DD.log`

Daily rotation. On startup, delete log files older than 7 days.

#### Log Entry Structure

```
[2026-03-21 09:15:00] [v2.4.1@a3f7c2e] [core] STARTUP — linux mint 22, x11, 2 monitors, plugins: [plugin-launcher@0.3.0@b2c4d1f, plugin-alt-tab@0.8.1@d4e5f6a]
[2026-03-21 09:15:02] [v2.4.1@a3f7c2e] [plugin:plugin-launcher@0.3.0@b2c4d1f] ERROR plugin.daemon_start_failed — daemon failed to start: connection refused (daemon_lifecycle/spawn.rs:42)
[2026-03-21 09:15:03] [v2.4.1@a3f7c2e] [plugin:plugin-launcher@0.3.0@b2c4d1f] ERROR plugin.daemon_start_failed — daemon failed to start: connection refused (x2) (daemon_lifecycle/spawn.rs:42)
[2026-03-21 09:15:03] [v2.4.1@a3f7c2e] [plugin:plugin-launcher@0.3.0@b2c4d1f] ERROR plugin.daemon_start_failed — daemon failed to start: connection refused (x5, suppressed) (daemon_lifecycle/spawn.rs:42)
```

Fields per entry:
- **Timestamp** — ISO 8601 with seconds
- **App version + commit** — `v2.4.1@a3f7c2e`, baked in at compile time via build.rs
- **Source** — `core`, `plugin:{id}@{version}@{commit}`, `update`, `hotkeys`, `lifecycle`
- **Level** — `STARTUP` (once) or `ERROR` (all other entries)
- **Signature key** — stable identifier for rate limiting (e.g. `plugin.daemon_start_failed`)
- **Message** — human-readable with dynamic context
- **Location** — `file:line` from `file!()` and `line!()` macros

#### Build Info Embedding

Phase 2 creates a new `build.rs` at the crate root (does not exist yet):

```rust
let hash = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output();
println!("cargo:rustc-env=GIT_COMMIT_HASH={}", hash);
```

Available at runtime as `env!("GIT_COMMIT_HASH")`.

Plugin commit hashes are stored in `plugin.toml` by the plugin's CI/build pipeline:

```toml
[build]
commit = "b2c4d1f"
```

Phase 2 deliverables include:
- Creating `build.rs` for qol-tray
- Extending `PluginManifest` schema (`src/plugins/manifest/schema.rs`) with an optional `build` section containing `commit: Option<String>`
- Updating plugin CI pipelines to emit the commit hash into `plugin.toml`
- Graceful fallback: if `[build]` section or commit is absent, log source omits the commit hash (e.g. `plugin:plugin-launcher@0.3.0`)

#### log_error! Macro

```rust
log_error!("plugin.daemon_start_failed",
    source = plugin_source,
    "daemon failed to start: {}", err);
```

- First argument: stable signature key (never changes across code edits)
- `source`: the component context (built from plugin ID + version + commit, or "core")
- Remaining: format string with dynamic values
- Macro automatically appends `file!()` and `line!()`

#### Rate Limiting

Each unique signature key is tracked by the rate limiter.

- Occurrences 1-4: log each with running count
- Occurrence 5: log once with "suppressed", stop writing
- Suppression is permanent until user unsuppresses via Logs UI OR qol-tray version changes

**Version-aware reset:** When qol-tray starts and its version differs from the `version` field stored in a suppressed entry, that entry is automatically unsuppressed. The error may have been fixed in the new version — give it a fresh chance. If it recurs, it will be re-suppressed after 5 occurrences.

Suppressed state persisted to `~/.config/qol-tray/suppressed-errors.json`:

```json
{
  "plugin.daemon_start_failed": {
    "count": 47,
    "first_seen": "2026-03-21T09:15:02",
    "last_seen": "2026-03-21T09:15:03",
    "last_message": "daemon failed to start: connection refused",
    "version": "v2.4.1@a3f7c2e",
    "source": "plugin:plugin-launcher@0.3.0@b2c4d1f",
    "location": "daemon_lifecycle/spawn.rs:42"
  }
}
```

Survives restarts. User manages via Logs UI.

#### Concurrency

Multiple sources emit errors concurrently: main thread, plugin relay threads (one per daemon via `std::thread::spawn`), and async task threads. The rate limiter uses `Mutex<HashMap<SignatureKey, RateState>>` for synchronization. Lock contention is negligible since only errors are logged and the critical section is a hash lookup + counter increment.

#### Startup Context Line

One exception to "errors only": a single STARTUP entry logged on every boot with:
- OS name and version
- Display server (X11/Wayland)
- Monitor count
- All loaded plugins with version and commit hash

This gives full environment context for every error that follows.

#### Plugin Daemon Error Capture

Plugin daemons are separate processes. Their stderr is already relayed via `src/logging/relay.rs`. This relay must be wired into the production log file writer with the same rate limiting. Daemon errors flow through the same log_error! pipeline with `source = plugin:{id}@{version}@{commit}`.

#### What Gets Logged

- Plugin daemon start/stop failures
- Hotkey registration failures
- Self-update failures (download, install, version check)
- Plugin install/update/uninstall failures
- Ephemeral cleanup failures
- State migration errors

#### What Does NOT Get Logged

- Successful operations
- Lifecycle events (started, stopped) beyond the single STARTUP line
- HTTP request logs
- Internal state dumps
- Debug or info level messages

---

### Phase 3: Logs UI

Pending visual prototype. Requirements established:

- New "Logs" view in the dashboard sidebar
- Sub-sidebar with two sections:
  - **Live Log** — real-time SSE stream, newest entry at top
  - **Suppressed** — lists suppressed error signatures with count, first/last seen, version, source. User can unsuppress to re-enable logging for that signature.
- Inherits existing qol-tray styling (theme tokens, component contracts)
- Log entry presentation format TBD (one-line not feasible given field count — prototype needed)

---

## Phases

| Phase | Scope | Dependencies |
|---|---|---|
| 1 - State & Lifecycle | Dir restructure, dev-link gating, migration, ephemeral lifecycle contract, PID tracking overhaul, constant updates in store.rs/types.rs/control.rs | None |
| 2 - Production Logging | File logger, rate limiter, build.rs creation, PluginManifest schema extension, plugin CI updates, daemon error capture, rotation | Phase 1 (needs /tmp/qol-tray/ structure) |
| 3 - Logs UI | Dashboard view, SSE streaming, suppression management | Phase 2 (needs log infrastructure) |

Each phase is independently deployable and testable.
