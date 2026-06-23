# qol-tray Plugin Contract

The single hub for how a plugin talks to qol-tray: what it declares, how config
reaches it, how actions are dispatched, how its process is spawned and guaranteed
to die, and every other channel between a plugin and the host.

This is a reference, not a tutorial. It cites the source of truth so the facts
stay checkable; line numbers are hints (paths and symbol names are durable). When
in doubt, the code wins - if you find this doc wrong, fix it in the same change.

Related skills (load these for surrounding context): `qol-tray:qol-tray-core`
(plugin contract overview), `qol-project:qol-arch-channels` (which channel to use
when adding a new one), `qol-tray:qol-tray-rust` (host backend), `qol-langs:gpui-conventions`
(gpui), `qol-tray:qol-tray-dev-recompile` (the reload-everything button).

## Contents

1. Mental model and channel inventory
2. Declaration files (`plugin.toml`, `qol-config.toml`, `qol-runtime.toml`)
3. Config delivery at runtime
4. Action dispatch (hotkey / dashboard / launcher to effect)
5. Process lifecycle and ownership (daemon protocol + the host-death watchdog)
6. The platform-state socket
7. The `gpui` capability
8. Other channels (config reload, logging, notifications)
9. Environment variables qol-tray injects
10. Footgun index
11. Recipe: add a hotkey-triggered action

---

## 1. Mental model and channel inventory

A plugin is **a binary plus up to three declaration files** at its repo root.
qol-tray does four things with it:

- **Discovers** it from `plugin.toml` (menu, shortcuts, runtime command, daemon, deps).
- **Configures** it: writes `config.json`, which the plugin reads on startup; renders an editor form from `qol-config.toml`.
- **Dispatches actions** to it: a hotkey, a dashboard click, or a launcher entry all funnel to one executor that either talks to the plugin's daemon socket or spawns `the-binary <argv>`.
- **Owns its lifetime**: spawns, tracks the PID, kills it on reload/exit, and arms a host-death watchdog so the process cannot outlive qol-tray.

Source-of-truth crates: `libs/qol-plugin-api` (manifest schema + validation),
`libs/qol-config` (config + runtime schema, config loader), `libs/qol-plugin-daemon`
(the daemon helper a plugin links), `libs/qol-runtime` (state socket client +
watchdog + wire protocol), `libs/qol-gpui` (gpui plugin building blocks), and
`apps/qol-tray/src/{plugins,hotkeys,runtime,logging}` (the host side).

### Channel inventory

| Channel | Direction | Transport | Source of truth |
|---|---|---|---|
| Config delivery | host to plugin | `config.json` on disk + `QOL_TRAY_PLUGIN_ID` env | `qol-config/src/lib.rs` (`load_plugin_config_from_env`) |
| Config form / query / action | host UI to plugin daemon | HTTP to daemon socket | `plugin_config_handlers/`, `qol-config/src/contract/runtime.rs` |
| Config reload | host to plugin | `reload` daemon action (else daemon restart) | `.../plugin_config_handlers/notify.rs` |
| Action dispatch | host to plugin | daemon socket OR runtime spawn | `apps/qol-tray/src/plugins/action_executor/` |
| Platform state | host to plugin | `QOL_TRAY_STATE_SOCKET` UDS, `get_state` / `subscribe` | `libs/qol-runtime/src/client.rs`, `.../protocol.rs` |
| `set_focus` | plugin to host | state socket, fire-and-forget | `runtime/server/socket/requests.rs` |
| Lifeline (watchdog) | plugin and host | state socket, held open until EOF | `libs/qol-runtime/src/watchdog.rs` + `requests.rs` |
| Logging | plugin to host | piped stderr/stdout relay | `apps/qol-tray/src/logging/relay.rs` |
| OS notification | plugin to OS (NOT the tray) | `osascript` / `notify-send` | each plugin (e.g. `plugin-cli-sessions/src/notify.rs`) |

---

## 2. Declaration files

Three independent files, three independent validators, plus one cross-validator
(`qol-config` to `qol-runtime`). Source: `libs/qol-plugin-api/src/manifest/`
(`plugin.toml`) and `libs/qol-config/src/contract/` (`qol-config.toml`,
`qol-runtime.toml`).

### 2.1 `plugin.toml` (required) - discovery, menu, shortcuts, runtime, daemon

Schema structs in `libs/qol-plugin-api/src/manifest/schema.rs`
(`PluginManifest`, `PluginInfo`, `ActionDeclaration`, `MenuConfig`, `MenuItem`,
`ActionType`, `RuntimeConfig`, `DaemonConfig`, `Capabilities`, `Dependencies`,
`ShortcutDeclaration`, `ConfigDeclarations`). Current manifest version is 3
(`mod.rs`); the accepted range 1..=3 is enforced in `validation/manifest_rules.rs`.

Sections:

- `[plugin]` (`PluginInfo`): `name`, `description`, `version` (free string, not
  semver-checked), optional `id`, `author`, `platforms`, `uid`. `id` charset is
  `[A-Za-z0-9_-]`, max 64, no leading `-`. `id` is parse-optional but **required
  at the install boundary** (`require_declared_id`), so a manifest can validate yet
  fail on install. `uid` is an opaque frozen identity (authored once, never changed)
  used as the key for durable plugin state (lock, hotkeys, config files); parse-optional,
  and when absent the host warns and uses a transitional uid equal to `id`. `platforms`
  matching is **exact string** against `std::env::consts::OS` ("linux"/"macos"/"windows");
  `"LINUX"`, `" linux"`, `"linux "` silently match nothing, and `[]` means supported nowhere.
- `[runtime]` (`RuntimeConfig`): `command` (binary basename, same charset as id -
  **no paths, no `.sh`**) and legacy optional
  `actions: { <action-id> = [argv...] }`.
- `[action.<id>]` (`ActionDeclaration`): canonical activation-surface action
  catalog. `label` is required. `kind` defaults to `run` and may be `run`,
  `settings`, or `toggle-config`. `args` supplies runtime argv for `run` and
  `settings` actions; omit it to use `[id]`. `toggle-config` entries require
  `config_key` and are not executable/hotkey-bindable. Dashboard actions,
  hotkey choices, shortcut validation, and runtime argv resolution use this
  catalog first.
- `[menu]` (`MenuConfig`): `label`, optional `icon`, and `items` (may be `[]`).
  `MenuItem` is tagged on `type`: `action {id,label,action,config_key?}`,
  `checkbox {id,label,checked?,action,config_key?}`, `separator`,
  `submenu {id,label,items}` (recurses). `action`/`checkbox` carry both an `id`
  AND an `action` of type `ActionType` (`run` | `settings` | `toggle-config`,
  TOML spelling `"toggle-config"`).
- `[[shortcuts]]` (`ShortcutDeclaration`): `id`, `name`, `enabled` (default true),
  `export_to_launcher` (default true), `action` (default `"open"`). `action` must
  reference an existing executable `[action.<id>]` entry when the catalog is
  present, otherwise an existing executable menu action id.
- `[daemon]` (`DaemonConfig`): `enabled`, `command`, optional `socket` (must be an
  **absolute** path, no `..`). Presence with `enabled=true` flips the plugin to the
  host-owned daemon model (section 5).
- `[capabilities]` (`Capabilities`): `serial`, `gpui` bools; any unknown key is
  captured into `extras` and never rejected (forward-compat).
- `[[dependencies.binaries]]`: `name` (basename charset), `repo`, `pattern` (e.g.
  `"plugin-x-{os}-{arch}"`). One binary keeps store release discovery simple.
- `[config]` (`ConfigDeclarations`): per-field sync **scope** (`core` | `os` |
  `device`), separate from `qol-config.toml`. `"any"` is a legacy alias for `core`.

#### Validation invariants (the part that bites)

`PluginManifest::validate()` (`validation/manifest_rules.rs`) runs in order:
version, identity, menu, runtime, shortcuts, daemon, dependencies.

- **Prefer `[action.<id>]` for every new plugin.** It is the one-stop declaration
  for plugin actions exposed to the dashboard, hotkeys, shortcuts, and runtime
  spawning. `[menu].items` is legacy action metadata plus config-toggle layout;
  `[runtime.actions]` is legacy argv mapping.
- **Menu action id vs `ActionType` are different things.** The `runtime.actions`
  keys, `shortcut.action`, and runtime coverage all key off the menu item `id`,
  never off the `ActionType`. Dozens of items can all have `action = "run"`; that
  does not mean you write `actions = { run = [...] }`. This rule only matters for
  legacy menu-derived actions.
- **`runtime.actions` coverage is all-or-nothing for legacy menu actions.** Declaring the table at all
  means every `type="action"` menu item id must have a mapping, or validation
  fails. Checkbox ids are exempt (checkboxes are not "executable"). Omit the table
  entirely and the default argv for an action becomes `[action_id]`.
- **Catalog actions replace `[runtime.actions]`.** If any `[action.<id>]` entry
  exists, `[runtime.actions]` must be absent. Put every executable action's argv
  in `[action.<id>].args`; non-catalog runtime mappings are rejected so they
  cannot become hidden direct-execution actions.
- **An empty `[runtime.actions]` table is an error** (omit it instead).
- **Duplicate menu action/checkbox ids are rejected** across the whole tree
  (submenus recurse). Submenu container ids are not tracked, so they are not
  deduplicated.
- **`shortcut.action` must reference an executable action id** from the catalog
  when present, otherwise an executable menu action id. Unknown references fail;
  duplicate shortcut ids fail.

Minimal `plugin.toml`:

```toml
[plugin]
id = "plugin-template"
name = "My Plugin"
description = "A qol-tray plugin"
version = "0.1.0"
platforms = ["linux", "macos"]

[runtime]
command = "plugin-template"

[action.run]
label = "Run"
args = ["run"]

[action.settings]
label = "Settings"
kind = "settings"
args = ["settings"]

[menu]
label = "My Plugin"
items = []

[[dependencies.binaries]]
name = "plugin-template"
repo = "qol-tools/plugin-template"
pattern = "plugin-template-{os}-{arch}"
```

### 2.2 `qol-config.toml` (optional) - the settings UI schema

Schema in `libs/qol-config/src/contract/v1.rs` (`ConfigSpecV1`, `SectionSpec`,
`FieldSpec`, `FieldKind`); validation in `.../validation.rs`; normalization
(defaults + override merge) in `.../normalized.rs`. Authoritative prose doc:
`libs/qol-config/docs/v1.md` - but note it is **stale** (missing `color`, `action`,
`list`, `status`, `qr_code` and several attributes); trust the structs.

- Top level: `schema_version` (must be `1`), optional `title`, `description`.
- `[section.<id>]`: `label`, `description`, optional `actions` (section-level
  buttons referencing runtime action names). Order preserved.
- `[field.<id>]`: required `type` (`FieldKind`, snake_case: `boolean`, `string`,
  `number`, `select`, `string_array`, `object_array`, `object_map`, `color`,
  `action`, `list`, `status`, `qr_code`), plus `config_key`, `label`,
  `description`, `placeholder`, `section`, `default`, `show_when`, `align`, `span`.

Per-kind rules (validated, not ignored):

- `number`: `min`/`max`/`step` (`step>0`, `min<=max`); rejected on other kinds.
- `select`: `options` (non-empty) + optional `option_labels`; default/override must
  be an option.
- `object_array`: `[field.<id>.item.fields]`; `object_map`: `key_label` +
  `[field.<id>.entry_fields]`.
- `color`: hex string, optional `alpha`; streamable.
- **`action` / `list` / `status` / `qr_code` hold no stored value**: they must NOT
  have a `default`, and they require a matching `qol-runtime.toml` declaration. Every
  other kind **must** have a `default`.
- `config_key` (dotted, e.g. `"audio.enabled"`) routes the value into a nested JSON
  path in `config.json`; defaults to the field id (so renaming a field id silently
  moves storage unless you pin `config_key`).
- `show_when { field, equals }` conditionally renders a field.

```toml
schema_version = 1
title = "Window Actions"

[section.center]
label = "Center"

[field.center_mode]
type = "select"
section = "center"
default = "fixed"
options = ["fixed", "percent"]
[field.center_mode.option_labels]
fixed = "Fixed Size"
percent = "Relative Size"

[field.center_width_px]
type = "number"
section = "center"
default = 1152
min = 200
[field.center_width_px.show_when]
field = "center_mode"
equals = "fixed"
```

### 2.3 `qol-runtime.toml` (optional) - the runnable surface

Distinct from `plugin.toml`'s `[runtime]` block. Declares the actions/queries/streams
that `qol-config.toml` fields (and section `actions`) bind to. Schema in
`libs/qol-config/src/contract/runtime.rs` (`RuntimeSpec`, `ActionSpec`, `QuerySpec`,
`StreamSpec`); cross-validation in `.../cross_validate.rs`.

- `[action.<name>]`: `description`, optional `confirm`, optional `input` map.
- `[query.<name>]`: `description`, `poll_interval_ms`.
- `[stream.<name>]`: `description`, `throttle_ms` (clamped 16..=1000), optional
  `initial_query`.
- Names must be `[a-z][a-z0-9_]*` (stricter than action ids: no dashes, no
  uppercase, no leading digit) and globally unique across actions/queries/streams.

Cross-file: an `action` field's `action` and a `list`/`status`/`qr_code` field's
`query` must reference a declared runtime entry; a `stream = "..."` attribute is
allowed only on `color`/`number` fields.

---

## 3. Config delivery at runtime

A plugin reads its config on startup:

```rust
let cfg: MyConfig = qol_config::load_plugin_config_from_env(PLUGIN_ID);
```

Source: `libs/qol-config/src/lib.rs` (`load_plugin_config_from_env`,
`plugin_id_from_env`, `load_plugin_config`, `plugin_config_paths`, `config_roots`).

- Identity comes from the `QOL_TRAY_PLUGIN_ID` env var (injected by the host). If
  present-but-invalid the loader **panics** (host bug); if absent it uses the
  passed fallback id (standalone run).
- The file is `{config_root}/plugins/{id}/config.json` - **JSON**, searched across
  an ordered list of roots, first that parses wins.
- A missing or unparseable file **silently falls back to `T::default()`** (parse
  errors are logged, not raised), so a schema/struct mismatch degrades to defaults
  rather than erroring. Define `T: Deserialize + Default`.
- The host writes that file from the editor form (`PUT /api/plugins/{id}/config`),
  merging on write to preserve daemon-owned fields, then signals reload (section 8.1).
- `list`/`status`/`qr_code`/`action` fields are never serialized into `config.json`;
  they are driven live over the daemon socket (sections 4 and 8.1).

---

## 4. Action dispatch (hotkey / dashboard / launcher to effect)

Three entry points converge on one executor:
`action_executor::try_execute_action(plugin_manager, plugin_id, action_id)`
(`apps/qol-tray/src/plugins/action_executor.rs`).

### 4.1 What is bindable, and who owns the key

- The hotkey catalog (`apps/qol-tray/src/hotkeys/catalog.rs`) collects bindable
  actions from `manifest.executable_action_ids()`: executable `[action.<id>]`
  entries when the catalog is present, otherwise legacy executable menu actions
  (recursing into submenus). Checkbox/toggle-config ids are config controls, not
  executable hotkey targets. On the fallback path an uncatalogued binding is
  dropped at plan time (logged); on the kernel-capture path it is installed and
  fails to resolve an action at dispatch.
- Bindings live in `hotkeys.json`, **OS-scoped** in the profile
  (`os/<platform>/hotkeys.json`). Key syntax `MOD+MOD+KEY`, case-insensitive; mods
  `ctrl`/`alt`/`shift`/`super` (+ aliases) plus a non-modifier key (zero keys is
  rejected; the parser does not error on extra keys, last one wins).
- qol-tray **takes the key back from the desktop environment** via kernel-level
  capture (macOS `CGEventTap` at HID, dropping matched KeyDown; Linux evdev
  `EVIOCGRAB` + uinput re-emit). It uses kernel capture OR the `global_hotkey` crate,
  never both: kernel capture is tried first and `global_hotkey` is the fallback
  (Windows always falls back). The fallback co-registers with the OS and can collide;
  conflicts surface to the doctor (`hotkey_shadows`). Which path is active is logged
  at startup.

### 4.2 Resolution and the daemon-vs-runtime decision

`action_executor/resolution.rs` builds a `ResolvedAction`:

- `daemon_socket` is `Some` only if `[daemon].enabled` and `socket` are set.
- If an action catalog is present, `action_id` must be an executable catalog id
  and `args` come from `[action.<id>].args`, defaulting to `[id]`.
- Without an action catalog, legacy `args` come from
  `runtime.actions[action_id]`, or `[action_id]` if no `actions` table, or error
  (`MissingActionMapping`) if the table exists without this id.
- The runtime command must stay inside the plugin dir (no absolute path, no `..`).

`action_executor/execution.rs` then chooses:

- **daemon socket present** -> `dispatch_daemon_action(socket, action_id)`. On
  `Handled` it is done; on `Fallback`/`Unavailable` it falls back to a runtime spawn
  **if `runtime_fallback_allowed`**; an `Error(msg)` short-circuits to
  `ActionRejected` before any fallback check (it is never a runtime fallback).
- **no daemon socket** -> spawn `command <args>` (the runtime path). Spawns are
  single-flight deduped via `RUNNING_ACTIONS` except the no-daemon `open` action
  (treated as an activation request, so a second `open` can focus an existing window).

`DaemonActionDispatch` (`apps/qol-tray/src/plugins/action_transport/`) is the
transport: a newline-terminated `DaemonRequest{action}` JSON over the Unix socket,
10s IO timeout, returning `Handled{payload?}` / `Fallback` / `Error` / `Unavailable`.
The same transport carries config **queries**.

### 4.3 Entry points and gotchas

- Hotkey (kernel capture closure or fallback listener), dashboard click
  (`POST /api/plugins/{id}/actions/{action}`), and launcher (`qol-tray exec shortcut
  <id>` to the daemon HTTP) all reach `try_execute_action`.
- The **native tray menu does NOT list plugin actions** - it shows "Open Dashboard",
  the Mode (dev/prod) toggle, an Update item when one is available, and Quit. Plugin
  actions live in the web dashboard.
- `ActionType::Run` and `ActionType::Settings` are executable and dispatch the
  same way today. `ActionType::ToggleConfig` is not executable/hotkey-bindable;
  checkbox `checked` is the manifest-declared initial value, not live config.
- Launcher export (`export_to_launcher`) is a third concept, orthogonal to hotkeys
  and to shortcuts: it writes a macOS `.app` / Linux `.desktop` that runs
  `qol-tray exec shortcut <id>` (Windows no-op, honoring "leave host as found").

### 4.4 Worked trace: `plugin-cli-sessions` `open`

`open` has no `[daemon]`, so it resolves to a runtime spawn of `cli-sessions open`
with `args=["open"]`. `main.rs` first tries `send_action(&CONFIG, "open", false)` to
its own socket; if an instance is running, that instance receives `Command::Open`
and shows the panel and the second process exits; if not, this process binds the
socket and runs the gpui panel itself (self-daemonizing). The spawn carries
`QOL_TRAY_PLUGIN_ID` and `QOL_TRAY_STATE_SOCKET` (which arms the watchdog).

---

## 5. Process lifecycle and ownership

### 5.1 The daemon helper crate (`libs/qol-plugin-daemon`)

A plugin links this to receive actions while running (`src/daemon.rs`):

- `DaemonConfig { default_socket_name, use_tmpdir_env, support_replace_existing }`.
- Socket path: `QOL_TRAY_DAEMON_SOCKET` env if set, else `$TMPDIR/<name>` when
  `use_tmpdir_env` and `TMPDIR` is set, otherwise `/tmp/<name>`.
- Wire protocol: one JSON line `{"action":"..."}` (bare `open` / `action:open` text
  also parse). Response `{"status":"handled"|"fallback"|"error",...}`.
- `start_listener(config, tx, parse_command)` binds the socket, **calls
  `qol_runtime::spawn_host_death_watchdog()`** (the single line that prevents
  orphans), and accepts connections, mapping each action via your `parse_command`
  into a `ReadResult` (`Command(C)` forwards to your mpsc channel; `Handled`,
  `HandledWithData`, `Fallback`, `Error`, `Ignore`).
- `send_action` / `send_kill` / `send_ping` are the client helpers a plugin's CLI
  front-end uses to forward to an already-running instance of itself.
- Replace-existing: on `AddrInUse` it pings the owner; takes over only if
  `support_replace_existing` and `QOL_TRAY_DAEMON_REPLACE_EXISTING` is truthy.
- The crate is **Unix-only by `compile_error!`** because the watchdog is Unix-only;
  shipping a daemon plugin on another OS would silently leak.

### 5.2 Two ownership models

- **Host-owned daemon** (`[daemon] enabled=true`): qol-tray autostarts it
  (`plugins/daemon_lifecycle/`), supplies `QOL_TRAY_DAEMON_SOCKET`, and dispatches
  actions over the socket. Examples: alt-tab, lights, launcher, pointz.
- **Self-daemonizing** (no `[daemon]`, only `[runtime]`): spawned fresh per action;
  the plugin itself becomes a daemon on first invocation (via `send_action` to its
  own socket, binding if that fails). qol-tray never sets `QOL_TRAY_DAEMON_SOCKET`.
  Example: cli-sessions.

### 5.3 Spawn, track, kill

- Daemon spawn (`daemon_lifecycle/spawn.rs`) sets env (below), `setsid()` so the
  daemon leads its own process group (teardown signals the negative PID to kill the
  whole group), and registers the PID in three places: in-memory `Child`, an
  async-signal-safe atomic table (for the SIGINT handler), and on-disk
  `<id>.pid` files.
- Stop paths: graceful `terminate_daemon` (SIGTERM to `-pid`, 2s, escalate to
  SIGKILL); `kill_all_plugin_processes` for short-lived action procs;
  `stop_all_plugins` on reload/exit (kill actions -> stop daemons -> clear map ->
  clear PID files -> `kill_orphan_daemons`); an orphan sweep that also scans live
  processes whose exe lives under a managed root (protecting the host binaries).
- The **Recompile** button (`features/plugin_store/server/dev_services/recompile/`)
  rebuilds qol-tray, runs `stop_all_plugins`, verifies no plugin leaks, then
  `exec`s the fresh binary in place (same PID). It does not rebuild plugin binaries
  or touch config/sync state. See `qol-tray:qol-tray-dev-recompile`.

### 5.4 The host-death watchdog (orphan prevention)

This is mission non-negotiable #3 (host left exactly as found, no orphaned daemons).
Source: `libs/qol-runtime/src/watchdog.rs`; host side
`apps/qol-tray/src/runtime/server/socket/requests.rs`.

- `spawn_host_death_watchdog()` **does nothing unless `QOL_TRAY_STATE_SOCKET` is in
  the environment.** When present, it spawns a thread that opens a `lifeline` to the
  state socket and holds it; no events flow - when qol-tray dies the socket EOFs and
  the plugin `exit`s. If the lifeline can't connect, it checks `getppid()==1`
  (reparented to init) and exits if orphaned, else retries.
- A plugin therefore **leaks if and only if the watchdog is not armed**: either
  `QOL_TRAY_STATE_SOCKET` was not injected (the action-spawn path used to omit it -
  this was a real leak) or the plugin hand-rolled a daemon without calling
  `spawn_host_death_watchdog`. Using `qol_plugin_daemon::start_listener` arms it for
  you.
- The host audits this at startup: any enabled declared daemon that never arms a
  lifeline gets a loud error (`plugins/manager/autostart.rs`).

---

## 6. The platform-state socket

A host-authoritative Unix socket (`QOL_TRAY_STATE_SOCKET`, default
`/tmp/qol-tray-state.sock`, const in `apps/qol-tray/src/paths.rs`). Client API in
`libs/qol-runtime/src/client.rs` (`PlatformStateClient`, `Subscription`); protocol
in `libs/qol-runtime/src/protocol.rs`; server in `apps/qol-tray/src/runtime/server/`.

Requests (newline JSON, `cmd`-tagged): `get_state` (monitors etc.),
`set_focus {monitor_idx}` (fire-and-forget), `subscribe {plugin_id, events}` (held-open
stream of `RuntimeEvent`; kinds include `active_monitor_changed`, `cursor_moved`,
`focus_changed`, `monitors_changed`, `window_list_changed`, `launcher_apps_synced`),
`lifeline {plugin_id}`
(the watchdog), `armed_lifelines` (host-internal audit).

**Direction invariant**: this is host-to-plugin for data. Plugins read state and
consume events; the only plugin-to-host writes are `set_focus`, opening a
`subscribe`/`lifeline` stream, and the `armed_lifelines` query. **A plugin cannot
push status or notifications to the tray over this socket** - the event publisher is
qol-tray-internal. Unknown verbs get no response and the connection is dropped.

gpui plugins consume this via `qol_gpui::MonitorTracker` (placement) and
`qol_gpui::event_router` (reacting to monitor/focus changes).

---

## 7. The `gpui` capability

`capabilities.gpui = true` is a thin flag; the host's only behavioral difference is
broadcasting ghost-debug runtime-config reloads to gpui plugins
(`features/plugin_store/server/dev_state_handlers.rs`). The real contract is the
`libs/qol-gpui` crate that such plugins depend on:

- `keepalive::open_keepalive` - a hidden 1x1 window so the app process stays alive
  with no visible windows.
- `popup_window` - `configure_popup_window`, `show_window_by_title`,
  `hide_window_by_title`, `reposition_window_by_title`, `reason_scope` (RAII guard
  recording why a show/hide happened, visible in probes), `set_ghost_debug`.
- `monitor::MonitorTracker` - `snapshot_monitor`, `snapshot_monitor_focus_first`,
  `all_monitors`; wraps `PlatformStateClient`.
- `platform` - `set_accessory_policy` (macOS: no dock icon / no focus theft),
  `ghost_window_kind`, `ghost_window_decorations`, `should_poll_focus`.
- `command_loop::spawn_command_loop` + `LoopFlow {Continue, Stop}` - the standard
  daemon-to-UI command bridge (an action arrives over the socket, becomes a
  `Command`, runs on the app executor).
- `window::open_window_with_focus` - create + eager OS focus steal; `ghost.rs` -
  one warm ghost per monitor reconciliation; `event_router::spawn_runtime_event_router`.

Worked consumer: `plugins/plugin-cli-sessions/src/ui/run.rs` uses keepalive,
accessory policy, MonitorTracker placement, ghost decorations/kind,
`open_window_with_focus`, and `spawn_command_loop`. Platform divergence is sharp
(macOS `Normal` windows + opacity vs Linux/X11 `PopUp` + unmap; ghosts must be
`is_movable`; Muffin drops cross-monitor moves) - see `qol-langs:gpui-conventions`
and the `libs/qol-gpui` rules, and verify Linux ghost behavior via `qol trace`, not
a live session.

---

## 8. Other channels

### 8.1 Config reload (host to plugin)

After the editor saves, the host dispatches a `reload` action over the daemon
socket (`.../plugin_config_handlers/notify.rs`). If the daemon does not return
`Handled` for `reload`, the host **restarts the daemon**. Contract: a daemon should
treat `reload` as "re-read `config.json`".

### 8.2 Logging (plugin to host)

The host pipes plugin stderr (and stdout in dev). In prod, lines matching `ERROR`,
`error`, `FATAL`, `panic`, or `PANIC` (so any line containing "error" is captured -
deliberately aggressive) are forwarded into the host's structured error capture
tagged `plugin.{id}.daemon_stderr` (`apps/qol-tray/src/logging/relay.rs`), so plugin
errors surface in the host. `RUST_LOG` is injected per profile. `qol_runtime::probe!` is a
debug-only per-process trace to `/tmp/qol-altmon.log` (not collected by the host);
see `qol-project:qol-trace`.

### 8.3 OS notifications (plugin to OS, NOT the tray)

There is no plugin-to-tray notification/status channel. A plugin that notifies the
user shells out directly (`osascript display notification` on macOS, `notify-send`
on Linux), as in `plugins/plugin-cli-sessions/src/notify.rs`.

### 8.4 Adding a new channel

`qol-project:qol-arch-channels` is the canonical decision guide for picking or
adding a host-plugin channel; reuse the infra in `libs/qol-runtime` rather than
inventing a socket.

---

## 9. Environment variables qol-tray injects

Set when spawning daemon and/or runtime processes (`daemon_lifecycle/spawn.rs`,
`action_executor/execution.rs`):

| Var | When | Meaning |
|---|---|---|
| `QOL_TRAY_PLUGIN_ID` | every spawn | the plugin's id; config loader and watchdog read it |
| `QOL_TRAY_PLUGIN_DIR` | daemon spawn | the plugin's install dir |
| `QOL_TRAY_STATE_SOCKET` | every spawn | the platform-state socket path; **arms the host-death watchdog** |
| `QOL_TRAY_DAEMON_SOCKET` | declared-daemon spawn / fallback | the daemon socket the plugin should bind/use |
| `QOL_TRAY_DAEMON_REPLACE_EXISTING` | daemon spawn | `1` lets the plugin take over an existing socket |
| `RUST_LOG` | daemon spawn | `debug` in dev, `warn` in prod; runtime/action spawns inherit the host value |

---

## 10. Footgun index

- Config is JSON on disk (`config.json`); the editor schema is TOML
  (`qol-config.toml`). Two different files.
- A bad/missing `config.json` silently yields `T::default()`, not an error.
- New plugins should use `[action.<id>]` as the one-stop executable action
  catalog. Dashboard actions, hotkeys, shortcuts, and runtime argv resolution all
  use it.
- `[runtime.actions]` is legacy-only. If an action catalog exists, the table is an
  error even for non-catalog ids; put argv in `[action.<id>].args`.
- For legacy menu-derived actions, `runtime.actions`, `shortcut.action`, and
  runtime coverage key off the menu item `id`, not the `ActionType`. Coverage is
  all-or-nothing once the table exists.
- `[runtime.actions]` present but empty is an error; legacy plugins can omit it to
  get `[action_id]` default argv.
- `platforms` is exact-string matched; capitalization/whitespace makes a plugin
  unsupported everywhere.
- A plugin leaks unless the watchdog is armed, which requires `QOL_TRAY_STATE_SOCKET`
  in env AND `spawn_host_death_watchdog` being called (free if you use
  `qol_plugin_daemon::start_listener`).
- The state socket is host-to-plugin; plugins cannot push notifications/status to
  the tray. Shell out for OS notifications.
- `ActionType::Run`/`Settings` are executable; `ToggleConfig` is not a direct
  dispatch target.
- The native tray menu does not surface plugin actions; the dashboard does.
- `config_key` defaults to the field id - renaming an id moves storage silently.
- `action`/`list`/`status`/`qr_code` config fields must omit `default` and need a
  `qol-runtime.toml` declaration.
- `qol-config/docs/v1.md` is stale; trust the structs.
- `qol-plugin-daemon` is Unix-only (`compile_error!`); do not add a non-Unix
  `platforms` entry to a daemon plugin.

---

## 11. Recipe: add a hotkey-triggered action to a plugin

The exact "basics" that get re-learned. To add an action `foo` that a user can bind
a global hotkey to:

1. **Manifest** (`plugin.toml`) - declare the executable action once:
   - `[action.foo]` with `label = "Do Foo"` and `args = ["foo"]` (omit `args` only
     when the runtime argv should be `["foo"]`).
   - `[[shortcuts]]` add `{ id = "foo", name = "My Plugin: Foo", action = "foo" }`.
   - Keep `[runtime] command = "plugin-binary"`; do not add `[runtime.actions]`
     when the catalog exists.
2. **Receive it.** In your daemon's `parse_command`, map `"foo"` to a `Command`
   variant; handle that variant in your command loop.
3. **Forward it (self-daemonizing plugins).** In `main`, add a `Some("foo")` arm
   that calls `send_action(&CONFIG, "foo", false)` so a hotkey spawn reaches the
   running instance. (Host-owned daemons skip this - the action arrives over the
   socket directly.)
4. The user binds a key to the "My Plugin: Foo" shortcut in qol-tray; qol-tray owns
   the key. Worked example: the `next` action in `plugins/plugin-cli-sessions/`.
