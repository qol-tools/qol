# plugin-alt-tab

Better Alt+Tab for qol-tray (Linux + macOS). GPUI window list with live previews. Replaces the native Alt+Tab via qol-tray's hotkey system.

## Contract (test-enforced)

- Runtime command: `alt-tab`
- Runtime actions:
  - `open -> ["--show"]`
  - `open-reverse -> ["--show-reverse"]`
  - `settings -> ["--settings"]`
- Daemon: **enabled**. Socket: `/tmp/qol-alt-tab.sock`. Bare `alt-tab` starts the daemon.
- Menu: `Alt Tab (Open/Next)` (`run`), `Alt Tab (Previous)` (`run`, id `open-reverse`), separator, `Settings` (`settings`).
- Platforms: `linux`, `macos`.
- `validate_plugin_contract()` parses `plugin.toml` via qol-tray's `PluginManifest` and must stay green.

## Architecture

| File / Dir | Purpose |
|---|---|
| `src/main.rs` | Entry: loads config, dispatches `--show` / `--show-reverse` / `--kill` to daemon, or starts daemon via `picker::run::run_app()`. |
| `src/daemon.rs` | Unix socket daemon (show/kill/ping) via `qol_plugin_api::daemon`. |
| `src/config.rs` | TOML config via `qol_config::load_plugin_config(["plugin-alt-tab", "alt-tab"])`. |
| `src/picker/run.rs` | Daemon event loop, caches, `dispatch_show()`. |
| `src/picker/mod.rs` | Picker orchestration: `open_picker()` with cycle / reuse / create paths. |
| `src/picker/create.rs` | Window creation, `pre_create_offscreen()` for instant open. |
| `src/picker/reuse.rs` | `compute_layout`, `resize_if_needed`, `reposition_if_needed`. Source of truth for placement. |
| `src/picker/gather.rs` | Window gathering from cache or live discovery. |
| `src/picker/monitor_listener.rs` | Subscribes to `FocusChanged + CursorMoved + MonitorsChanged` purely as recompute triggers. Holds no state. On every signal: `PopupPlacement::from_tracker(tracker)` -> `reuse::compute_layout` -> reposition + resize the ghost. On focus/monitors signals also re-discovers MRU windows. |
| `src/picker/platform/macos.rs` | macOS NSWindow control: `pre_create`, `set_ghost_opacity`, `reposition_picker_window`, level (`NSPopUpMenuWindowLevel`). |
| `src/app/mod.rs` | `AltTabApp` GPUI component, focus/blur handling, centralized `dismiss()`, one-shot tap-too-fast Alt-release fallback. |
| `src/app/render.rs` | UI: grid layout, transparency, card styling, `on_modifiers_changed` -> dismiss when Alt drops in `HoldToSwitch`. |
| `src/discovery/platform/macos/` | macOS window enumeration via CoreGraphics (z-order = MRU). |
| `src/discovery/platform/linux.rs` | Linux enumeration via x11rb `_NET_CLIENT_LIST_STACKING`. |
| `src/capture/platform/macos.rs` | Preview capture via `CGWindowListCreateImage`. |
| `src/capture/platform/linux.rs` | Preview capture via x11rb Composite `GetImage`. |
| `src/actions/platform/` | Per-platform window actions (activate, close, quit, minimize). |
| `ui/` | Settings HTML/JS/CSS served by qol-tray at `/plugins/plugin-alt-tab/`. |

## Daemon architecture

GPUI's GPU init is too slow for Alt+Tab on cold start, so the daemon stays resident with a pre-created picker window.

1. qol-tray launches the daemon at boot (`daemon.enabled = true`).
2. Daemon binds `/tmp/qol-alt-tab.sock`, initializes GPUI, pre-creates the picker window offscreen.
3. Each `--show` writes `"show"` to the socket (<5ms).
4. Daemon refreshes window cache (fresh MRU), reloads config, reuses the existing picker window.

**Invariants:**
- Picker window is **created once at boot, reused across opens**. Never destroy between opens on macOS.
- A hidden `qol_plugin_api::keepalive` PopUp keeps GPUI alive when the picker is dismissed.
- Window cache is refreshed synchronously per show (`refresh_cache_for_show`) so MRU is always current.
- Config is reloaded per show so settings changes take effect without restart.
- Transparency changes are applied via `window.set_background_appearance()` + `disable_window_shadow()` on reuse.

## Non-negotiables

1. **Live query per show. No polling. No long-lived MRU cache.** `Platform.visible_windows()` is called fresh on every open so z-order matches what the OS thinks is frontmost. No `AXObserver` / `WindowStore` / stacking-order watcher that "keeps state warm" - that leaked stale windows and missed focus changes.
2. **Strategy pattern, zero `#[cfg(target_os)]` in business logic.** Platform differences live in `src/<feature>/<os>/mod.rs` behind a trait. cfg gates exist only in the `mod.rs` re-export layer. Unsupported OS returns a typed `Err`, never `compile_error!` or `unimplemented!()`.
3. **AX calls can stall.** Preserve all of: (a) 1s messaging timeout via `init_messaging_timeout`, (b) parallel AX prefetch in `discover_live_windows` so one slow PID caps `max`, not `sum`, (c) short-TTL process-wide cache in `ax::ax_windows` so repeated opens within ~2s skip known-slow PIDs.
4. **Preview cache is flicker-buffer, not source of truth.** Re-capture every non-minimized window per show via `capture_previews_cg`. `HashMap::extend` overwrites; do NOT filter out already-cached ids. Icon cache is different (per-app, long-lived OK).
5. **Daemon-backed picker.** Cold GPUI startup is too slow for Alt+Tab. Daemon pre-creates the window offscreen; `keepalive` PopUp prevents GPUI quit.
6. **Debug logs under `#[cfg(debug_assertions)]`.** Prefix every log line with `[alt-tab/...]` (e.g. `[alt-tab/timing]`, `[alt-tab/ax] SLOW`, `[alt-tab/ghost]`) so qol-tray suppress/mute filters work. Never leak into release.
7. **Data-driven dispatch over N-way switches.** Rule kinds, action handlers, platform bindings all go through `{ key, handler }` tables.

## Ghost-popup (macOS)

The picker window stays alive between invocations with `alpha=0 + ignoresMouseEvents=true`. The ghost recenters when the user moves between monitors so the next show is a pure alpha toggle on the correct monitor.

### Active monitor is qol-runtime's coalesced signal

**Single source of truth.** Both show and ghost paths call `PopupPlacement::from_tracker(&tracker)` at use time. This resolves through `MonitorTracker::snapshot_monitor()` -> `PlatformState::active_monitor()` -> the `active_monitor_idx` qol-runtime produces in `qol-tray/src/runtime/state.rs:pick_active_monitor`. The plugin holds zero monitor state, never caches a `LastActiveMonitor`, never decodes event payloads to make routing decisions. If the picker ever lands on the wrong monitor, the fix is in qol-runtime, not here.

**Do NOT** use `PopupPlacement::from_tracker_focus_first` for the alt-tab picker. That accessor forces focus to win even when cursor activity is more recent, which breaks the "follow the user" UX. Only use focus-first for popups that should strictly track the focused window.

### Sub-signals that feed `state.active_monitor()`

Coalescer in `state.rs:pick_active_monitor`: most-recent `Stamped.at` of `state.cursor` vs `state.focus`, ties favor focus.

Cursor side (writes `state.cursor: Stamped` in `state.rs:update_cursor`):

| # | Source | Path | Gate |
|---|---|---|---|
| C1 | Cursor poll, threshold-changed | `runtime/channels/cursor.rs:CursorChannel::poll` (16ms) -> `runtime/server/poll/sample.rs:apply_cursor_update` | \|dx\|>1.0 OR \|dy\|>1.0 vs last polled pos |
| C2 | Cursor crossed to a different monitor | `update_cursor` branch `monitor_change` | `moved=true && !same_monitor` |
| C3 | Cursor moved within same monitor, focus newer (cursor catches up) | `update_cursor` branch `reclaim_from_focus` | `moved=true && same_monitor && focus_is_newer` |
| C4 | Stationary cursor, focus moved elsewhere (cursor reclaims its own monitor) | `update_cursor` branch `still_here_reclaim` | `moved=false && same_monitor && focus_is_newer && focus_elsewhere` |

Focus side (writes `state.focus: Stamped` in `state.rs:update_focus`):

| # | Source | Path | Gate |
|---|---|---|---|
| F1 | Focus poll, focused-window bounds changed | `runtime/channels/focus.rs:FocusChannel::poll` (100ms) -> `runtime/server/poll/sample.rs:apply_focus_update` | `fresh != prev && fresh.is_some() && sample.focus_changed`; needs platform `poll_focused_window()` |
| F2 | Plugin UDS `SET_FOCUS idx` (used by alt-tab `push_focus_hint` when activating a window) | `runtime/server/socket/requests.rs:apply_focus` | `idx` must resolve in the current monitors list |

Plugins consume the coalesced result. `MonitorsChanged` only updates the `monitors` Vec; it does not stamp cursor or focus. The subscription events `FocusChanged` / `CursorMoved` / `MonitorsChanged` are recompute triggers, not carriers of the active-monitor decision.

### Key rules

- Ghost recenter goes through the same `picker::reuse::compute_layout` the show path uses. Pixel-exact match is the contract.
- Active-monitor selection: ALWAYS `PopupPlacement::from_tracker(tracker)`. Never cache a local `LastActiveMonitor` or equivalent. Never re-implement crossing/reclaim logic in the plugin - runtime owns C1..C4 and F1..F2.
- For gpui_y -> ns_y conversion use `NSScreen::screens(mtm).iter().next()` (the menu-bar / anchor screen, fixed across a session). **Never use `NSScreen::mainScreen()`** - it moves with the focused window.
- gpui's `window.window_bounds()` returns **screen-local** coords (relative to whichever NSScreen the window is on), not global gpui coords. Confirm with `NSWindow::frame()` ns_origin if reconciling.
- Always-on-top: `NSWindow::setLevel(NSPopUpMenuWindowLevel)` in `pre_create` once at boot.
- `CursorMoved` is already threshold-gated upstream in `CursorChannel::poll`. Do not add a second filter in the plugin.

## Config

Loaded via `qol_config::load_plugin_config(["plugin-alt-tab", "alt-tab"])` from `qol-config.toml`.

| Field | Effect |
|---|---|
| `display.max_columns` | Max grid columns. |
| `display.transparent_background` | Transparent window background (requires shadow disable on macOS). |
| `display.card_background_color` / `card_background_opacity` | Card styling. |
| `display.show_minimized` | Include minimized windows. |
| `display.show_hotkey_hints` | Header bar with keybindings. |
| `display.show_debug_overlay` | Debug header. |
| `display.ghost_opacity` | Ghost picker alpha (0.0 = invisible; >0 used for debug). |
| `action_mode` | `hold_to_switch` (release Alt to confirm) or `sticky` (press Enter). |
| `reset_selection_on_open` | Reset selection to index 0 each open. |
| `open_behavior` | `cycle_once` or `show_list`. |
| `label.*` | Label font size, show app name, show window title. |

## Build and dev workflow

- No Makefile-as-build-system. Use `cargo build` directly.
- Do NOT leave an `alt-tab` binary in the plugin root - it shadows `target/debug/alt-tab`.
- qol-tray resolves binaries in order: plugin root -> `target/debug/` -> `target/release/`.
- If the daemon is pre-fix your change is invisible. `pgrep -fl alt-tab`, check binary mtime vs your build, kill stale process before re-test.

## Verification (before reporting work complete)

```
cargo fmt --all --check
cargo clippy --all-targets --all-features --keep-going -- -D warnings
cargo build
cargo test
```

If you touched `plugin.toml`, `src/main.rs` arg parsing, or the daemon protocol: `alt-tab --kill`, relaunch via qol-tray (or `target/debug/alt-tab`), and verify behavior in the picker. Type-check passes != feature works.

## Testing preferences

1. **Property tests (proptest)** for ordering invariants (stable-window-order merge, MRU stabilization), parsing, path-safety.
2. **Parameterized tables** for exact-output contracts (CGWindow -> WindowInfo, AX filter decisions).
3. **No smoke tests.** `assert!(x.is_ok())` must fail on a plausible regression or it stays out.
4. **Every bug starts with a failing test.** Stale-preview, stale-MRU, AX stall: when you fix, add the test.

## Known issues / TODO

1. **Linux preview accuracy.** X11 `GetImage` captures the off-screen buffer; minimized or occluded windows may return stale/blank pixels.
2. **Wayland.** Not implemented. Linux path is x11rb only.

## Systematic debugging

Recurring failure mode: "stale state persists across opens because a cache was designed optimistically". For any "works first time, then gets weird" report, check:

- Preview cache (per-show, NOT per-lifetime)
- Icon cache (per-app, long-lived OK)
- MRU / stable-window-order (per-query, plus a global keyed cache in `window_enum.rs`)
- AX result cache (short TTL, slow-PID aware)
- Daemon binary vs freshly-built binary (kill/respawn)

## Settings UI

Served by qol-tray at `/plugins/plugin-alt-tab/`. Endpoints:

- `GET /api/plugins/plugin-alt-tab/config` - load
- `PUT /api/plugins/plugin-alt-tab/config` - save (JSON body)

The `--settings` runtime action opens this URL in the default browser.
