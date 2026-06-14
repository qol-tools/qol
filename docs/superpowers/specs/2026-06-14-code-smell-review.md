# qol monorepo code smell review

## Purpose

This is a static review brief for external code review. It records concrete code smells found in the qol monorepo, with file evidence and proposed follow-up slices. It is not a runtime profiling report, and it does not claim the listed hot paths are currently user-visible regressions without measurement.

The review focus was:

- Repeated platform/runtime code that should probably be lifted into shared crates.
- Expensive-looking hot paths that deserve measurement or simplification.
- Hard-to-reason modules with too many responsibilities.
- Unsafe or FFI-heavy code whose risk is amplified by duplication.

## Executive summary

The strongest architectural smell is that platform primitives are scattered across host, shared libraries, and plugins. The same low-level concepts appear repeatedly: macOS CF/CG/AX bindings, Linux X11 window operations, monitor geometry parsing, focus ownership checks, and daemon socket handling.

The densest unsafe surface is `plugin-os-themes`' Linux cursor backend, which combines unsafe Xlib/XFixes code, cursor allocation/freeing, event casting, pointer warping, diagnostics, and refresh policy inside one large file that can tick near a 16ms loop while a cursor is scaled. It is contained, though - Linux-only, one optional plugin, a separate process, and free when idle - so it is a maintainability and safety-surface concern, not the repo's top blast-radius risk.

The lowest-risk first implementation slice is a shared `xrandr` monitor parser. Treat it as a bug fix, not a tidy-up: the four parsers disagree on signed offsets, so a monitor placed at a negative virtual coordinate (left of, or above, the primary) is silently dropped by at least one consumer. The pure parser and monitor model belong in `qol-runtime`, beside the existing `MonitorBounds` type, while the `xrandr` command execution stays in each Linux caller. It can be tested without a live display or unsafe code. The macOS FFI consolidation is still important, but should start later with only CF/CG ownership primitives, not AX, SkyLight, or raw ObjC message dispatch, and in a dedicated leaf crate rather than the capability crate.

## Findings

### F1. `plugin-os-themes` X11 cursor backend mixes unsafe FFI, diagnostics, and a hot refresh loop

Evidence:

- [runtime.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/runtime.rs:14) defines a 16ms tick interval.
- [runtime.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/runtime.rs:57) calls `session.refresh()` while the cursor is scaled.
- [x11.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/x11.rs:27) keeps X display state, cursor handles, pointer state, refresh state, XFixes event state, and ignored serials in one session struct.
- [x11.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/x11.rs:577) drains raw X events and casts `XEvent` into XFixes cursor notifications.
- [x11.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/x11.rs:985) samples live cursor images with synchronous X calls and sleeps.
- [x11.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/x11.rs:1060) includes recompute/probe behavior that can warp the pointer.
- [x11.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-os-themes/src/cursor/platform/linux/x11.rs:1525) contains extensive unguarded diagnostic logging helpers near the same subsystem.

Why this matters:

This concentrates unsafe pointer/resource handling and refresh timing in one place. The loop is free-when-idle (it blocks on cursor-move events and only ticks at 16ms while a cursor is actively scaled), so this is not the repo's top blast-radius risk - it is Linux-only, in one optional plugin, in a separate process. But while scaled, the changed/notification paths do synchronous X calls, pixel copies, sleeps, pointer warps, and unguarded stderr logging, and the unsafe surface is large, so it is a real maintainability and safety-surface concern.

Suggested direction:

The cheap, immediate win is gating the unguarded `eprintln!` diagnostics (currently always compiled and printed in release) behind a debug flag or environment variable. The larger refactor is medium priority: split the backend by concern before changing behavior - X display/event RAII, owned cursor image data, refresh state machine, tree application, pointer probing, and diagnostics. Add timing probes around refresh paths before optimizing.

### F2. Linux window and desktop primitives are fragmented across plugins and shared libraries

Evidence:

- [plugin-alt-tab actions linux.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/actions/platform/linux.rs:61) uses `x11rb` for window activation/minimize/close behavior.
- [plugin-alt-tab discovery linux.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/linux.rs:60) opens an X11 connection and interns atoms for discovery.
- [plugin-window-actions system.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/system.rs:17) shells out to `xdotool`.
- [plugin-window-actions system.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/system.rs:47) shells out to `xprop` for window stacks.
- [plugin-window-actions system.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/system.rs:94) shells out to `wmctrl`.
- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:61) repeatedly opens X11 connections and interns atoms in popup operations.
- [qol-plugin-daemon focus linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-plugin-daemon/src/focus/platform/linux.rs:13) and [qol-gpui platform linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/platform/linux.rs:68) contain near-identical, copy-pasted process-focus checks (`has_process_focus`, `owns_window`, `window_pid`, `get_pid_atom`) that have drifted into different poll gates.

Why this matters:

Window activation, active window lookup, `_NET_WM_PID`, stack order, monitor movement, and focus ownership are the same platform domain. Today some callers use direct X11 APIs, some shell out to desktop tools, and some duplicate helper code.

The focus duplication is not cosmetic - the two copies have drifted into a real bug. `qol-plugin-daemon` gates polling on `qol_platform::linux_display_backend()`, which classifies the session as Wayland whenever `WAYLAND_DISPLAY` is set, so it skips the X11 focus poll under XWayland. `qol-gpui` reimplements the gate inline as `DISPLAY is set || XDG_SESSION_TYPE == x11`, so it does poll under XWayland. Same intent, opposite behavior on the same session.

The shell-out callers (`xdotool`/`xprop`/`wmctrl`) are a deliberate choice - `plugin-window-actions` carries no `x11rb` dependency at all - and they run on explicit user actions, not a hot path.

Suggested direction:

Merge the two focus checks into one shared focus module (the daemon already owns `focus/`; do not bloat the dependency-free capability crate `qol-platform` with `x11rb`). When merging, pick the gate deliberately: prefer "can I open an X11 connection?" over "is the session nominally Wayland?", since the `_NET_WM_PID` ownership walk works for X11 and XWayland windows and already falls back to `true` when no connection is available (the native-Wayland case still has no X11 focus to read). The remaining primitives (X11 session, atom cache, window/pid lookup, stack order, activation/minimize/close) can share a Linux X11 layer later; the per-OS strategy files stay, calling the shared primitives. Migrate the shell-out paths only as a possible later consolidation.

### F3. `xrandr` monitor parsing is duplicated and inconsistent

Evidence:

- [qol-tray desktop_state linux.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/desktop_state/platform/linux.rs:230) parses `xrandr --current` for physical monitors.
- [qol-tray trace linux.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/runtime/server/trace/platform/linux.rs:56) has a separate trace/debug parser.
- [plugin-window-actions monitor_move.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/monitor_move.rs:141) parses `xrandr` output for monitor movement.
- [plugin-screen-recorder linux.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-screen-recorder/src/platform/linux.rs:241) parses `xrandr` output for capture bounds.

Why this matters:

This is a correctness bug, not just duplication. The four parsers handle different subsets of signed offsets, and none handles all of them:

- `qol-tray` desktop_state and the trace parser select the geometry token with `contains('+')` and split offsets with `split_once('+')`. They parse xrandr's literal negative spelling (`+-1920`) correctly but drop any bare-minus spelling, and never even select a both-negative token (no `+` to match).
- `plugin-window-actions` scans offsets with `find(['+','-'])` and selects via `find_map`, so it parses bare-minus layouts (including both-negative) but drops xrandr's literal `+-1920` spelling (the inner scan stops on the `-` right after the first `+`, leaving `"+"` to parse).
- `plugin-screen-recorder` combines the worst of both: a `contains('+')` token filter (drops both-negative at selection) plus a `find(['+','-'])` offset scan (drops the literal `+-1920` spelling).

Net: a monitor placed at a negative virtual coordinate (left of, or above, the primary) is silently dropped by at least one consumer, and which one depends on how the running X server spells the offset. The exact spelling xrandr emits (`+-1920` vs bare `-1920`) should be confirmed against a real negative-offset layout; the shared parser must accept both.

Suggested direction:

Put a pure monitor parser and model in `qol-runtime`, beside the existing `MonitorBounds` type (not `qol-platform`, which is a dependency-free capability crate and would need a new edge); keep the `xrandr --current` command execution in each Linux caller. The model must carry the superset the callers need (connector, primary, x, y, width, height; the trace consumer is release-gated). Table-test the full sign matrix in both spellings: `+0+0`, `+1920+0`, and `-1920+0`, `+0-1080`, `-1920-1080` plus their literal twins `+-1920+0`, `+0+-1080`, `+-1920+-1080`, alongside primary flag, disconnected lines, and mode rows. Replace the four local parsers with the shared one.

### F4. macOS CF/CG/AX/SkyLight FFI is duplicated across host and plugins

Evidence:

- [qol-tray macos.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/desktop_state/platform/macos.rs:6) defines local `CGRect`, `CGPoint`, and `CGSize`.
- [plugin-alt-tab ffi.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/ffi.rs:3) defines CF/CG type aliases, geometry structs, CoreGraphics/CoreFoundation bindings, CF string conversion, dictionary helpers, and CG window-list constants.
- [plugin-window-actions objc.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/macos/objc.rs:5) defines another local geometry model and CF/AX/CG/ObjC bindings.
- [plugin-alt-tab ax.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/ax.rs:11) defines local AX bindings and AX window helpers.
- [plugin-alt-tab spaces.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/spaces.rs:31) dynamically loads private SkyLight symbols.
- [plugin-alt-tab actions macos.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/actions/platform/macos.rs:325) separately dynamically loads SkyLight activation symbols.

Why this matters:

The duplication is not just naming. It repeats unsafe ownership choices around CF Create/Copy/Get results, manual `CFRelease`, borrowed dictionary/array values, private framework loading, and raw Objective-C ABI calls. That increases the chance of leaks, over-release, null-pointer misuse, or subtly different permission/error handling. The spread is wider than the evidence list: the `CGRect`/`CGPoint`/`CGSize` triple is also redefined in `qol-app-icon` and `plugin-pointz`, and `CfGuard` exists in two copies (over `*mut` vs `*const`), so three different CF-ownership disciplines (mut-guard, const-guard, no-guard) are in play.

Suggested direction:

Start later with a dedicated leaf crate (e.g. `qol-macos`) exposing `cf`/`cg` only, or reuse the `core-graphics`/`core-foundation` types already in the dependency graph via gpui rather than authoring fresh `repr(C)` structs. Do not bloat the capability crate `qol-platform`. Scope the first slice to:

- Owned guard for CF Create/Copy/Retain results.
- Borrowed wrappers for dictionary/array values that must not be released.
- CF string creation/conversion.
- CF number/bool/dict helpers.
- CG window-list constants and wrappers.

Do not lift AX, SkyLight, or raw ObjC `msgSend` in the first slice. AX has timeout behavior that must be preserved, SkyLight is private API, and raw ObjC dispatch should probably be replaced with `objc2` rather than promoted as shared API.

### F5. `plugin-lights` reimplements daemon socket protocol and config loading

Evidence:

- [plugin-lights daemon mod.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/daemon/mod.rs:23) defines local daemon request/response types.
- [plugin-lights daemon mod.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/daemon/mod.rs:54) binds and handles its own listener loop.
- [qol-plugin-daemon daemon.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-plugin-daemon/src/daemon.rs:92) already provides shared listener behavior.
- [plugin-pointz daemon.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-pointz/src/daemon.rs:1) shows the shared helper pattern used by another plugin.
- [plugin-lights config store.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/config/store.rs:8) implements local config load/default/migration logic.
- [plugin-lights config store.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/config/store.rs:47) uses `path.parent().unwrap()` and ignores `create_dir_all` errors.

Why this matters:

Most plugins can benefit from common daemon lifecycle, request parsing, and response shape. A plugin-local protocol risks drift from `qol_runtime::protocol::DaemonResponse`. The config store may need light-specific validation and migration, but filesystem defaults and path handling should not be brittle.

Suggested direction:

`plugin-lights` intentionally avoids the shared daemon dependency (`qol-plugin-daemon`); its response JSON already appears intended to match the canonical `DaemonResponse`, and round-trips cleanly today. So the immediate, low-cost win is a round-trip/shape test pinning the plugin's response shape to `qol_runtime::protocol::DaemonResponse`, which de-risks drift without forcing the dependency migration. Defer the full migration, and treat it as reversing a deliberate isolation choice, not fixing an oversight. The config path handling (`path.parent().unwrap()` plus the discarded `create_dir_all` result) is brittle but not a live bug - the `.unwrap()` is unreachable given how the path is built, and the ignored error is still surfaced by the following `fs::write` - so tidy it opportunistically rather than as a priority.

### F6. `qol-gpui` popup X11 operations repeat connection and atom setup

Evidence:

- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:61) opens X11 and interns atoms for reposition.
- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:80) repeats setup for setting bounds.
- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:138) repeats setup for hiding with opacity.
- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:182) repeats setup for showing.
- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:223) repeats setup for popup configuration.

Why this matters:

Popup operations are window lifecycle paths. Reopening X11 and reinterning common atoms on each operation adds overhead and keeps X11 error handling scattered.

Suggested direction:

Introduce a cached popup X11 session or reuse the broader `qol-platform` X11 session once it exists.

### F7. Several large modules combine unrelated responsibilities

Evidence:

- [qol-cli dev_console.rs](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/dev_console.rs:426) contains TUI state, session loops, process spawning, dashboard rendering, doctor execution, trace display, and shutdown logic in one large file.
- [plugin-alt-tab window_enum.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/window_enum.rs:28) combines stable ordering, known-window tracking, AX cache integration, CG filtering, minimized-window recovery, cross-space state, and debug output.
- [plugin-alt-tab picker mod.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/picker/mod.rs:43) combines picker open/reuse/create/cycle behavior, placement, state, and tests.

Why this matters:

These are not all urgent runtime risks, but they slow future work and make regressions easier during feature changes. The risk is highest when large modules also include unsafe/platform behavior; lower when the size is mostly tests or dev tooling.

Suggested direction:

Do not split only by line count. Extract around stable responsibilities: polling/session machinery, rendering spans, process/doctor control, window ordering policy, AX enrichment, and debug output.

## Recommended implementation order

1. **Fix `xrandr` negative-offset parsing (bug, not refactor).** Put the pure parser and monitor model in `qol-runtime` beside `MonitorBounds`; keep command execution in the Linux callers. Ship it with the full sign matrix as tests (`-1920+0`, `+0-1080`, `-1920-1080` and their literal `+-` twins). Replacing the four duplicated parsers falls out of the fix.
2. **Fix the focus-poll gate drift (bug).** Merge the two copied process-focus checks into one shared focus module and pick a single gate that prefers "can I connect to X11?" over "is the session nominally Wayland?", so XWayland is handled consistently. Do not park the heavy X11 primitives in the capability crate `qol-platform`.
3. **Linux X11 session abstraction - possible later consolidation, low urgency.** The `xdotool`/`xprop`/`wmctrl` shell-outs in `plugin-window-actions` are a deliberate no-`x11rb` choice on a cold path; folding them into a shared X11 layer adds the dependency they avoid. Frame as optional, not cleanup.
4. **`plugin-os-themes` cursor: ship the logging gate first.** Gating the unguarded release-mode `eprintln!` in the 16ms-adjacent cursor subsystem is the cheap, immediate win. The split-by-concern refactor is real but medium priority and can follow.
5. **macOS CF/CG primitives in a leaf crate (`qol-macos`) or reuse existing typed geometry.** Do not author fresh `repr(C)` geometry in `qol-platform`; prefer the `core-graphics` types already in the dependency graph. Keep AX timeout/prefetch/cache, SkyLight, and raw ObjC dispatch plugin-local.
6. **`plugin-lights`: test first, migrate later.** Add a round-trip/shape test pinning the plugin's response JSON to the canonical `DaemonResponse`; defer the full `qol-plugin-daemon` migration, which would reverse the plugin's deliberate avoidance of that dependency.

## External review questions

- Ownership (partly resolved): `qol-platform` is a dependency-free capability/detection crate, so the heavy primitives should not land there. Resolved direction - monitor-geometry parsing into `qol-runtime` (beside `MonitorBounds`); focus ownership into a shared focus module (the daemon already owns `focus/`); X11 window operations and macOS CF/CG into the existing `qol-gpui`/`qol-plugin-daemon` homes or a dedicated `qol-macos`/`qol-x11` leaf. Open for reviewers: is a new `qol-x11` leaf worth it versus extending `qol-plugin-daemon`?
- Should the first implementation slice be the low-risk shared `xrandr` parser, or should the cursor hot path be tackled first despite higher risk?
- Are any duplicated platform behaviors intentionally different because of product semantics?
- Which macOS private API use should remain plugin-local even after CF/CG primitives are shared?
- Two bugs are now confirmed in code and have reordered the priorities above: negative monitor offsets are silently dropped by at least one `xrandr` parser, and the XWayland focus-poll gate has drifted between two copies. Still open: any user-visible cursor-refresh-latency or window-activation issue that should be promoted similarly?

## Non-goals for the first pass

- No wholesale platform rewrite.
- No SkyLight abstraction.
- No AX behavior changes.
- No cursor behavior changes before measurement.
- No cleanup of large modules unless it directly supports one of the platform consolidation slices.
