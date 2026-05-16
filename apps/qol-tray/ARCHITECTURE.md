# qol-tray architecture: ground-truth findings

Code-grounded research for the Claude Design "Runtime Architecture Map". Each increment is pasteable directly into Claude Design as a revision brief. Every claim is anchored to a file:line so the diagram can be audited.

Repo state at time of research:
- branch: `main`
- HEAD: `e8f9674`
- version: `qol-tray 3.15.1`
- repo path: `/Users/kaho/repos/private/qol-tools/qol-tray`

Current correction pass, 2026-05-17:

- Runtime fallback is derived from `daemon.socket`, `runtime.command`, and runtime/daemon path equality. There is no per-action `runtime_fallback` manifest flag in the current schema.
- Per-plugin daemon RPC sends a JSON `qol_runtime::protocol::DaemonRequest { action }` line, not a raw `action_id\n` line.
- Windows has tray, global hotkeys, autostart, and runtime process spawning, but per-plugin daemon socket RPC and the runtime state socket are Unix-only.
- `clean_stale_sockets` runs during plugin loading after registry resolution and manifest load, not from `paths::init_runtime_dirs`.
- Install/update reloads the plugin manager and autostarts daemon-enabled installed plugins immediately; the supervisor monitors and restarts them later.
- `hotkeys::trigger_reload` signals the `global_hotkey` fallback listener when it is running. The Linux kernel evdev capture path currently has no matching reload channel.
- Profile and meta HTTP routes are mounted directly under `/api` (`/api/config/*`, `/api/sync/*`, `/api/version`, `/api/events`, etc.), not under `/api/profile/*` or `/api/meta/*`.

---

## Trace index

The diagram's TRACES array and this doc track these 9 named flows. Use as a navigable map - each Tn is cited line-by-line in the increment listed.

| # | Label | Surface | Plugin path | Documented in |
|---|---|---|---|---|
| T0 | Cold start | main thread + tokio | all | Increment 1.5 |
| T1 | Tray → daemon plugin | OS std::thread | daemon socket (channel 3) | Increment 3 |
| T2 | Tray → ephemeral plugin | OS std::thread | spawn one-shot (no socket) | Increment 3 |
| T3 | Hotkey (kernel evdev OR global_hotkey) | OS callback | same as T1/T2 | Increment 3 |
| T4 | Dashboard / CLI → axum POST | tokio worker | daemon socket (channel 3) | Increment 3 |
| T5 | Install cascade (the multi-region trace) | tokio | manager reload autostarts daemons; EventBus fans to SSE + tray | Increment 5 |
| T6 | Hotkey reload (back-edge from plugins to input) | tokio | global_hotkey fallback reload signal on supervisor transitions | Increment 5 |
| T7 | Query (read-only sibling of T1/T4) | tokio worker | daemon socket, returns body | Increment 3 |
| T8 | Stale socket recovery (3-way protocol) | plugin load + spawn + probe | host cleanup + unlink-on-bind + has_live_listener | Increment 6.8 |

For each Tn the diagram's `narrative` field is the canonical 1-paragraph version; this doc holds the file:line citations and surrounding reasoning.

---

## Increment 1: identity, binaries, IPC channels, and the truth about the event bus

### 1.1 Three binaries, not one

The diagram only shows the daemon. The crate ships three: `Cargo.toml:9-21`.

| Binary | Entry | Role |
|---|---|---|
| `qol-tray` | `src/main.rs` | The daemon (tray + plugins + UI server) |
| `qol-tray-install` | `src/installer/main.rs` | Standalone installer |
| `qol-tray-doctor` | `src/doctor/main.rs` | Standalone doctor (auto-fix + diagnostics) |

The daemon also embeds installer + doctor logic and calls them on startup (`installer::bootstrap_current_install()` at `main.rs:31`, `doctor::auto_fix_startup()` at `main.rs:51`). So `qol-tray-install` and `qol-tray-doctor` are external escape hatches; the daemon runs the same code internally on every boot.

### 1.2 Module tree is 23 entries, not 6

Top-level src modules from `src/lib.rs:1-30`:

```
credentials  daemon  desktop_state  dev*  doctor  features  file_io  hotkeys
housekeeping installer logging  menu  mode  paths  plugins  process_utils  profile
runtime*     shortcuts  signal*  sync  test_support  tray  updates  version
```
(`*` = Unix-only or feature-gated)

Diagram regions (6) → real modules (23) mapping:

| Diagram region | Real modules | Missing from diagram |
|---|---|---|
| 01 User input | `tray`, `hotkeys`, `shortcuts`, axum `/api/...` | `menu` (router + builder), CLI `exec` subcommand |
| 02 Platform integration | `tray/platform/{linux,macos,windows}.rs` | `desktop_state`, `runtime` (Unix-only state server), `signal` |
| 03 Daemon core | `daemon`, `paths`, `housekeeping`, `doctor`, `updates` | `mode`, `installer`, `credentials`, `version`, `file_io`, `process_utils`, `sync` |
| 04 Plugin system | `plugins/*` (14 submodules) | `action_transport`, `capabilities`, `config`, `daemon_tracker`, `execution_contract` |
| 05 Plugin processes | per-plugin daemons | nothing major |
| 06 Persistence | `~/.config/qol-tray/`, `/tmp/qol-tray/` | `sync` (cloud), `logs/` |

### 1.3 There are THREE communication channels, not one

The diagram only labels the per-plugin Unix socket. Two others exist.

| # | Channel | Endpoint | Who uses it | Code |
|---|---|---|---|---|
| 1 | axum HTTP | `127.0.0.1:42700` | browser dashboard, CLI `qol-tray exec`, plugin store API | `features/plugin_store/server.rs:51`, `main.rs:186-242` |
| 2 | desktop-state Unix socket | `/tmp/qol-tray-state.sock` (Unix only) | plugin daemons and external tools reading monitor, cursor, and focus state | `runtime/server.rs:1-47`, `paths.rs:12` |
| 3 | per-plugin Unix socket | path from each `plugin.toml`'s `daemon.socket` | tray + axum dispatching actions to plugin daemons | `plugins/action_executor.rs:138-158`, `plugins/action_transport/mod.rs:16-21` |

Channel 1 is what dashboards talk to. Channel 2 is a one-way state feed for external consumers (per-plugin daemons can read it; not used for action dispatch). Channel 3 is the actual plugin RPC.

The CLI `qol-tray exec <plugin> <action>` opens a raw TCP socket and writes a hand-rolled HTTP POST to channel 1 (`main.rs:185-236`). It does NOT speak the per-plugin Unix socket directly - it goes through axum, which then dispatches to the plugin via channel 3.

### 1.4 EventBus is for state-change broadcast - NOT a tray-click bus

The diagram's T1 trace shows `Tray icon → Linux → Event bus → Tokio runtime → Action executor → plugin·A`. The Event bus is on that path. **This is wrong.**

`EventBus` (`daemon/events.rs:9-46`) is a `tokio::sync::broadcast::Sender<DaemonEvent>` with capacity 64. The variants it carries (`daemon/mod.rs:28-94`):

- `PluginsChanged { revision }`
- `PluginManifestInvalid { plugin_id, path, reason }`
- `PluginResolvedFromFallback { ... }`
- `PluginUnavailable { ... }`
- `UpdateProgress { percent }`, `UpdateComplete`, `UpdateFailed { message }`
- dev-only: `DiscoveryStarted`, `DiscoveryComplete`, `BuildStarted`, `BuildPluginProgress`, `BuildComplete`, `PluginCpuSnapshot`, `SelfRecompileProgress/Complete/Failed`

These are all **outbound state-change notifications** from the daemon to subscribers (the tray menu, the dashboard SSE stream). Tray clicks do not publish to the bus.

The real tray click flow is on the **request side**: OS-native menu callback → `tray-icon::MenuEvent::receiver()` (a `std::thread`, not tokio - `tray/platform/mod.rs:175-194`) → `menu::router::EventRouter::route(event_id)` → for plugin actions, `plugins::action_executor::execute_action()` → either spawn a one-shot subprocess OR send a request to the plugin's own Unix socket via `action_transport::dispatch_daemon_action()`.

Event bus appears on the **response side**: tray subscribes so it can rebuild the menu when `PluginsChanged` fires.

So a single click can hit the bus twice (subscribe-rebuild on plugin reload), zero times (plain plugin action), or once (the action causes a state change downstream). It is never *on* the dispatch path.

### 1.5 Boot is 10+ steps, not 4 bricks

The diagram shows Daemon core as `Bootstrap | Runtime dirs | Doctor | Update check`. The real boot from `main.rs:17-69` and `main.rs:265-352`:

```
PRE-TOKIO (synchronous, main thread)
 1. try_handle_cli_flag()                            # --version, --help, --write-mode
 2. try_exec_subcommand()                            # qol-tray exec → HTTP to running daemon
 3. logging::init_logger()
 4. installer::bootstrap_current_install()           # register this install
 5. is_already_running()                             # TCP probe on :42700
 6. paths::init_runtime_dirs()                       # wipe + recreate /tmp/qol-tray/{pids,cache}
 7. housekeeping::run_startup_cleanup(config_dir)    # migrations
 8. doctor::auto_fix_startup()                       # self-heal

TOKIO MULTI-THREAD (block_on async init)
 9. check_for_updates() (2s timeout)
10. runtime::RuntimeServer::start()                  # Unix only: state socket
11. PluginLoader::ensure_plugin_dir()
12. SyncService::new() + spawn pull_on_launch        # cloud profile sync
13. PluginManager::new() + load_plugins()            # scan + manifest + resolve
14. Daemon::new()                                    # creates EventBus
15. FeatureRegistry::new() + register plugin_store
       (+ register mode_toggle in dev)
16. plugin_store::Plugins::start_server()            # spawn axum on :42700
17. hotkeys::start_capture()                         # try kernel evdev, fallback global_hotkey
18. daemon_supervisor::spawn_supervisor()            # per-plugin daemon supervisor
19. spawn_blocking(launcher_apps::trigger_full_sync)

MAIN THREAD (after init returns)
20. TrayManager::new()                               # create native tray + menu
21. tray::platform::run_app event loop               # native OS event loop
```

Doctor (8) and Update check (9) are not peers - update check runs after tokio is up, doctor runs before. Bootstrap (4) is actually `installer::bootstrap_current_install` and happens before logging is even fully wired.

### 1.6 Persistence paths in the diagram are wrong

From `paths.rs:131-212`, the real paths:

| Diagram claim | Actual path |
|---|---|
| `~/.config/qol-tray/profile/` | ✓ correct (Linux); macOS uses `~/Library/Application Support/qol-tray/`, Windows uses `%APPDATA%/qol-tray/` |
| `profile/mode.json` | ✗ actually `~/.config/qol-tray/mode.json` (at config root, NOT under profile) |
| `profile/plugins/registry.json` | ✗ actually `~/.config/qol-tray/profile/plugins.lock.json` (no `plugins/` subdir, different filename) |
| (missing) | `profile/manifest.json`, `profile/core/{hotkeys,shortcuts,task-runner}.json`, `profile/plugin-configs/<id>.json` |
| (missing) | `~/.config/qol-tray/sync/state.json` + `sync/backups/` |
| (missing) | `/tmp/qol-tray/{pids,cache}/` runtime dir (wiped on every startup) |
| (missing) | `~/.config/qol-tray/.github-token`, `.github-auth.json` |
| (missing) | `~/.config/qol-tray/suppressed-errors.json` |

The `/tmp/qol-tray/` runtime dir is architecturally important: it's the "ephemeral" half of persistence, wiped fresh on every boot. PIDs of supervised plugin daemons live there.

### 1.7 Features registry is a trait, not a slot

`FeatureRegistry` is a `Vec<Box<dyn MenuProvider>>` where `MenuProvider` requires `menu_items()` + `handle_event(event_id)` (`features/mod.rs:12-35`). At boot, only two providers register:

- `plugin_store::Plugins` (always)
- `mode_toggle::ModeToggle` (dev only)

But the `features/` directory has 6 sub-modules: `github_auth`, `launcher_apps`, `mode_toggle`, `plugin_store`, `profile`, `task_runner`. The other four are called directly from various paths (not via MenuProvider), e.g. `launcher_apps::trigger_full_sync` is invoked as a `spawn_blocking` task at boot (`main.rs:343`).

So "features" is two distinct things conflated under one directory: (a) `MenuProvider` implementations that the tray menu queries, (b) standalone subsystems like sync and task runner.

---

## What this means for the diagram

The most impactful single corrections, in priority:

1. **Remove Event bus from the T1 Tray click trace path.** It is not on dispatch. Trace should be: `Tray icon → tray-icon MenuEvent (std::thread) → EventRouter → action_executor → per-plugin Unix socket → plugin·A`. Tokio runtime is also not directly on this path; the menu-event handler thread is OS-native.

2. **Add a second IPC channel to the canvas**: `/tmp/qol-tray-state.sock` (the `runtime::RuntimeServer`). It's Unix-only, used by plugin daemons and external consumers for monitor, cursor, and focus state, and currently invisible.

3. **Fix Persistence paths**: `mode.json` is at config root not under profile; the plugin registry file is `plugins.lock.json` not `plugins/registry.json`; add `/tmp/qol-tray/{pids,cache}/` as the ephemeral half.

4. **Split the "Daemon core" boot bricks into pre-tokio vs tokio-spawn phases.** Bootstrap → Doctor → Init dirs happen synchronously before tokio exists; UpdateCheck → SyncService → PluginManager → axum → hotkeys → Supervisor happen inside the tokio runtime. The diagram currently treats them as peers.

5. **Add the three other binaries' relationship**: `qol-tray-install` and `qol-tray-doctor` are standalone CLIs whose logic is also invoked from `qol-tray` on boot. A footnote box or a sidebar showing "same code, three entry points" would close the gap.

---

---

## Increment 2: Plugin system depth + cross-cutting truths

Region 04 ("Plugin system") is the largest gap. Diagram shows 8 bricks; code has 14+ modules and several material concepts the diagram misses.

### 2.1 Two different registry files (the diagram conflates them)

| File | Path | Owner | Purpose |
|---|---|---|---|
| Plugin registry | `~/.config/qol-tray/plugin-registry.json` | `plugins::registry` (`registry/mod.rs:8-55`) | Resolver source: which binary to execute for each plugin_id, with active + fallback slots |
| Profile plugin lock | `~/.config/qol-tray/profile/plugins.lock.json` | `features::profile::core::plugins_lock` (`features/profile/core/storage.rs:35-48`) | Profile inventory for cloud sync: which plugins are installed in this profile |

These solve **different problems**. Registry = "what binary do I run for plugin X right now?" (with dev-link / worktree overrides). Lock = "what plugins does this profile claim to have, for sync portability?" The diagram shows one brick "Plugin lock" which is the lock file but doesn't expose the registry as a separate entity. Yet the resolver (`plugins/resolver.rs`) reads from the registry, not the lock.

### 2.2 SlotSource has 3 variants, not 2

`registry/mod.rs:42-51`:

```rust
pub enum SlotSource {
    ReleaseAsset,
    DevLink { origin_path: PathBuf },
    WorktreeLink { origin_path: PathBuf, branch: String },
}
```

Diagram says `installed · dev-linked` for Registry. The persisted slot enum has three variants, but the loaded `PluginSource` currently collapses both `DevLink` and `WorktreeLink` to `DevLinked`. Current dev-link creation writes `DevLink`; the active worktree branch is normally stored separately in `dev/active-worktree.txt` and can rewrite active dev paths in memory during plugin load.

### 2.3 Plugin manifest has TWO command fields, not one

A plugin's `plugin.toml` can declare:

- `runtime.command` (`manifest::RuntimeConfig`) - the executable for one-shot actions (ephemeral spawn-and-exit)
- `daemon.command` + `daemon.enabled` + `daemon.socket` (`manifest::DaemonConfig`) - the executable for long-running daemons

A plugin can have either, both, or neither (`plugins/execution_contract.rs:60-81`). The diagram's "daemon vs ephemeral" distinction on plugin process boxes maps directly to whether `daemon.enabled = true`.

Schema exports (`plugins/manifest/mod.rs:9-13`): `ActionType`, `BinaryDependency`, `BuildInfo`, `Capabilities`, `DaemonConfig`, `Dependencies`, `MenuConfig`, `MenuItem`, `PluginInfo`, `PluginManifest`, `RuntimeConfig`. The diagram's "plugin.toml · semver" label is true but understates this - a plugin.toml is a 10-section schema.

### 2.4 Execution contract enforces binary-inside-dir (security)

`plugins/execution_contract.rs:172-180` canonicalises the binary path and refuses it if the resolved path doesn't start with the canonicalised plugin_dir. This blocks symlink escapes. The diagram has no representation of this safety boundary. A note in Region 04 ("Manifest validation") would close it - e.g. `plugin.toml + canonicalized binary in plugin_dir`.

Dev mode also accepts `<plugin_dir>/target/debug/<command>` and `<plugin_dir>/target/release/<command>` (`execution_contract.rs`). Live sources prefer those dev candidates before the primary path; installed sources prefer the primary path first. Windows mode also tries `<plugin_dir>/<command>.exe`. The diagram does not show either fallback, but they're real OS behaviour.

### 2.5 Wire format is shared via `qol_runtime` crate

`plugins/action_transport/protocol.rs:2` imports `qol_runtime::protocol::DaemonResponse`. So plugins and host BOTH depend on the `qol-runtime` external crate for protocol schema. This is the actual "contract" between host and plugin - not informal, not WIT, just a shared Rust crate.

`DaemonResponse` variants from the test cases (`protocol.rs:30-50`):

```json
{"status":"handled"}                                          # action consumed
{"status":"handled","data":{"devices":[{"ieee":"0x123"}]}}    # action consumed + JSON payload
{"status":"fallback"}                                          # daemon refused, host should fall back
{"status":"error","message":"daemon busy"}                     # daemon errored
```

The protocol also has a plain-word fallback ("handled", "fallback", "error <msg>") for human-readable test fixtures and minimal plugin daemons that don't pull in serde.

So the diagram label "Unix socket · ndjson" is correct, but undersells: the format is structured JSON over ndjson, schema-shared via a sibling crate. A footnote `protocol: qol_runtime::DaemonResponse` would ground it.

### 2.6 Daemon supervisor: 5s tick, 5-strike retry, restart on dead

`plugins/daemon_supervisor.rs:7-82`:

```
SUPERVISION_INTERVAL: 5 seconds
MAX_CONSECUTIVE_FAILURES: 5
```

Each tick: snapshot every plugin's daemon PID. Classify each transition into one of `Alive`, `DeadToAlive`, `AliveToDead`, `DeadStaysDead`. For dead-with-retries-remaining, call `restart_daemon`. On any state change, **trigger `hotkeys::trigger_reload()`** (`daemon_supervisor.rs:80`) - this signals the global_hotkey fallback listener when that backend is active.

This last fact is non-obvious and worth its own arrow in the diagram, but with the backend caveat: the fallback listener rebuilds from `hotkeys.json` plus the catalog of currently loaded plugin actions. The Linux kernel evdev capture path currently has no equivalent reload channel.

### 2.7 PID tracking lives at `/tmp/qol-tray/pids/<plugin_id>.pid`

`plugins/daemon_tracker/mod.rs:40-65` writes a `<plugin_id>.pid` file under the runtime PID dir whenever a daemon spawns. On startup, `kill_orphan_daemons()` scans for processes whose PIDs aren't tracked (`daemon_tracker/mod.rs:27-30`). This is how the daemon recovers from a crash-restart without leaking child processes.

The diagram's Persistence "Runtime dir" brick covers the file location but doesn't show that daemon_tracker reads/writes here. An edge from Region 04 down into Persistence ("daemon_tracker writes PIDs") + an edge from Region 03 boot up to Persistence ("doctor scans PIDs") would close the gap.

### 2.8 Plugin module map - actual 14, diagram has 8

| Diagram brick | Code module | Diagram coverage |
|---|---|---|
| GitHub discovery | `features/plugin_store/github/` | partial |
| Loader | `plugins/loader/{scan,manifest_loader}.rs` | ✓ (correct file paths; "fs walk · binary discovery" wording is wrong) |
| Manifest validation | `plugins/manifest/{schema,validation}.rs` | partial - misses Capabilities, Dependencies, RuntimeConfig, DaemonConfig schema |
| Registry | `plugins/registry/mod.rs` | partial - misses WorktreeLink variant |
| Resolver | `plugins/resolver.rs` | ✓ (active/fallback slot is correct) |
| Action executor | `plugins/action_executor.rs` + `action_executor/` dir | ✓ |
| Supervisor | `plugins/daemon_supervisor.rs` | partial - "restart · zombie reap" is approx; reaping is in `daemon_tracker` not here |
| Daemon lifecycle | `plugins/daemon_lifecycle/{spawn,readiness}.rs` | partial - "spawn · health · recompile" - "health" is `wait_for_daemon_ready`, "recompile" is dev-mode self-rebuild not part of this module |
| (missing) | `plugins/action_transport/` | the protocol/wire layer |
| (missing) | `plugins/capabilities.rs` | per-plugin permission framework (serial only so far, Linux-only) |
| (missing) | `plugins/config/` | per-plugin user config persisted at `profile/plugin-configs/<id>.json` |
| (missing) | `plugins/daemon_tracker/` | PID files + orphan kill |
| (missing) | `plugins/execution_contract.rs` | binary-in-dir security validator |
| (missing) | `plugins/manager/{loading,runtime,autostart}.rs` | PluginManager - the in-memory state holder, the actual `HashMap<PluginId, Plugin>` |

### 2.9 features/profile is a major subsystem the diagram doesn't expose

Tree (`features/profile/`):

```
core/         {bundle, import, plugins_lock, storage, types, tests}
http/         {import_export, sync, mod}     # HTTP routes mounted on the axum server
sync/         {service, resolve, types}      # cloud sync engine
startup.rs
mod.rs
```

This handles: profile import/export, cloud sync (the `pull_on_launch` task at boot), conflict resolution between local and remote profiles. The diagram has no visible representation - "Profile" appears once in Persistence as a flat directory, but the **logic** that manages profile state lives here as a feature.

A minimal honest representation: add a "Profile sync" brick to the Tokio band of Daemon core, with `src/features/profile/` reference, showing `cloud pull · import · export · resolve`.

### 2.10 Hotkeys is 9 files, not one

`hotkeys/` modules: `capture`, `catalog`, `listener`, `manager`, `parser`, `planning`, `registration_status`, `store`, `types`. The diagram shows hotkey as one brick in User input.

Two distinct backends:

- **Kernel evdev/uinput** (`hotkeys/capture/`, Linux + feature `linux_evdev`) - preferred, captures hotkeys at the kernel layer. Works even when other apps grab keys.
- **global-hotkey crate** (`hotkeys/listener.rs`) - fallback. Used on macOS/Windows always; on Linux when the kernel backend fails or isn't compiled in.

`hotkeys::start_capture` (`hotkeys/mod.rs:19-45`) is the entry, with `start_hotkey_listener` (`listener.rs`) as the fallback path. Hotkey fires → invokes `action_executor::execute_action(plugin_manager, plugin_id, action)` - same entry point as the menu router uses. **Hotkey is an alternate input edge into the same dispatch node.**

This means a real diagram should show hotkey and tray-icon menu both pointing at action_executor as a converging input fanout, not two separate vertical paths.

---

## What this means for the diagram (Increment 2 cuts)

Top-priority changes for the next pass:

1. **Split Registry from Plugin lock**: they're separate files with separate purposes. Registry (active+fallback slots) belongs in Region 04 next to Resolver as the data store it consumes. Lock belongs in Region 06 (profile state).

2. **Add three missing Region 04 bricks**: `action_transport` (the wire), `capabilities` (per-plugin permissions), `daemon_tracker` (PIDs + orphan kill). The current 8 bricks miss them all.

3. **Show hotkey and tray menu converge on action_executor**: a fan-in arrow, not two parallel rails. Same for CLI exec (which fans through axum back to action_executor).

4. **Add `qol_runtime` external crate as a shared-protocol footnote**: both host and plugin daemons depend on it for `DaemonResponse` schema. Currently the IPC seam looks magical; in fact it's a shared Rust crate.

5. **Add the runtime/daemon command split inside plugin process boxes**: each plugin can have `runtime.command` (ephemeral) AND/OR `daemon.command` (persistent). Currently shown as binary choice but plugins can mix.

6. **Annotate supervisor edge → hotkeys**: supervisor's `trigger_reload()` mutates hotkey state on every daemon transition. Currently invisible.

7. **Surface features/profile as a band in Tokio runtime**: the Sync service brick exists but profile/core + profile/http + profile/sync is an entire 13-file subsystem. Group it as `features::profile { core · http · sync }`.

---

---

## Increment 3: T1-T5 concrete traces (every step with file:line)

Five trace lenses. Each is a real call chain that begins with a user-visible event and ends at the leaf. Steps marked `*` indicate the path crosses a thread or process boundary.

### T1 - Tray click → daemon plugin (the headline trace)

```
[OS thread]
 1. tray-icon crate: native menu click → MenuEvent::receiver() channel  (tray-icon crate)
 2. tray::platform::spawn_menu_event_handler                            (tray/platform/mod.rs:184-194)
 3. handle_menu_event(router, event_id, shutdown_tx)                    (tray/platform/mod.rs:197-213)
 4. menu::router::EventRouter::route(event_id)                          (menu/router.rs:42-52)
 5. plugins::action_executor::execute_action(pm, plugin_id, action_id)  (plugins/action_executor.rs:81-101)
 6. try_execute_action → resolve_plugin_action                          (action_executor.rs:103-110, 170-182)
 7. plugin_manager.lock() acquires Mutex<PluginManager>                 (action_executor.rs:175-181)
 8. resolution::resolve_action(plugin, action_id)                       (action_executor/resolution.rs:15-34)
       └─ builds ResolvedAction{daemon_socket=Some(...), command_path=Some(...), runtime_fallback_allowed}
 9. execution::execute_resolved_action                                  (action_executor/execution.rs:10-18)
       └─ daemon_socket.is_some() → execute_via_daemon
*10. action_transport::dispatch_daemon_action(socket_path, action_id)   (plugins/action_transport/mod.rs:16-21)
*11. platform::dispatch_action(endpoint, action_id)                     (plugins/action_transport/platform/...)
       └─ connect UnixStream → write JSON DaemonRequest { action } + "\n" → read one line
*12. protocol::parse_response(line)                                     (action_transport/protocol.rs:4-22)
       └─ deserialize qol_runtime::protocol::DaemonResponse
13. result: DaemonActionDispatch::Handled { payload }                  (action_executor/execution.rs:26)
```

Steps 1-4 are on the OS-native event thread (NOT tokio). Step 10 crosses a process boundary to the plugin daemon. Steps 11-12 are still inside `qol-tray`, parsing the plugin's response.

If the daemon returns `Fallback` or `Unavailable` AND `runtime_fallback_allowed` is true (`resolution.rs`), step 9 switches to T2's path. A daemon `Error` is rejected rather than silently falling back.

### T2 - Tray click → ephemeral plugin (one-shot spawn)

Steps 1-8 identical to T1. Branch at step 9:

```
 9.  execute_resolved_action: daemon_socket.is_none() OR daemon refused → execute_via_runtime  (execution.rs:70-87)
10.  tracking::reserve_runtime_spawn(plugin_id, action_id)                                     (execution.rs:72)
       └─ in-memory dedup; if already spawning the same (plugin, action), bail
11.  std::process::Command::new(command_path)                                                  (execution.rs:111-122)
       .args(resolved.args)
       .current_dir(plugin_dir)              # cwd = plugin install dir
       .stdout(Stdio::null())                # silent stdout
       .stderr(Stdio::inherit())             # errors visible in qol-tray's stderr
       .env("QOL_TRAY_DAEMON_SOCKET", ...)   # if daemon socket exists, pass it through
       .spawn()
12.  track_action_process(plugin_id, action_id, child.pid)                                     (execution.rs:83)
*13. std::thread::spawn(|| child.wait(); untrack_action_process(...))                          (execution.rs:124-131)
```

Each ephemeral action gets its own OS-native `std::thread` that waits for the child to exit, then untracks it. So spawning N actions in parallel = N reaper threads. Not goroutines, real OS threads.

Security check at step 11: `resolution::validate_runtime_command_path` (`resolution.rs:142-153`) rejects absolute paths and `..` components. Combined with the canonicalised-binary-in-dir check from `execution_contract.rs` at load time, this is two-layer defense against plugin command-injection.

### T3 - Hotkey → action (fans into the same node as T1/T2)

```
[Linux + feature linux_evdev: kernel thread]
 1. /dev/input/event* device read                                       (hotkeys/capture/platform/linux/)
 2. BindingMatcher matches keycode sequence against parse_combo result  (capture/platform/linux/matcher.rs)
 3. /dev/uinput re-emits to userspace                                   (capture/platform/linux/)

[OR: macOS/Windows/Linux-fallback: global_hotkey crate thread]
 1'. global_hotkey crate fires registered callback                       (global-hotkey crate)

[then convergence path - any backend]
 4. capture::install's boxed callback runs                              (hotkeys/mod.rs:39-44)
 5. action_executor::execute_action(pm, plugin_id, action_id)           (hotkeys/mod.rs:42, same as T1/T2 step 5)
 6+ ... same as T1 step 6 onward
```

So T3 enters the dispatch graph at the same node as T1 step 5. The fan-in tag `from: hotkey listener` on Action executor is literally this convergence point.

Hotkey table is dynamic on the `global_hotkey` fallback path: every supervisor transition calls `hotkeys::trigger_reload()`, and the listener rebuilds from `hotkeys.json` plus currently loaded plugin actions. The Linux kernel evdev capture path currently has no reload receiver.

### T4 - Dashboard action → axum POST → action_executor (fans in via axum)

```
[browser]
 1. fetch('http://127.0.0.1:42700/api/plugins/<id>/actions/<action>', { method: 'POST' })

[axum tokio worker thread]
 2. axum router matches POST /plugins/{id}/actions/{action}             (features/plugin_store/server/plugin_handlers.rs:28-31)
 3. plugin_handlers::execute_plugin_action                              (plugin_handlers.rs:103-122)
 4. validate_plugin_id(&id)                                             (plugin_handlers.rs:107)
 5. plugins::action_executor::try_execute_action(pm, id, action)        (plugin_handlers.rs:111)
 6+ ... same as T1 step 6 onward

[after dispatch returns]
n. action_error_response or { success: true, message: "Action dispatched" }   (plugin_handlers.rs:113-120)
n+1. axum serializes Json<ExecuteActionResult>, returns 200 to browser
```

Note: the call is **synchronous** even though it's on tokio - `try_execute_action` is not `async`. The plugin's response is awaited via blocking I/O on the Unix socket inside the tokio worker thread. This is a small but real point: an axum worker can block on plugin daemon I/O.

There's also a parallel **query** path (`plugin_handlers.rs:38-55`): `GET /plugins/{id}/queries/{query}` invokes `action_executor::dispatch_query`, which goes through `action_transport::dispatch_daemon_action` (`action_executor.rs:112-136`) but returns the JSON payload as the HTTP response body. Queries are read-only; actions are fire-and-forget-with-ack. The diagram does not show queries.

### T5 - Plugin install (the multi-stage one with EventBus on the response side)

```
[browser]
 1. fetch('http://127.0.0.1:42700/api/install/<id>', { method: 'POST' })

[axum tokio worker]
 2. axum router → plugin_handlers::install_plugin                       (plugin_handlers.rs:72-79)
 3. plugin_services::install_plugin(state, id)                          (features/plugin_store/server/plugin_services/)
 4. installer::operation_lock acquires per-plugin lock                  (plugin_store/installer/operation_lock.rs)
 5. installer::source resolves install source (GitHub release asset)    (plugin_store/installer/source.rs)
 6. reqwest downloads tarball + checksum                                (plugin_store/installer/operations.rs)
 7. installer::staging untars into temp staging dir                     (plugin_store/installer/staging.rs)
 8. manifest validation on staged plugin.toml                           (plugins/manifest/validation.rs)
 9. execution_contract validates binary-in-dir for staged plugin        (plugins/execution_contract.rs:46-82)
10. atomic move staging dir → ~/.config/qol-tray/plugins/<id>/
11. registry::save_registry adds active slot + new revision             (plugins/registry/mod.rs ~210)
12. profile/core/storage updates profile/plugins.lock.json               (features/profile/core/storage.rs:35-48)
13. plugin_manager.reload_plugins() rescans + replaces in-memory map    (plugins/manager/mod.rs:27-30)
14. EventBus.send_plugins_changed() increments revision, broadcasts     (daemon/events.rs:31-35)
                                            
[broadcast fan-out - all happen concurrently]
15a. SSE handler at GET /api/events forwards DaemonEvent to browser     (features/plugin_store/server/plugin_handlers.rs)
15b. tray menu rebuild subscribers receive PluginsChanged → rebuild     (tray/platform/{linux,macos,windows}.rs)
15c. PluginManager.reload_plugins autostarts daemon-enabled installed plugins immediately →
       daemon_lifecycle::spawn_daemon → wait_for_daemon_ready →
       register PID at /tmp/qol-tray/pids/<id>.pid → hotkeys::trigger_reload()
```

This is the trace where Event bus actually appears - and it appears on the OUTBOUND side after the manager reload. PluginsChanged is the lone event from the install path. T5 also writes to **three** persistence layers (registry file, profile lock, /tmp pids dir) and triggers two UI rebuilds (browser SSE + tray menu); the supervisor later monitors and restarts any daemon that dies.

---

## What this means for the diagram (Increment 3 cuts)

Top-priority changes for the next pass:

1. **T1 trace path should include action_transport as a step**: The v4 diagram now puts Action transport in the breadcrumbs, which is correct. The trace text could also name the wire payload precisely: `qol_runtime::protocol::DaemonRequest { action_id }` outbound, `qol_runtime::protocol::DaemonResponse { status, data? }` inbound. Currently the wire label says DaemonResponse only; both types exist.

2. **T2 should be its own selectable trace**: "Tray click → ephemeral" is structurally different from T1 - no Action transport, no Unix socket, an OS subprocess spawn + a `std::thread` reaper. The diagram has one tray-click trace; it should have two (or one trace with a branch annotation).

3. **T3 should show as a fan-in arrow from Hotkey to Action executor**: The diagram already has `from: hotkey listener` as a tag on Action executor. A T3 trace should highlight: Global hotkey → (kernel evdev OR global_hotkey crate) → action_executor. Same convergence, different entry surface.

4. **T4 should highlight that axum worker BLOCKS on Unix socket I/O**: The trace text could note: "axum worker thread enters action_executor synchronously; Unix socket I/O blocks the worker". This explains why a slow plugin daemon can starve axum.

5. **T5 needs its own region/lens**: install is multi-stage (download, stage, validate, atomic move, registry update, profile lock update, broadcast). None of the diagram's traces show install. T5 also justifies the Event bus brick: it appears on the response side here. Could be the trace that lets "Event bus" come alive in the diagram.

6. **Add a query trace**: `GET /plugins/{id}/queries/{query}` is a parallel path to actions. Returns a JSON payload. Currently invisible.

---

---

## Increment 4: Persistence read/write edges (Region 06 deep dive)

The diagram currently has 4 Persistence bricks: Profile, Plugin registry, Plugin lock, Mode. The real persistence layer has **15+ distinct files/sockets** spread across **4 base directories**, and Region 06 is best thought of as a **four-way conversation**, not a stack at the bottom of the canvas.

### 4.1 Persistence map - all files, all directories

Three base dirs (per-OS resolution from `dirs` crate):

| Base | Linux | macOS | Windows |
|---|---|---|---|
| `shared_config_dir()` (paths.rs:76) | `~/.config/qol-tray/` | `~/Library/Application Support/qol-tray/` | `%APPDATA%/qol-tray/` |
| `base_data_dir()` (paths.rs:80) | `~/.local/share/qol-tray/` | `~/Library/Application Support/qol-tray/` | `%LOCALAPPDATA%/qol-tray/` |
| `log_dir()` (logging/platform/) | `~/.config/qol-tray/logs/` | `~/Library/Logs/qol-tray/` | `%LOCALAPPDATA%/qol-tray/logs/` |
| `runtime_dir()` (paths.rs:216) | `/tmp/qol-tray/` | `/tmp/qol-tray/` | `/tmp/qol-tray/` |

### 4.2 The full read/write matrix

| File | Path (relative) | Written by | Read by | Trigger |
|---|---|---|---|---|
| Plugin registry | `<config>/plugin-registry.json` | `plugins::registry::save_registry` (`registry/mod.rs:217`, atomic via `.new`+rename) | `plugins::resolver::resolve_from_registry` (`resolver.rs`); `GET /plugins/registry` (`plugin_handlers.rs:67`) | install, uninstall, dev-link, worktree-link |
| Mode config | `<config>/mode.json` | `mode::ModeConfig::set` (`mode.rs:39`); `--write-mode` CLI (`main.rs:97-108`) | `mode::ModeConfig::load` (`mode.rs:29`); `mode_toggle` feature (dev only) | user toggles mode, CLI flag |
| GitHub token | `<config>/.github-token` | `github_auth::storage:save_token` (`storage.rs:49-73`) | `github_auth::storage::load_token` | OAuth flow completes; token revoked |
| GitHub auth | `<config>/.github-auth.json` | `github_auth::storage::save_auth` (`storage.rs:22-44`) | `github_auth::storage::load_auth` (`storage.rs:22`) | OAuth state save |
| Suppressed errors | `<config>/suppressed-errors.json` (or `<log>/suppressed-errors.json` if shared_config_dir fails) | `logging::file_logger::write_suppressed` (`file_logger.rs:186`) | `GET /logs/suppressed-errors` (`logs_handlers.rs:47`) | error suppressor fires; dashboard reads |
| First-run marker | `<config>/.first-run-done` | `main.rs:393-397` after first launch | `is_first_run()` (`main.rs:387-391`) | once, on first boot |
| Active install id | `<data>/active-install-id` | `paths::set_active_install_id` (`paths.rs:117-125`) | `paths::has_active_install_id` (`paths.rs:127`) | installer registers current install |
| Profile manifest | `<config>/profile/manifest.json` | `features::profile::core::storage::save_manifest` (`storage.rs:31`) | `storage::load_manifest` (`storage.rs:21`) | profile import; cloud sync apply |
| Profile plugin lock | `<config>/profile/plugins.lock.json` | `storage::save_plugins_lock` (`storage.rs:47`) | `storage::load_plugins_lock` (`storage.rs:35`) | plugin install/uninstall (profile inventory) |
| Hotkeys config | `<config>/profile/core/hotkeys.json` | `storage::write_json_config` (`storage.rs:95`); `sync::service` on restore (`service.rs:902,908`) | `hotkeys::HotkeyManager::new` (`manager.rs:22`); profile bundle load (`storage.rs:81`) | user edits hotkeys; profile sync restore |
| Shortcuts config | `<config>/profile/core/shortcuts.json` | `shortcuts::store::save` (`shortcuts/store.rs:12`); `storage.rs:101` | `shortcuts::store::load` (`shortcuts/store.rs:7`); `storage.rs:85` | user edits shortcuts |
| Task runner config | `<config>/profile/core/task-runner.json` | `storage.rs:106` | `task_runner::config::load` (`task_runner/config.rs:47`); `storage.rs:89` | task config edit |
| Per-plugin config | `<config>/profile/plugin-configs/<id>.json` | `plugins::config::store::save_config` (`config/store.rs:57,69`); `features::profile::startup::sync_from_profile` (`startup.rs:86`) | `plugins::config::store::load_config`; `startup.rs:62` | plugin saves user settings; profile restore |
| Sync state | `<config>/sync/state.json` | `features::profile::sync::state::save_state` (`state.rs:97`) | `state.rs:88` | every sync round-trip |
| Sync backups | `<config>/sync/backups/<file>.json` | `sync::service::write_backup` (`service.rs:617`); `state.rs:152` | `sync::service` (`service.rs:261,272`) | before destructive sync apply |
| Plugin daemon PID | `/tmp/qol-tray/pids/<plugin_id>.pid` | `daemon_tracker::save_plugin_pid` (`daemon_tracker/mod.rs:40`); `manager::runtime` (`manager/runtime.rs:94`) | `daemon_tracker::list_tracked_pids` (`mod.rs:50`); `orphan_kill` (`orphan_kill.rs:52`) | plugin daemon spawns; supervisor tick; shutdown clears |
| GitHub release cache | `/tmp/qol-tray/cache/plugin-cache.json` | `features::plugin_store::github::cache` (`cache.rs:9`) | same | plugin store fetches release metadata |
| Daily rolling log | `<log>/qol-tray.YYYY-MM-DD` | `tracing_appender::rolling` daily (`file_logger.rs:28-33`) | dashboard `GET /logs/*`; user reading file | every log line; daily rotation |
| Plugin install dir | `<config>/plugins/<plugin_id>/{plugin.toml, <binary>, …}` | installer (`features/plugin_store/installer/`) | PluginLoader (`plugins/loader/`); execution_contract (`execution_contract.rs`) | install completion |
| Per-plugin Unix socket | path from `plugin.toml`'s `daemon.socket` (typically `/tmp/qol-tray-<id>.sock`) | plugin daemon process binds | `action_transport::dispatch_daemon_action` (`action_transport/mod.rs:16`) | daemon ready; every action dispatch |
| Desktop state socket | `/tmp/qol-tray-state.sock` | `runtime::server::socket::run_at` (`runtime/server/socket.rs:14`) | external consumers + plugin daemons via `QOL_TRAY_STATE_SOCKET` env var (`daemon_lifecycle/spawn.rs:77`) | daemon boot; every state subscriber connect |

### 4.3 Two env vars are persistence-adjacent

Plugin daemon subprocesses are spawned with two env vars (passed at spawn time, not files but architecturally part of the persistence-and-IPC seam):

| Env var | Set in | Value | Read by |
|---|---|---|---|
| `QOL_TRAY_DAEMON_SOCKET` | `action_executor::execution::runtime_command` (`execution.rs:118-119`) | path to the plugin's own daemon socket | the spawned one-shot subprocess - so an ephemeral action can call back into its own daemon if it wants |
| `QOL_TRAY_STATE_SOCKET` | `plugins::daemon_lifecycle::spawn` (`spawn.rs:77`) | `/tmp/qol-tray-state.sock` | plugin daemons - so they can subscribe to monitor, cursor, and focus state |

### 4.4 The four conversational quadrants

Region 06 should be drawn as four logical quadrants (not a flat row of 4 bricks):

```
                  config_dir                       profile_dir
                  -----------                      -----------
boot config       plugin-registry.json     ──┐     profile/manifest.json
                  mode.json                  │     profile/plugins.lock.json
                  active-install-id          │     profile/core/{hotkeys,shortcuts,task-runner}.json
                  .github-{token,auth.json}  │     profile/plugin-configs/<id>.json
                  .first-run-done            │
                  suppressed-errors.json     │
                                             │
                  ──────────  user data is the source of truth   ──────────
                                             │
runtime state     /tmp/qol-tray/pids/<id>.pid│     <config>/sync/state.json
                  /tmp/qol-tray/cache/       │     <config>/sync/backups/*
                  /tmp/qol-tray/logs/        │     <log>/qol-tray.YYYY-MM-DD
                  /tmp/qol-tray-state.sock   │
                  per-plugin /tmp/*.sock     │
                  -----------                      -----------
                  ephemeral (/tmp)                 portable (cloud sync)
```

The X axis is **scope of truth**: left half is local-machine state (registry, mode, runtime/, install marker, GitHub auth - all tied to this workstation), right half is the **portable profile** that can be exported/imported and round-tripped through cloud sync.

The Y axis is **lifetime**: top half is durable config (lives in user's config dir, survives reboot), bottom half is ephemeral runtime state (`/tmp` is wiped on boot; logs rotate; sync state is a regenerable conversation log).

### 4.5 Who edges into and out of Persistence

Edges INTO Persistence (writes), grouped by source region:

| Region | Writes to |
|---|---|
| Pre-tokio boot | `active-install-id` (installer), wipe `/tmp/qol-tray/` (paths::init_runtime_dirs), `.first-run-done` (main.rs) |
| Daemon core (Doctor) | `suppressed-errors.json` (via tracing) |
| Daemon core (Logging) | `<log>/qol-tray.*` |
| Daemon core (Profile sync) | `profile/manifest.json`, `profile/plugins.lock.json`, `profile/core/*.json`, `profile/plugin-configs/<id>.json`, `sync/state.json`, `sync/backups/*` |
| Daemon core (Update check) | `<config>/plugins/*` (during update install) |
| Plugin system (Installer via axum) | `plugin-registry.json`, `<config>/plugins/<id>/`, `profile/plugins.lock.json` |
| Plugin system (Manager runtime) | `/tmp/qol-tray/pids/<id>.pid` |
| Plugin system (daemon_tracker) | `/tmp/qol-tray/pids/*` |
| Plugin processes | per-plugin Unix sockets (bind) |
| Platform integration (state socket) | `/tmp/qol-tray-state.sock` (bind) |
| User input (Hotkey edits, mode toggle) | `profile/core/hotkeys.json`, `mode.json` |
| Plugin process action (one-shot) | `profile/plugin-configs/<id>.json` (if plugin saves state) |

Edges OUT of Persistence (reads), grouped by destination region:

| Region | Reads from |
|---|---|
| Pre-tokio boot | `active-install-id`, `mode.json` |
| Daemon core (Bootstrap) | `<config>/.first-run-done` |
| Daemon core (Update check) | `plugin-cache.json` |
| Daemon core (Profile sync) | all of `profile/*`, `sync/state.json` |
| Plugin system (Resolver) | `plugin-registry.json` → produces `ResolvedPlugin[]` |
| Plugin system (Supervisor) | `/tmp/qol-tray/pids/*` (via daemon_tracker) |
| Plugin system (Loader) | `<config>/plugins/<id>/plugin.toml` for each resolved entry |
| User input (Hotkey listener) | `profile/core/hotkeys.json` |
| User input (Shortcut executor) | `profile/core/shortcuts.json` |
| Plugin processes | `profile/plugin-configs/<id>.json` (via plugins/config), QOL_TRAY_*_SOCKET env vars |
| Axum HTTP (dashboard) | nearly everything: registry, suppressed-errors, logs, profile/*, sync state, etc. |

### 4.6 Things that look like persistence but aren't

These appear in the codebase or diagram-adjacent thinking but are NOT files:

- `Daemon::events` (EventBus): in-memory `tokio::sync::broadcast`. Not persisted, not survived across restart.
- `PluginManager::plugins`: in-memory `HashMap<PluginId, Plugin>`. Rebuilt on `load_plugins`.
- `desktop_state::SharedState`: in-memory `Arc<SharedState>` polled by runtime poll thread. Snapshot served over state socket.
- `signal::register_daemon_pid` table: in-memory; the on-disk version is the pids dir.

---

## What this means for the diagram (Increment 4 cuts)

Top-priority changes for the next pass:

1. **Add the four-quadrant frame to Region 06**: scope-of-truth × lifetime, with the bricks placed in the right quadrant. Currently 4 flat bricks; the natural grouping is `boot-config | profile | runtime | sync-state`. This is the single highest-impact visual change.

2. **Add the missing persistence bricks**: `profile/core/{hotkeys,shortcuts,task-runner}.json`, `profile/plugin-configs/<id>.json`, `/tmp/qol-tray/pids/`, `<log>/qol-tray.*`, `sync/state.json`, `.github-{token,auth.json}`. Currently 4 bricks; reality is 15+. At minimum, group them into the quadrants from 4.4.

3. **Draw the read-edges**: from Region 04 Resolver back up to `plugin-registry.json` brick. From User input Hotkeys back up through Daemon core down to `profile/core/hotkeys.json`. From Plugin processes down to `profile/plugin-configs/<id>.json`. The diagram is currently top-down only; Persistence has both inbound writes AND outbound reads.

4. **Two env vars edge out of Persistence area into Plugin processes**: `QOL_TRAY_DAEMON_SOCKET` (executor → ephemeral action) and `QOL_TRAY_STATE_SOCKET` (lifecycle spawn → plugin daemon). These are part of the IPC seam, currently invisible.

5. **Mark the registry/lock distinction explicitly**: the "Plugin registry" brick is the resolver's data; the "Plugin lock" brick is the profile's inventory. They are not the same shape, not consumed by the same readers. Quadrant placement (left vs right of 4.4) makes this obvious.

6. **Per-plugin Unix sockets are persistence-adjacent**: each plugin daemon binds a socket at the path declared in its `plugin.toml`. These are runtime artifacts (created on daemon spawn, deleted on stop) but they are the actual IPC seam. Could appear in the runtime/ephemeral quadrant as `plugin/<id>.sock`.

---

## Sanity checks against the v5 diagram

What the v5 diagram has right:

- IPC seam name `unix socket · ndjson` + `wire: qol_runtime::DaemonResponse` (now with the trace text covering DaemonRequest as well) - correct.
- T1 path: Tray icon → Linux → Menu router → Action executor → Action transport → plugin·A - matches code.
- Daemon core split into pre-tokio + Tokio bands - matches `main.rs` boot order.
- Three IPC channels visible: axum HTTP, Runtime state socket, per-plugin Unix sockets.
- Action executor fan-in with 4 input tags (menu router, hotkey listener, axum POST action, axum GET query) - matches code.
- Daemon tracker brick at `/tmp/qol-tray/pids/<id>.pid` - matches `daemon_tracker/mod.rs:40`.
- Supervisor "5s tick · 5-strike retry · transition → hotkey reload" - matches `daemon_supervisor.rs:7-82`.
- Capabilities brick "serial only · linux" - matches `capabilities.rs:34-40`.
- Profile sync brick covers `features/profile/{core,http,sync}` - matches.

What the v5 diagram is still missing or imprecise on:

- Persistence remains 4 flat bricks; reality is the four-quadrant structure with 15+ files.
- T2 (ephemeral) isn't a selectable trace.
- T5 (install) isn't a selectable trace - the only thing that exercises the Event bus.
- The CLI `qol-tray exec` arrow into axum HTTP isn't drawn (Dashboard UI and CLI both feed axum).
- The two stand-alone binaries `qol-tray-install` and `qol-tray-doctor` are mentioned in the footer ("SAME CRATE, THREE ENTRY POINTS") but there's no diagram element showing them as alternate entry points.
- The fact that supervisor → hotkey reload triggers the **hotkey table to refresh** (not just a generic reload) could be on the same arrow.

---

## Increment 5: Cross-region trace chains (T0 boot, T5 install, T6 hotkey reload)

The diagram now captures static structure well. What's missing are the **multi-region chains** - traces that touch 4-6 regions and explain how the daemon actually lives. Three matter most:

- **T0 - boot** (pre-tokio → Tokio runtime → Plugin system → Plugin processes)
- **T5 - install cascade** (User input → Daemon core → Plugin system → Persistence → Plugin processes, with Event bus on the response side)
- **T6 - hotkey reload** (Plugin system → User input via daemon-state edge)

Each one would be selectable as a trace lens, alongside T1.

### 5.1 T0 - Boot trace (21 steps, pre-tokio + Tokio)

```
[pre-tokio · main thread]
 1. main()                                                              (main.rs:17)
 2. try_handle_cli_flag()                                               (main.rs:71-93)
       └─ --version, --help, --write-mode short-circuit here
 3. try_exec_subcommand()                                               (main.rs:133-157)
       └─ qol-tray exec <plugin> <action> turns this process into an HTTP client
 4. logging::init_logger()                                              (main.rs:26-29)
       └─ tracing_appender::rolling daily → <log>/qol-tray.YYYY-MM-DD
 5. installer::bootstrap_current_install()                              (installer/mod.rs:34-50)
       └─ if no active-install-id: create one, write autostart entry, ensure plugins dir
 6. is_already_running() → TCP probe :42700                             (main.rs:238-242)
       └─ single-instance lock: connect succeeds = another instance owns the port
 7. paths::init_runtime_dirs()                                          (paths.rs:228-242)
       └─ WIPE /tmp/qol-tray/ + recreate pids/ and cache/
 8. housekeeping::run_startup_cleanup(config_dir)                       (housekeeping.rs:4-10)
       ├─ migrate_dev_files: dev-*.json → dev/*.json
       ├─ profile::run_startup_cleanup: hotkeys.json + shortcuts.json + task-runner.json → profile/core/
       │                                plugin-configs.json → profile/plugin-configs/<id>.json
       ├─ clean_legacy_ephemeral: remove .daemon-pids + .plugin-cache.json from config root
       └─ clean_stale_staging: remove .<plugin>.{installing,updating,backup}.* dirs from plugins/
 9. doctor::auto_fix_startup()                                          (doctor/mod.rs:103-112)
       ├─ trigger::take(): pick up any deferred fix from previous session
       ├─ fix_with_policy(with_de_fixes()): apply DE-related fixes
       │   (autostart entries, app launcher registration, file associations)
       └─ log attempts/failures/remaining
10. tray::platform::run_app(app_init)                                   (main.rs:64-68)
       └─ enters the native OS app loop with init callback

[tokio multi-thread runtime - block_on async_init_inner]
11. tokio::runtime::Builder::new_multi_thread().enable_all().build()    (main.rs:268-270)
12. check_for_updates() with 2s timeout                                 (main.rs:295, 428-443)
13. broadcast::channel::<()>(1) for shutdown_tx/rx                      (main.rs:296)
14. runtime::RuntimeServer::start() [Unix only]                         (main.rs:298)
       ├─ spawn poll thread for desktop state (monitors, workspaces)
       └─ spawn socket thread bound at /tmp/qol-tray-state.sock
15. PluginLoader::ensure_plugin_dir()                                   (main.rs:299)
16. SyncService::new(plugins_dir) + spawn pull_on_launch                (main.rs:300-308)
       └─ cloud profile pull happens in background
17. PluginManager::new() + load_plugins()                               (main.rs:309-311)
       ├─ ensure_registry_initialized
       ├─ resolve_from_registry → ResolvedPlugin[] (active/fallback slots)
       └─ manifest_loader::load_resolved_plugin for each → in-memory HashMap
18. Daemon::new() creates EventBus                                     (main.rs:316)
19. FeatureRegistry::new() + register plugin_store + (dev) mode_toggle  (main.rs:317-321)
20. features::plugin_store::Plugins::start_server()                    (main.rs:322-329)
       └─ axum::serve on 127.0.0.1:42700 - the dashboard API + SSE feed
21. hotkeys::start_capture()                                            (main.rs:330-338)
       ├─ try kernel evdev (Linux + feature linux_evdev): /dev/input/event* + /dev/uinput
       └─ on failure, start_hotkey_listener (global_hotkey crate)
22. plugins::daemon_supervisor::spawn_supervisor()                     (main.rs:339-342)
       └─ tokio::spawn supervision loop: 5s tick, classify transitions, restart dead
23. tokio::task::spawn_blocking(launcher_apps::trigger_full_sync)      (main.rs:343)
       └─ full launcher app sync in a blocking task

[main thread - after init returns]
24. TrayManager::new(feature_registry, shutdown_tx, shutdown_rx,        (main.rs:278-284)
                     update_available, events)
25. native tray menu construction; subscribes to EventBus              (tray/platform/{linux,macos,windows}.rs)
26. (if first run) show_first_run_welcome                               (main.rs:286-288)
27. tray::platform::run_app event loop runs forever                    (tray/platform/mod.rs:148-156)
```

Steps 1-10 are synchronous on the OS main thread. The tokio multi-thread runtime is built at step 11. Steps 14-23 are spawned on tokio worker threads. Step 24 returns to the main thread to build the native tray (which MUST be on the main thread on macOS/Windows for the OS).

The diagram could highlight this trace by:

- Dimming everything except T0's path
- Showing arrows in numbered sequence inside Region 03 (currently bricks are peers; T0 would imply order)
- Animating the dashed band crossover: pre-tokio half lights up first, then "TOKIO MULTI-THREAD RUNTIME" crossbar fires, then async tasks band lights up
- Step 24 crosses back from Tokio to main thread → tray. Currently the diagram shows this as static structure; T0 would expose it as a temporal arc

### 5.2 T5 - Install cascade (the multi-region chain)

```
[browser]
 1. fetch('http://127.0.0.1:42700/api/install/<id>', { method: 'POST' })

[axum tokio worker]
 2. axum POST /install/{id} → plugin_handlers::install_plugin          (plugin_handlers.rs:72-79)
 3. plugin_services::install_plugin(state, id)
 4. installer::operation_lock acquires per-plugin lock                 (plugin_store/installer/operation_lock.rs)
 5. installer::source resolves install source (GitHub release URL)     (plugin_store/installer/source.rs)
 6. reqwest downloads tarball + checksum into staging                  (plugin_store/installer/operations.rs)
 7. installer::staging untars to /tmp staging dir
 8. manifest validation on staged plugin.toml                          (plugins/manifest/validation.rs)
 9. execution_contract validates binary-in-dir for staged plugin       (plugins/execution_contract.rs:46-82)
10. atomic move staging dir → <config>/plugins/<id>/
11. registry::save_registry: append active slot for <id>               (plugins/registry/mod.rs:217)
12. profile::core::storage::save_plugins_lock                          (features/profile/core/storage.rs:47)
       └─ updates profile/plugins.lock.json with new entry
13. plugin_manager.reload_plugins() rescans + replaces HashMap         (plugins/manager/mod.rs:27-30)
14. EventBus.send_plugins_changed() → revision++                      (daemon/events.rs:31-35)

[broadcast fan-out - three concurrent subscribers]
15a. axum SSE handler at GET /api/events writes DaemonEvent JSON to browser
15b. tray menu subscribers (one per OS platform impl) rebuild native menu
15c. PluginManager.reload_plugins:
        ├─ stop previously running plugin daemons
        ├─ resolve registry active/fallback slots and load manifests
        ├─ clean stale sockets for the loaded plugins
        ├─ autostart daemon-enabled installed plugins
        ├─ daemon_tracker::save_plugin_pid → /tmp/qol-tray/pids/<id>.pid
        └─ hotkeys::trigger_reload (signals the global_hotkey fallback listener)
```

This trace would light up:
- User input · Dashboard UI (browser) ← step 1
- Tokio runtime / axum HTTP brick ← steps 2-3
- Plugin system / GitHub discovery + Loader + Manifest validation + Registry + PluginManager ← steps 4-13
- Persistence / Plugin registry + Plugin lock + Plugins dir + Plugin PIDs ← steps 11-12, 15c
- Tokio runtime / Event bus ← step 14
- Plugin processes / daemon autostart ← step 15c
- User input / global_hotkey listener reload signal ← step 15c last step

It would be the trace that exercises Event bus and SSE feed. Currently those bricks have no trace that walks through them.

### 5.3 T6 - Hotkey reload (the back-edge from Plugin system to User input)

The supervisor's `trigger_reload` call (`daemon_supervisor.rs:80`) is one of the most non-obvious edges in the system. It runs on every daemon state transition. Its effect depends on which hotkey backend is active:

```
[supervisor 5s tick OR install cascade]
 1. daemon state transition detected (Alive ↔ Dead)                    (daemon_supervisor.rs:96-100)
 2. hotkeys::trigger_reload()                                          (hotkeys/listener.rs)
 3. if global_hotkey fallback is running, listener reloads hotkeys.json
 4. catalog::load_available_actions collects actions from loaded plugin manifests
 5. enabled bindings are re-registered with global_hotkey
```

This is a back-edge: Plugin system writes to a behavior that lives in User input. The Linux kernel evdev capture backend currently has no equivalent reload channel, so the edge should be labeled as a global_hotkey fallback reload signal, not a universal hotkey rebinder.

### 5.4 What goes in the trace selector

After Increments 1-5, the trace selector could have 6+ lenses:

| ID | Name | Path through regions | Key insight |
|---|---|---|---|
| T0 | Boot | (none → Region 03 → Region 04 → Region 02 tray) | Pre-tokio vs Tokio vs main-thread phases |
| T1 | Tray → daemon | 01 → 02 → 03 (menu router) → 04 (executor + transport) → 05 (plugin A) | Standard action dispatch, OS thread |
| T2 | Tray → ephemeral | 01 → 02 → 03 (menu router) → 04 (executor only) → spawn subprocess | Subprocess spawn, no Unix socket |
| T3 | Hotkey → action | 01 (hotkey) → 03 (executor) → 04 → 05 | Fan-in convergence |
| T4 | Dashboard → action | 01 (browser) → 03 (axum) → 04 → 05 | axum worker blocks on Unix socket I/O |
| T5 | Install cascade | 01 → 03 (axum) → 04 (installer + manager) → 06 (registry + lock + pids) → 05 (daemon autostart) → 03 (event bus) → 02 (tray rebuild) + 01 (SSE to browser + hotkey reload signal) | The only trace that exercises Event bus |
| T6 | Hotkey reload | 04 (supervisor) → 01 (global_hotkey listener) → 06 (hotkeys.json read) → loaded action catalog → global_hotkey re-register | Plugin system writes to User input behavior when fallback listener is active |

The T5 lens is the killer one. It's the only trace that justifies Event bus as a first-class brick.

---

## What this means for the diagram (Increment 5 cuts)

Top-priority changes for the next pass:

1. **Add T0 (boot) as a trace lens**: walk through the dashed-band structure that already exists in Region 03 (pre-tokio · main thread / TOKIO MULTI-THREAD RUNTIME / async tasks). T0 is the only trace where those bands matter; without it they look decorative.

2. **Add T5 (install cascade) as a trace lens**: this is the single highest-information-density trace, because it exercises manager reload, daemon autostart, Event bus, SSE, tray menu rebuild, hotkey reload signaling, and three persistence writes. It also takes 14 steps in axum + a fan-out after reload.

3. **Add T6 (hotkey reload) as a trace lens**: the system's most counter-intuitive edge, but label it as a global_hotkey fallback behavior rather than a universal backend behavior.

4. **Numbering on Region 03 boot bricks**: Bootstrap → Runtime dirs → Housekeeping → Doctor → (TOKIO START) → Update check → Profile sync → Feature registry → axum HTTP → Hotkeys → Supervisor. Add small step numbers (1-11) when T0 is active.

5. **EventBus brick gets a "broadcasts to" sub-label**: PluginsChanged | UpdateProgress | BuildComplete (dev). With T5 active, show outbound arrows to SSE/browser and tray menu subscribers.

6. **Two-character "phase" tags on Region 03 bricks**: `[sync]` for pre-tokio bricks, `[task]` for tokio-spawned bricks. Or use the existing dashed-band placement and just lean on T0 to make it visible.

---

## Increment 6: Operational hardening (dispatch, lifecycle, doctor, http surface, sockets, capabilities)

Increments 1-5 established what the daemon IS. Increment 6 captures the safety + recovery seams that govern how it RUNS: how the dispatcher chooses between paths, how plugin daemons are spawned and killed, how the doctor self-heals, how the HTTP surface protects itself, how stale sockets get reaped, and what the capability model actually enforces.

### 6.1 Resolver fallback - derived, not manifest-authored

`plugins/action_executor/resolution.rs` is the decision point that determines whether a daemon refusal or unavailable socket flows through to a fallback runtime spawn.

The current schema has no per-action `runtime_fallback` field. Fallback is derived from `daemon.socket`, `runtime.command`, and whether the resolved runtime and daemon command paths are the same binary:

| daemon socket | runtime.command | daemon command relation | Result |
|---|---|---|---|
| None | Some | n/a | runtime spawn is the execution target |
| Some | None | n/a | daemon-only; no fallback path |
| Some | Some | daemon command missing | runtime fallback allowed |
| Some | Some | runtime path differs from daemon path | runtime fallback allowed |
| Some | Some | same resolved path | runtime fallback allowed only when the daemon socket is unreachable |

If `runtime.actions` is present, every executable action must have an explicit action-to-args mapping. If `runtime.actions` is absent, the runtime command receives the action id as its single argument.

This is the most consequential rule in the dispatch graph: it decides whether T1 degrades to T2 when a plugin daemon returns `Fallback` or the socket is unavailable. A daemon `Error` remains an action error.

### 6.2 Action dispatcher entry points - 4 public surfaces

`plugins/action_executor.rs` exposes four public functions. Each enters the dispatch graph at the same internal `try_execute_action` after light caller-specific framing.

| Function | Caller | Semantics |
|---|---|---|
| `execute_action` | tray menu router, hotkey listener | Resolve + dispatch + log. Fire-and-forget with ack. |
| `try_execute_action` | axum POST /api/plugins/:id/actions/:action | Same as above but returns `Result` for HTTP responses. |
| `dispatch_query` | axum GET /api/plugins/:id/queries/:query | Read-only path. Returns the plugin's `DaemonResponse.data` JSON as the HTTP body. |
| `dispatch_action_by_name` | dev tools | Looks up the action by its public name (not ID), used by CLI introspection. |

The query path is structurally distinct: it always goes through `action_transport::dispatch_daemon_action` (no fallback to runtime spawn), and the response payload IS the response. The diagram's T7 trace covers it.

### 6.3 Daemon spawn - env vars, process group, two-mode readiness

`plugins/daemon_lifecycle/spawn.rs` builds a `std::process::Command` with three side effects beyond the obvious binary + args + cwd.

Three environment variables get set on the child:

| Env var | Value | Why |
|---|---|---|
| `QOL_TRAY_DAEMON_SOCKET` | daemon_socket path from `plugin.toml` | so a runtime-spawn ephemeral action can call back into its own daemon |
| `QOL_TRAY_DAEMON_REPLACE_EXISTING` | `"1"` | tells the starting plugin daemon to unlink any existing socket at its path before binding |
| `QOL_TRAY_STATE_SOCKET` | `/tmp/qol-tray-state.sock` (Unix) | so plugin daemons can subscribe to monitor, cursor, and focus state |

`REPLACE_EXISTING=1` is the plugin-side half of stale socket recovery. The host also calls `daemon_tracker::clean_stale_sockets(&plugins)` during plugin load, after it knows the loaded plugin socket paths.

`apply_process_group` invokes `libc::setsid()` inside `pre_exec` so the child becomes the leader of a new process group. This is the foundation for clean teardown: `terminate_daemon` in `readiness.rs` later sends SIGTERM via `libc::kill(-(pid), SIGTERM)`. The **negative pid** means "send to the whole process group", so any sub-processes the plugin daemon itself spawned die with it. Without setsid, a misbehaving plugin daemon could orphan grandchildren.

Readiness has two modes (`readiness.rs:1-128`):

| Mode | Trigger | Mechanism |
|---|---|---|
| Socket poll | `daemon_socket` is set | Connect-test the socket every 50ms up to a 5s deadline. First successful connect → ready. |
| Wait-for-exit | No socket (one-shot) | Sleep 100ms, then `try_wait` on the child handle. Treats non-zero exit as failure. |

These map to the two plugin shapes from Increment 2.3: daemon (long-lived, socket-bound) vs runtime-only (ephemeral).

### 6.4 Doctor - 2 checks, 8 fixes, 2 policies

`doctor/mod.rs` is structured as a fixed enum of `CheckId` and `FixAction`. The whole module is data-driven: adding a new check or fix is one variant + one handler in the dispatch table. No N-way if/else.

**CheckIds (read-only diagnostics)** - 2 variants:

| CheckId | What it scans |
|---|---|
| `PluginProcessLeaks` | Compare `/tmp/qol-tray/pids/*.pid` against actually-alive processes. Stale entries = leaks. |
| `HotkeyShadows` | Detect hotkeys that another desktop binding already owns (DE shortcuts, IDE bindings). |

**FixActions (mutate state)** - 8 variants:

| FixAction | Effect |
|---|---|
| `SetActiveInstallId` | Write `<data>/active-install-id` if missing |
| `WriteInstallMarker` | Write `<config>/.first-run-done` |
| `WriteAutostartEntry` | Per-OS autostart entry (`.desktop`, launchd plist, registry Run key) |
| `EnsurePluginsDir` | Create `<config>/plugins/` if missing |
| `KillPluginProcessLeaks` | Kill processes from stale pid files; clean up |
| `UnshadowDeBinding` | Unbind a DE-owned shortcut so qol-tray can claim it |
| `DisableSymbolicHotkey` | macOS-specific: disable a system symbolic hotkey |
| `ClearWindowsAppKey` | Windows-specific: clear a conflicting AppKey registry entry |

(Plus `InstallShellHook` for shell PATH integration.)

**FixPolicy** gates which fixes the doctor is allowed to apply:

| Policy | Allowed fixes |
|---|---|
| safe | First 5 fixes (idempotent, no DE/OS-level changes) |
| with_de_fixes | All 8 fixes (includes DE/OS-level unshadow + system key changes) |

`auto_fix_startup` uses `with_de_fixes` so the daemon self-heals DE collisions on every boot. The standalone `qol-tray-doctor` CLI defaults to `safe` and requires `--apply-de-fixes` to enable the full set.

### 6.5 Capability registry - the security framework that almost nothing uses

`plugins/capabilities.rs:1-50` defines `PermissionState` with 4 variants:

| State | Meaning |
|---|---|
| `Granted` | capability is available |
| `Fixable` | user action would grant it (e.g. join group, install pkg) |
| `RequiresLogout` | granted but won't take effect until next session |
| `Denied` | not granted, not fixable |

And `REGISTRY` as a `&[CapabilityRule]` slice. The framework is fully wired - the dispatcher calls `verify_capabilities` before dispatch - but the `REGISTRY` const has **one rule**: serial port access on Linux (a `dialout`/`uucp` group membership check). macOS and Windows registries are empty.

So most plugin permissions are currently unenforced at the host level. The framework exists for future plugin authors to add rules without changing the dispatcher.

### 6.6 Execution contract - binary-in-dir + dev candidates

`plugins/execution_contract.rs:60-180` resolves a plugin's `command` to an actual binary, refusing anything that resolves outside `plugin_dir`.

Candidate paths are tried in source-aware order:

| Source | Order |
|---|---|
| Live source (`DevLink`/`WorktreeLink`) | `target/debug/<command>`, then `target/release/<command>`, then `<plugin_dir>/<command>` |
| Installed/release source | `<plugin_dir>/<command>`, then dev candidates when compiled with `feature = "dev"` |
| Windows suffix | after the source-aware candidates, `<plugin_dir>/<command>.exe` is tried when the command has no extension |

After resolving, `is_allowed_candidate` canonicalises the path (resolves symlinks) and verifies it starts with the canonicalised `plugin_dir` prefix. This blocks symlink escape from a plugin directory into the rest of the filesystem.

### 6.7 axum surface - merged routers, dev gate, cross-site mutation guard

`features/plugin_store/server.rs` mounts axum at `127.0.0.1:42700`. All API routes are nested under `/api`, but the profile and meta routes are not under `/api/profile` or `/api/meta`; their route functions define their final paths directly.

| Surface | Example paths | Purpose |
|---|---|---|
| Plugin API | `/api/plugins`, `/api/plugins/{id}/actions/{action}`, `/api/plugins/{id}/queries/{query}` | Action + query dispatch (T4, T7) |
| Install API | `/api/install/{id}`, `/api/update/{id}`, `/api/uninstall/{id}` | Install/update/uninstall (T5) |
| Events | `/api/events` | SSE DaemonEvent stream |
| Profile config | `/api/config/export`, `/api/config/import` | Profile import/export |
| Profile sync | `/api/sync/providers`, `/api/sync/status`, `/api/sync/pull`, `/api/sync/push`, `/api/sync/backups` | Sync providers and backup control |
| Logs | `/api/logs/*` | Suppressed errors + tail (logs_handlers) |
| Meta | `/api/version`, `/api/check-update`, `/api/self-update`, `/api/dev/enabled` | Daemon metadata and update operations |
| Dev API | `/api/dev/*` | Dev-only routes - gated by `require_dev_mode` middleware |

The `require_dev_mode` middleware checks the current mode (read from `<config>/mode.json`) and 404s if mode is not "dev". So in release mode none of the dev-only routes are reachable, even by a localhost client.

`features/plugin_store/server/security.rs` adds `reject_cross_site_mutations`, an axum middleware that runs on every `/api` request:

| Step | Rule |
|---|---|
| 1 | If method is GET/HEAD/OPTIONS → allow (read-only). |
| 2 | Else (POST/PUT/PATCH/DELETE) allow same-origin/same-site/none `Sec-Fetch-Site` values. |
| 3 | If `Origin` is present, require `http://127.0.0.1:42700`, `http://localhost:42700`, or `http://[::1]:42700`. |
| 4 | Otherwise reject with 403. Missing `Origin` is allowed. |

This is a CSRF guard: a malicious page on another origin cannot trigger writes against the local daemon, even though `127.0.0.1:42700` is technically reachable by any browser on the machine.

`server.rs:130` also has `start_sync_loop` which spawns `auto_push_if_dirty` as a tokio task on `auto_push_interval()` cadence - the periodic cloud-push half of profile sync.

### 6.8 Stale socket reaper - plugin-load cleanup with a liveness probe

`plugins/daemon_tracker/platform/socket_cleanup.rs` runs during plugin loading (called from `plugins/manager/loading.rs` after registry resolution and manifest load) and cleans up plugin sockets that survived a previous crash. Four steps:

| Step | Action |
|---|---|
| 1 | Scan known plugins: for each `plugin.toml`-declared `daemon.socket`, check if it exists on disk. |
| 2 | Orphan scan: walk runtime temp dirs for managed socket names that start with `qol-` and end with `.sock`. |
| 3 | For each candidate, call `has_live_listener(path)`: try `connect()` - success means a listener exists (leave alone), failure (ECONNREFUSED, ENOENT) means stale (unlink). |
| 4 | Skip symlinks and non-sockets; on Windows this cleanup is a no-op. |

The liveness probe is what makes this safe. Without it, a fresh boot that runs before the previous daemon's children have fully exited could unlink a socket the previous process group is still listening on. The probe converts "file exists" (unreliable) into "listener exists" (canonical).

---

## What this means for the diagram (Increment 6 cuts - applied)

All seven cuts are now reflected in `diagram/data.js`:

| Cut | Card | Change |
|---|---|---|
| 1 | Action resolver | sub now "socket · runtime.command · path equality"; bullets enumerate the derived fallback policy |
| 2 | T8 trace | "Stale socket recovery" walks plugin-load cleanup + plugin unlink-on-bind + connect-probe sequence end-to-end |
| 3 | Action executor | sub now "fan-in · daemon|runtime fork"; bullets expose the `daemon_socket=Some/None` branch |
| 4 | axum HTTP | sub now names the merged router surfaces; ipc notes `/api/events` and `/api/dev/*` gated |
| 5 | Doctor | sub now "2 checks · 8 fixes · safe|with_de_fixes policy" |
| 6 | Capabilities | sub now "registry · near-empty"; bullets surface framework-vs-enforcement gap |
| 7 | Manifest validation | sub mentions candidate ladder; bullets expose `execution_contract` gating, candidate paths, symlink-safe canonicalisation |

---

## Increment 7: dev-mode subsystem

The diagram's `mode_toggle (dev)` brick and `dev_api_router` gate (documented in 6.7) are the only surface markers for an entire subsystem behind the dev gate. This increment maps it.

### 7.1 Mode toggle - one file, no cache

| Concern | File:line | Detail |
|---|---|---|
| Read | `src/mode.rs:28-35` | `ModeConfig::load()` reads `config/mode.json` on every call; default = `Dev` if compiled with `feature = "dev"`, else `Prod` |
| Write | `src/mode.rs:38-41` | `ModeConfig::save()` writes pretty-printed JSON via `file_io::write_pretty_json` |
| Toggle | `src/features/mode_toggle.rs:39-54` | Reads current, flips, saves. Stateless. Triggered by tray menu click. |
| Gate | `src/features/plugin_store/server/dev_gate.rs:8-17` | `require_dev_mode` middleware loads `ModeConfig` per request; 404 if not dev |

**Gotcha**: no cache. Every dev-API request pays a file read. Acceptable because the dev surface is internal-only.

### 7.2 Worktree picker - feature-grouped layout convention

Scans ancestor directories for a `worktrees/<branch-path>/<repo-name>/` layout. Walks until it finds `<repo-name>/Cargo.toml` + `.git`, then resolves branch via `git rev-parse --abbrev-ref HEAD`.

| Concern | File:line | Detail |
|---|---|---|
| Scan | `src/features/plugin_store/server/dev_services/worktrees.rs:6-21` | Entry; max depth 5 |
| Collect | same file:29-43 | `collect_feature_grouped` enumerates branches |
| Branch resolve | same file:86-92 | Shells `git rev-parse --abbrev-ref HEAD` per match |
| List route | `dev_handlers::list_worktrees_handler` | GET /api/dev/worktrees |

**Supported layout**: `worktrees/feat/x/qol-tray/`. **Rejected**: flat layout, repo-grouped layout, foreign-repo paths. The convention is load-bearing - rename the dir layout and the picker silently returns empty.

### 7.3 Self-recompile pipeline

The daemon can rebuild qol-tray itself in dev mode, then hot-swap the process.

| Step | File:line | Detail |
|---|---|---|
| Trigger | `dev_handlers.rs:69-84` | POST /api/dev/recompile-self · optional `{ branch }` in body |
| Queue | `dev_services/recompile/start.rs:11-27` | Acquires build lock via `state.runtime.try_start_self_recompile()`; returns 409 if busy |
| Build | `dev/build/cargo_build/self_build.rs:15-40` | Spawns `cargo build` with piped stdout/stderr |
| Progress | `self_build/artifacts.rs:51-122` | Parses `--message-format=json` artifact lines; emits percent (clamped 0-95) + crate name as `DaemonEvent::SelfRecompileProgress` |
| Stderr | same file:117-121 | Forwarded line-by-line, unfiltered |
| Result | `dev_services/recompile/result.rs` | Success → restart binary; failure → emit `SelfRecompileFailed` event with last non-empty stderr line |

**Process replacement**: ties to `REPLACE_EXISTING=1` (Increment 6.3) and the supervisor's restart logic. The new binary is launched; the old one exits after handoff. Stale socket reaper (6.8) covers the race.

### 7.4 Dev-link - registry slot type

A dev-link points an installed plugin's binary path at a local worktree's `target/debug/<binary>` instead of the released asset. Persisted in the same `plugin-registry.json` as releases, via a slot-type enum.

| Concern | File:line | Detail |
|---|---|---|
| Slot enum | `src/plugins/registry/mod.rs:40-51` | `SlotSource` = `ReleaseAsset` / `DevLink { origin_path }` / `WorktreeLink { origin_path, branch }` |
| Create | `dev/linking/store.rs:23-35` | Validates source exists, has `plugin.toml`, derives plugin id from dir name |
| Record | `registry/mod.rs:123-139` | Demotes previous `ReleaseAsset` to fallback; promotes `DevLink` to active |
| Remove | same file:153-189 | Inverse - removes dev-link, promotes fallback back to active |
| Resolve | `src/plugins/resolver.rs:131-135` | Maps `DevLink`/`WorktreeLink` to `PluginSource::DevLinked` |
| Worktree-aware path | `dev/build/planning/worktree.rs:5-13` | `resolve_worktree_paths` rewrites path per active branch; falls back to original on `git worktree list` failure |

**Gotcha**: plugin id is derived from the directory leaf name, not the manifest. Renaming the dir mid-link changes the id silently.

### 7.5 Mock handlers - 7 in-memory simulators

Dev-only HTTP routes that fake long-running operations (build completion, self-update, update check) without invoking real cargo or network. Used by the dashboard for UI development.

| Route | Behaviour |
|---|---|
| GET /api/dev/mock-check-update | Returns `{ available: true, latest: "99.0.0" }` |
| POST /api/dev/mock-plugin-build · /stop | Triggers/cancels async mock build completion |
| POST /api/dev/mock-self-recompile · /stop | Same shape, for self-recompile |
| POST /api/dev/mock-self-update · /stop | Same shape, for self-update |
| GET/POST /api/dev/mock-targets · /start · /stop | Enumerate + drive the 3 mock states in bulk |
| GET /api/dev/update-fixture.tar.gz | Serves a fixture binary tarball |

State lives in `dev_runtime_state.rs:12-32` as 3 instances of `MockTargetState { in_progress: AtomicBool, cancel: AtomicBool }`. No persistence - in-memory only, reset on daemon restart.

### 7.6 Plugin CPU monitor - 1s tick, 60-sample ring

Per-plugin CPU usage sampler. Runs as a background tokio loop in dev mode; broadcasts snapshots via the EventBus for the dashboard.

| Concern | File:line | Detail |
|---|---|---|
| Constants | `dev_plugin_cpu/mod.rs:13-14` | `SAMPLE_INTERVAL = 1s`, `HISTORY_LIMIT = 60` |
| Loop | same file:41-53 | Spawns tokio task; emits `DaemonEvent::PluginCpuSnapshot` per tick |
| macOS sampler | `dev_plugin_cpu/platform/macos.rs:9-26` | `libc::proc_pid_rusage(pid, RUSAGE_INFO_V2)`; converts via mach timebase; returns user+system microseconds |
| Linux sampler | `dev_plugin_cpu/platform/linux.rs` | Equivalent via procfs (not detailed) |
| PID collection | `dev_plugin_cpu/sampling/pid_collection.rs:19-35` | Includes daemon PID + transient action subprocess PIDs |

**Gotcha**: measures CPU time (microseconds of user+system), not wall-clock utilisation. The dashboard does the rate calculation client-side.

### 7.7 Dev API router - complete inventory

The dev_api_router gates 22 routes behind `require_dev_mode`. All also pass through `reject_cross_site_mutations` (CSRF guard). Full route list:

| Path | Method | Purpose |
|---|---|---|
| /api/dev/reload | POST | Queue plugin rebuild from linked sources |
| /api/dev/reload/{plugin_id} | POST | Queue single plugin rebuild |
| /api/dev/recompile-self | POST | Self-rebuild (see 7.3) |
| /api/dev/worktrees | GET | Enumerate available worktrees (see 7.2) |
| /api/dev/active-worktree | GET | Selected branch + current repo branch |
| /api/dev/links | GET / POST | List / create dev-links (see 7.4) |
| /api/dev/links/{id} | DELETE | Remove dev-link |
| /api/dev/log-controls/{id} | PUT | Mute / filter plugin logs |
| /api/dev/log-controls | GET | List all plugin log filters |
| /api/dev/mock-* | various | 7 mock handlers (see 7.5) |
| /api/dev/test-self-update | POST | Full self-update pipeline test |

**Conflict response**: reload/recompile endpoints return 409 CONFLICT if a build is already in progress. Mock targets and real builds are independent (no mutex contention between them).

---

## Increment 8: failure modes

This is the systematic dual of Increment 5 (happy-path traces). For each failure surface: what code catches it, what the user sees, and whether the doctor audits it after.

### 8.1 Plugin daemon crashes mid-RPC

| Concern | Outcome |
|---|---|
| Transport | `action_transport/platform/unix_common.rs:14-25` collapses all socket errors (ECONNREFUSED, broken pipe, EOF, read timeout) into `DaemonActionDispatch::Unavailable` |
| Timeout | `SOCKET_IO_TIMEOUT_MS = 10_000` |
| Retry | None - no backoff, no second attempt |
| User-visible | Action returns "daemon unavailable"; logged as `ActionExecutionError::ActionRejected`; nothing surfaced in tray |
| Doctor catch | No - no audit trail |

The supervisor's process-tree poll (5s tick) will eventually notice the dead daemon and respawn, but the in-flight action is already lost.

### 8.2 Plugin daemon spawn failed

| Concern | Outcome |
|---|---|
| Path resolve | `daemon_lifecycle/spawn.rs:48-61` bails with anyhow "Daemon executable not found for command" |
| Spawn | `spawn.rs:24,37` propagates `std::io::Error` up to `Result<()>` |
| Readiness | `daemon_lifecycle/readiness.rs:43-54` polls socket for 5s; on timeout kills child and bails "failed to bind socket within timeout"; if child exits early, bails "exited immediately with <status>" |
| Supervisor reaction | `daemon_supervisor.rs:132-140` logs warn and increments `record_failure()` (max 5 before backoff) |
| User-visible | Log line only: "Failed to restart daemon for plugin X" |
| Doctor catch | No proactive check; failure is logged |

### 8.3 Stale socket on boot, unlink fails

Already covered for happy path in 6.8. Failure delta:

| Concern | Outcome |
|---|---|
| Unlink call | `daemon_tracker/platform/socket_cleanup.rs:131-134` calls `fs::remove_file()` with silent error suppression |
| If unlink fails | Socket file remains on disk |
| Next consequence | New daemon attempts to bind same path. Plugin's own `REPLACE_EXISTING=1` logic re-attempts unlink before bind. If that also fails, bind returns "Address already in use" after 5s readiness timeout |
| Doctor catch | No - cleanup is silent |

### 8.4 Two installs collide (socket / command / hotkey)

Three sub-cases, three different handling levels.

| Collision | Detection | User-visible |
|---|---|---|
| Same `daemon.socket` path | None pre-install; runtime: second daemon to start wins via `REPLACE_EXISTING=1`; first plugin's actions then fail with Unavailable | Silent for losing plugin |
| Same action ID across plugins | None - dispatcher resolves by plugin_id first, so collision is in menu namespace only | Both visible in menu (potentially confusing labels) |
| Same hotkey | `hotkeys/manager.rs:80-96` - `GlobalHotKeyManager::register` returns Err on grab failure; captured as `RegistrationError` and stored in `registration_status::ERRORS` | Hotkey not registered; visible via `get_registration_errors()` API in tray UI |

Hotkey collisions also trigger `doctor::trigger::mark_needed("hotkey_shadows", message)` - the doctor catches them post-hoc. Socket and action-id collisions are not audited.

### 8.5 GitHub auth expires

| Concern | Outcome |
|---|---|
| Token source | `credentials::github_bearer_token()` |
| Validation | `features/profile/sync/providers/github.rs:20-24` calls `validate_token()` |
| HTTP failure path | same file:44-88 - 401/403 → `ProviderError::Auth("GitHub authentication failed: <status> <body>")` |
| Persistence | `SyncActionResult::Error { ... }` written to state file |
| User-visible | Tray sync indicator shows error; no auto-refresh |
| Doctor catch | No |

### 8.6 Disk full / write fails

| Concern | Outcome |
|---|---|
| Boot | `paths::init_runtime_dirs` `fs::create_dir_all` error → daemon boot aborts with "Failed to create runtime subdir <subdir>" |
| Profile sync save | `std::fs::write` failure → logged, returned as async error |
| Doctor trigger writes | `doctor/trigger.rs:23-31` - atomic-rename writes, errors returned to caller |
| In-memory state | Preserved (last successful state) |
| User-visible | Boot fails OR sync ops fail; existing state remains until next successful write |
| Doctor catch | No proactive "disk space" check |

### 8.7 Hotkey collision with system / another app

Covered in 8.4 above. The `hotkey_shadows` doctor trigger is the most reliable failure surface in the entire codebase - it's the only path where the doctor actively audits a runtime failure.

### 8.8 HTTP port 42700 already bound

| Concern | Outcome |
|---|---|
| Bind | `features/plugin_store/server.rs:172-176` - `TcpListener::bind("127.0.0.1:42700")` |
| If in use | Returns `io::Error { kind: AddrInUse }`; **no retry, no alternate port, no kill of old daemon** |
| User-visible | Daemon boot fails with "Address already in use"; tray + dashboard unreachable |
| Doctor catch | No |

This is one of the few hard-fail paths. The expectation is that single-instance enforcement (`installer::bootstrap_current_install`) catches duplicates before boot reaches this point.

### 8.9 Manifest invalid / plugin.toml malformed

| Concern | Outcome |
|---|---|
| Read | `plugins/loader/manifest_loader.rs:40-41` - I/O error wrapped with context "Failed to read plugin.toml" |
| Parse | same file:44-51 - `toml::from_str` failure wrapped "Failed to parse plugin.toml" |
| Validate | same file:53-64 - `validate_execution_contract_for_source` failure wrapped "Invalid plugin.toml contract" |
| Loader filtering | `plugins/loader/mod.rs` - `filter_map` silently drops failed plugins from the loaded list |
| User-visible | Plugin simply absent from UI; log line only |
| Doctor catch | No - missing/invalid plugins are not audited |

This is the most user-hostile failure mode in the system. A plugin that fails to parse simply vanishes; debugging requires reading the log file.

### 8.10 IPC message oversized / malformed

| Concern | Outcome |
|---|---|
| Size limit | None enforced host-side. `BufReader::read_line` allocates unbounded |
| Time limit | `SOCKET_IO_TIMEOUT_MS = 10_000ms` (10s) - eventual safety net |
| Malformed JSON | `action_transport/protocol.rs:4-22` - text fallback: checks for keyword prefix ("handled", "fallback", "error"); otherwise `Unavailable` |
| User-visible | Oversize → 10s wait then Unavailable; malformed → immediate Unavailable |
| Doctor catch | No |

A 100MB blob from a misbehaving plugin would either timeout or OOM the host. No size cap.

### 8.11 Profile sync conflict

| Concern | Outcome |
|---|---|
| Detection | `features/profile/sync/resolve.rs:9-22` - returns `SyncAction::Conflict` if (both sides differ from `last_synced`) OR (last_synced is None and hashes differ) |
| Auto-resolve | None. Last-write-wins is NOT applied. |
| Backup | Service writes `write_backup_file("conflict")` before returning Conflict |
| User-visible | Tray shows conflict state; user prompted to pull (keeps remote) or push (keeps local) |
| Doctor catch | Yes - conflict incidents tracked in state file; `list_backup_entries()` audits them |

### 8.12 Self-recompile fails (dev mode only)

| Concern | Outcome |
|---|---|
| Build failure | `dev_services/recompile/result.rs:5-40` - `handle_recompile_result` checks `build.success` |
| Error extraction | `build_failure_message` - last non-empty line of stderr |
| Event | Emits `DaemonEvent::SelfRecompileFailed { message }` |
| Old binary | Stays alive. No restart attempted on failure. |
| Restart binary missing | `restart_schedule.rs:61-77` - `resolve_restart_binary` failure emits `SelfRecompileFailed { "Restart binary not found after build" }` |
| User-visible | Tray/dashboard shows "Self recompile failed: <last line>"; daemon continues on old code |
| Doctor catch | No |

---

## Failure-mode summary

| # | Scenario | Handling | User-visible | Doctor audit |
|---|---|---|---|---|
| 8.1 | Daemon crash mid-RPC | None (bubbles to Unavailable) | Action returns unavailable | No |
| 8.2 | Daemon spawn fails | Logged; supervisor retries 5× with backoff | Log only | No |
| 8.3 | Stale socket unlink fails | Silent; next bind attempt may fail | Daemon startup error | No |
| 8.4 | Socket / command / hotkey collision | Hotkey only: captured as `RegistrationError` | Hotkey errors visible in tray | Yes (hotkey_shadows only) |
| 8.5 | GitHub token expired | `ProviderError::Auth` returned | Sync error in tray | No |
| 8.6 | Disk full / write fails | Boot aborts OR sync fails async | Daemon boot fails or sync error | No |
| 8.7 | Hotkey system collision | (same as 8.4 hotkey row) | Visible in tray UI | Yes |
| 8.8 | Port 42700 in use | Boot fails, no retry | Tray unreachable | No |
| 8.9 | Manifest parse fails | Plugin filtered out silently | Plugin absent from UI | No |
| 8.10 | IPC oversized / malformed | 10s timeout, then Unavailable | Action unavailable | No |
| 8.11 | Profile sync conflict | Backup written; user resolves manually | Conflict status in tray | Yes (backup audit) |
| 8.12 | Self-recompile fails | Daemon stays alive on old binary | "Recompile failed" in tray | No |

**Cross-cutting patterns**:
1. Most I/O failures bubble up as `Unavailable` with silent suppression - generous resilience, sparse observability.
2. Only **hotkeys** and **profile sync conflicts** trigger doctor audit paths. Everything else is fire-and-forget logging.
3. No auto-kill/respawn on transient errors; supervisor relies on its 5s process-tree poll.
4. Socket and command-id collisions are not prevented at install time. `REPLACE_EXISTING=1` is a best-effort runtime override.
5. Manifest parse failures are the user-hostility hotspot - the plugin silently vanishes from the UI.

---

## After Increment 8

The findings doc now covers:
- Increment 1: Static shape, identity, IPC channels, EventBus correction
- Increment 2: Plugin system depth + two-registry-file split + missing bricks
- Increment 3: T1-T5 concrete call chains with file:line
- Increment 4: Persistence read/write matrix + four-quadrant frame
- Increment 5: Multi-region trace chains (T0 boot, T5 install, T6 hotkey reload)
- Increment 6: Operational hardening - resolver fallback matrix, daemon spawn env + process group + SIGTERM(-pid), doctor model, capability registry, execution_contract candidate ladder, axum dev gate + CSRF guard, stale-socket reaper with liveness probe
- Increment 7: Dev-mode subsystem - mode toggle, worktree picker, self-recompile pipeline, dev-link slot model, mock handlers, plugin CPU monitor, dev_api_router (22-route inventory)
- Increment 8: Failure modes - 12 scenarios with handling / user-visible / doctor-audit columns, plus 5 cross-cutting patterns

The diagram pairs with this doc via 9 named traces (T0-T8) - see "Trace index" near the top.

What is NOT covered (and where to read instead):
- Plugin authoring guide → look at any plugin's `plugin.toml` + `src/main.rs` as a worked example; the loader and execution_contract code is the formal contract.
- Tray-icon library internals → external dep (`tray-icon` crate), out of scope here.
- HTTP/SSE protocol details → `axum 0.8` docs and `features/plugin_store/server/` handler shapes.
- WebSocket / SSE event payload schemas → enum variants on `DaemonEvent` in `daemon/mod.rs:28-94`.
