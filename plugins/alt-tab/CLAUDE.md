# plugin-alt-tab

Better Alt+Tab for qol-tray (Linux + macOS). GPUI window list with live previews, replacing the native Alt+Tab via qol-tray's hotkey system.

## Contract

`plugin.toml` is the source of truth for the runtime command, actions, daemon socket, menu, and platforms. `validate_plugin_contract()` parses it via qol-tray's `PluginManifest` and must stay green - do not duplicate its contents here.

## Non-negotiables

1. **Live query per show. No polling, no long-lived MRU cache.** Visible windows are queried fresh on every open so z-order matches what the OS thinks is frontmost. Any observer/store/stacking-watcher that "keeps state warm" leaks stale windows and misses focus changes.
2. **Strategy pattern, zero `#[cfg(target_os)]` in business logic.** Platform differences live behind a trait in per-OS modules; cfg gates exist only in the re-export layer. An unsupported OS returns a typed `Err`, never `compile_error!` or `unimplemented!()`.
3. **AX calls can stall.** Always keep: a messaging timeout, parallel AX prefetch so one slow PID caps `max` not `sum`, and a short-TTL slow-PID-aware cache so repeated opens skip known-slow PIDs.
4. **The show path never captures; previews are warmed in the background.** Opening is a pure, instant reveal of the cached previews - never trigger a screenshot on show and never block the show on capture. A macOS window screenshot is ~30-90ms and WindowServer-serialized, so capturing-on-open *is* the lag. Keep previews current instead by usage, not by opening: a hidden, idle-gated warmer re-shoots only the active window (idx 0) on a ~250ms timer while the picker is hidden and there has been recent HID input; every other window is captured the instant focus leaves it (`FocusChanged` -> capture the just-defocused window). A window's content only changes while it is frontmost, so its last-focused capture stays accurate until it is refocused - which is why re-shooting backgrounded windows is wasted work. The cache is the source of truth at show time. The warmer and the focus-leave fill use separate in-flight guards so neither blocks the other. The icon cache is per-app and long-lived.
5. **Daemon-backed picker.** Cold GPUI startup is too slow for Alt+Tab, so the daemon stays resident: the picker window is created once and reused across opens, never destroyed between opens on macOS. A hidden keepalive PopUp stops GPUI from quitting when the picker is dismissed.
6. **Config and window cache reload per show**, so settings and MRU are current without a restart.
7. **Debug logs under `#[cfg(debug_assertions)]`, prefixed `[alt-tab/...]`** so qol-tray's filters work. Never leak into release.
8. **Data-driven dispatch over N-way switches.** Rule kinds, action handlers, and platform bindings go through `{ key, handler }` tables.
9. **Every `Arc<RenderImage>` cache routes through the registry.** Inserts via `REGISTRY.retain`, removals via `REGISTRY.release`, so `App::drop_image` fires exactly once per `ImageId`. `MetalAtlas::remove` double-decrements on a double remove; a view that owns images must drain them in `Context::on_release`.
10. **Focus-out is passive; it NEVER activates the selection.** Activation is owned solely by explicit user intent (Enter, card click, alt-release via `on_modifiers_changed` or the alt-poll fallback). Routing activation through focus-out lets a click-outside hijack the selected window.
11. **Foregrounding the picked window is authoritative, not `NSRunningApplication.activate`** (inert on macOS 14+: returns true, does nothing). `_SLPSSetFrontProcessWithOptions` alone is silently ignored when an actively-front app holds front, so foreground via the target app's `kAXFrontmost` attribute plus the SkyLight `set_front` path, then re-assert both on a short generation-guarded loop until the target is frontmost. The picker teardown deactivates the daemon and the WindowServer restores the prior app, so a one-shot activation loses a timing race; the re-assert wins it. See the `macos-window-activation` skill.

## Ghost popup: active monitor is qol-runtime's single source of truth

The picker stays alive between invocations drawn at `alpha=0` + `ignoresMouseEvents`, and recenters as the user moves between monitors so the next show is a pure alpha toggle on the correct monitor.

- Show and ghost paths both resolve placement through `PopupPlacement::from_tracker(tracker)` at use time. The plugin holds zero monitor state: never cache a `LastActiveMonitor`, never decode event payloads for routing, never re-implement crossing/reclaim logic. Runtime owns that decision. If the picker lands on the wrong monitor, the fix is in qol-runtime, not here.
- Do NOT use `from_tracker_focus_first` for this picker - it forces focus to win over more-recent cursor activity, breaking "follow the user". Focus-first is only for popups that must strictly track the focused window.
- `FocusChanged` / `CursorMoved` / `MonitorsChanged` are recompute triggers, not carriers of the active-monitor decision. Ghost recenter runs the same placement computation the show path uses - pixel-exact match is the contract.
- macOS y-flip uses the first entry of `NSScreen::screens(mtm)` (the menu-bar/anchor screen, fixed for the session). NEVER `NSScreen::mainScreen()` - it follows the focused window and drifts on multi-monitor.
- gpui `window_bounds()` is screen-local, not global gpui coords; reconcile against `NSWindow::frame()` if in doubt.
- Always-on-top via `NSPopUpMenuWindowLevel`, set once at boot.

## Build and dev

- No Makefile-as-build-system; use `cargo` directly.
- Never leave an `alt-tab` binary in the plugin root - it shadows `target/debug/`. qol-tray resolves binaries plugin-root, then `target/debug/`, then `target/release/`.
- A resident daemon keeps serving the old binary until restarted. After a rebuild, restart the daemon (qol-tray Recompile, or kill + relaunch) or your change is invisible. Type-check passing does not mean the feature works.

## Verification (before reporting work complete)

```
cargo fmt --all --check
cargo clippy --all-targets --all-features --keep-going -- -D warnings
cargo build
cargo test
```

If you touched `plugin.toml`, arg parsing, or the daemon protocol: restart the daemon and verify behavior in the picker by hand.

## Testing

1. Property tests for ordering invariants (window-order merge, MRU stabilization), parsing, and path-safety.
2. Parameterized tables for exact-output contracts (window enumeration to `WindowInfo`, AX filter decisions).
3. No smoke tests - `assert!(x.is_ok())` must fail on a plausible regression or it stays out.
4. Every bug starts with a failing test.

## Systematic debugging

Recurring failure mode: stale state persists across opens because a cache was designed optimistically. For any "works first time, then gets weird" report, audit each cache for the correct lifetime - preview (background-warmed, pruned to the live window set per show), icon (per-app), MRU/window-order (per-query), AX results (short TTL, slow-PID aware), and the daemon binary itself (rebuilt vs running).
