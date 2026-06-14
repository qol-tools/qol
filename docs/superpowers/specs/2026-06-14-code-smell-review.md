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

The highest raw risk is `plugin-os-themes`' Linux cursor backend because it combines unsafe Xlib/XFixes code, cursor allocation/freeing, event casting, pointer warping, diagnostics, and refresh policy inside one large file that can run near a 16ms loop.

The lowest-risk first implementation slice is a shared Linux monitor parser in `qol-platform`. It is duplicated today, has observable correctness drift around negative `xrandr` offsets, and can be tested without a live display or unsafe code. The macOS FFI consolidation is still important, but should start later with only CF/CG ownership primitives, not AX, SkyLight, or raw ObjC message dispatch.

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

This is the highest-risk area by blast radius: unsafe pointer/resource handling and refresh timing live together. Even if most refresh ticks return early, the changed/notification paths can do synchronous X calls, pixel copies, sleeps, pointer warps, and stderr logging from the cursor runtime.

Suggested direction:

Split the backend by concern before changing behavior: X display/event RAII, owned cursor image data, refresh state machine, tree application, pointer probing, and diagnostics. Gate diagnostics behind an explicit debug flag or environment variable. Add timing probes around refresh paths before optimizing.

### F2. Linux window and desktop primitives are fragmented across plugins and shared libraries

Evidence:

- [plugin-alt-tab actions linux.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/actions/platform/linux.rs:61) uses `x11rb` for window activation/minimize/close behavior.
- [plugin-alt-tab discovery linux.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/linux.rs:60) opens an X11 connection and interns atoms for discovery.
- [plugin-window-actions system.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/system.rs:17) shells out to `xdotool`.
- [plugin-window-actions system.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/system.rs:47) shells out to `xprop` for window stacks.
- [plugin-window-actions system.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/system.rs:94) shells out to `wmctrl`.
- [qol-gpui popup linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/popup_window/platform/linux.rs:61) repeatedly opens X11 connections and interns atoms in popup operations.
- [qol-plugin-daemon focus linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-plugin-daemon/src/focus/platform/linux.rs:13) and [qol-gpui platform linux.rs](/Users/kaho/repos/private/qol-monorepo/libs/qol-gpui/src/platform/linux.rs:68) contain duplicated process-focus checks.

Why this matters:

Window activation, active window lookup, `_NET_WM_PID`, stack order, monitor movement, and focus ownership are the same platform domain. Today some callers use direct X11 APIs, some shell out to desktop tools, and some duplicate helper code. That creates inconsistent behavior, dependency drift, and avoidable process-spawn overhead.

Suggested direction:

Grow `qol-platform` into the shared Linux desktop primitive layer: X11 session, atom cache, window id/pid lookup, active window, stack order, activation/minimize/close, monitor geometry, and focus ownership. Migrate shell-out code after the parser/focus helpers are shared.

### F3. `xrandr` monitor parsing is duplicated and inconsistent

Evidence:

- [qol-tray desktop_state linux.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/desktop_state/platform/linux.rs:230) parses `xrandr --current` for physical monitors.
- [qol-tray trace linux.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/runtime/server/trace/platform/linux.rs:56) has a separate trace/debug parser.
- [plugin-window-actions monitor_move.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/monitor_move.rs:141) parses `xrandr` output for monitor movement.
- [plugin-screen-recorder linux.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-screen-recorder/src/platform/linux.rs:241) parses `xrandr` output for capture bounds.

Why this matters:

The parsers do not all handle the same geometry grammar. Some split only on `+`, while others search for either `+` or `-` offsets. That means a monitor layout such as `1920x1080-1920+0` can be parsed differently depending on the feature.

Suggested direction:

Create `qol_platform::linux::xrandr` with a small monitor model and a pure parser for connected monitor lines. Include table tests for primary flag, disconnected lines, positive offsets, negative offsets, and mode rows. Replace the four local parsers with the shared parser.

### F4. macOS CF/CG/AX/SkyLight FFI is duplicated across host and plugins

Evidence:

- [qol-tray macos.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/desktop_state/platform/macos.rs:6) defines local `CGRect`, `CGPoint`, and `CGSize`.
- [plugin-alt-tab ffi.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/ffi.rs:3) defines CF/CG type aliases, geometry structs, CoreGraphics/CoreFoundation bindings, CF string conversion, dictionary helpers, and CG window-list constants.
- [plugin-window-actions objc.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-window-actions/src/platform/macos/objc.rs:5) defines another local geometry model and CF/AX/CG/ObjC bindings.
- [plugin-alt-tab ax.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/ax.rs:11) defines local AX bindings and AX window helpers.
- [plugin-alt-tab spaces.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/discovery/macos/spaces.rs:31) dynamically loads private SkyLight symbols.
- [plugin-alt-tab actions macos.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-alt-tab/src/actions/platform/macos.rs:325) separately dynamically loads SkyLight activation symbols.

Why this matters:

The duplication is not just naming. It repeats unsafe ownership choices around CF Create/Copy/Get results, manual `CFRelease`, borrowed dictionary/array values, private framework loading, and raw Objective-C ABI calls. That increases the chance of leaks, over-release, null-pointer misuse, or subtly different permission/error handling.

Suggested direction:

Start later with `qol_platform::macos::{cf,cg}` only:

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

Migrate lights to `qol-plugin-daemon` with a local action/query mapper, then keep only light-specific config validation and legacy migration locally. Fix the unchecked config path handling even if the full daemon migration is deferred.

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

1. Add shared `xrandr` monitor parsing to `qol-platform` and migrate the four duplicated parsers.
2. Lift Linux focus ownership checks into `qol-platform` and make `qol-plugin-daemon` and `qol-gpui` call it.
3. Start a Linux X11 desktop/session abstraction and migrate one shell-out path from `plugin-window-actions`.
4. Split and instrument `plugin-os-themes` cursor backend before attempting behavior changes.
5. Add `qol_platform::macos::{cf,cg}` primitives, then migrate `plugin-alt-tab`'s `ffi.rs` as a compatibility layer.
6. Migrate `plugin-lights` daemon handling to `qol-plugin-daemon`.

## External review questions

- Is `qol-platform` the right owner for monitor geometry, Linux X11 window operations, Linux focus ownership, and macOS CF/CG primitives, or should any of these live in `qol-plugin-api` instead?
- Should the first implementation slice be the low-risk shared `xrandr` parser, or should the cursor hot path be tackled first despite higher risk?
- Are any duplicated platform behaviors intentionally different because of product semantics?
- Which macOS private API use should remain plugin-local even after CF/CG primitives are shared?
- Are there known user-visible bugs tied to negative monitor offsets, cursor refresh latency, or window activation that should change the priority order?

## Non-goals for the first pass

- No wholesale platform rewrite.
- No SkyLight abstraction.
- No AX behavior changes.
- No cursor behavior changes before measurement.
- No cleanup of large modules unless it directly supports one of the platform consolidation slices.
