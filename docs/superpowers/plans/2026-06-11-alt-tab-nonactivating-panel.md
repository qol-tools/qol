# Plan: non-activating picker panel (remove the focus race instead of winning it)

## Status (2026-06-12)

- Phase 1 landed: `PICKER_APP_ACTIVE` probes on all three show paths plus dismiss.
- Phase 2 attempted and reverted: with `WindowKind::PopUp` the app stayed inactive
  but the panel never became key, so gpui key routing (arrows/Enter) died. R1
  fired. AltTab avoids this because its keys come from a global event tap, not
  key-window routing. Next attempt: aim `force_front`'s SLPS make-key records at
  our own panel before retrying the flip; fallback is a picker-visible-scoped
  event tap.
- Probe semantics correction: `cx.activate(true)` lands asynchronously, so
  `at=show` reads `active=false` even on baseline at the call site. The
  discriminating assertion is `at=dismiss`: `active=true` today, must read
  `active=false` after the flip. The show sample is deferred one frame.
- Running now (target side, independent of the flip): `ns_activate_app` is out of
  the activation hot path (back to the `if !forced` fallback) and added to the
  reassert failure path. Hypothesis: the ~500ms WindowServer stalls are the
  app-level activation handshake with busy targets; SLPS needs no target
  cooperation. Verdict via `CG_SLOW`/`ACTIVATE_SETTLED` over the next days.

## Why

The reassert ladder, settle checks, and `force_front` re-assertion exist for one reason:
showing the picker activates the daemon app, so dismissing it makes WindowServer run an
asynchronous "restore the previous app" transaction that a one-shot target activation can
lose. Everything timing-shaped in the activation path is compensation for that race.

AltTab (lwouis/alt-tab-macos) proves the race is removable:

- `src/switcher/main-window/TilesPanel.swift`: the switcher is an `NSPanel` with
  `styleMask: .nonactivatingPanel`, `canBecomeKey: true`, shown via `makeKeyAndOrderFront`.
  The panel takes keyboard (key window) while the previous app stays the active app.
- `src/switcher/state/Window.swift focus()`: one shot, no retries:
  `_SLPSSetFrontProcessWithOptions` + two synthetic SkyLight event records + AX raise.
  No settle predicate, no ladder, because dismissing their panel triggers no restoration.

Measured cost of our current design: recurring ~500ms WindowServer stalls right after
alt-release (`CG_SLOW tag=settle_check ms=498/501`, the 18:48 probe-arg gap incident),
consistent with WindowServer serializing around the app activate/deactivate churn we cause
on every open/dismiss. Plus the daemon's own post-activation `CGWindowListCopyWindowInfo`
traffic (settle checks) lands exactly inside that window.

## Verified ground truth (recon 2026-06-11, gpui 0.2.2 from crates.io)

- Picker kind on macOS is `WindowKind::Normal` (`libs/qol-gpui/src/platform/macos.rs:28`).
  Linux already uses `PopUp`. `Normal` dates from the file's creation; PopUp was never
  tried and reverted on macOS (git log -L confirms).
- gpui maps `WindowKind::PopUp` to `GPUIPanel`, an `NSPanel` subclass, and ORs in
  `NSWindowStyleMaskNonactivatingPanel` (`gpui-0.2.2 mac/window.rs:622-625`). Both window
  classes override `canBecomeKeyWindow -> YES` (`window.rs:287-293`). PopUp windows also
  get an `NSTrackingArea` so hover/mouse-moved works while the app is inactive
  (`window.rs:790+`).
- `MacWindow::activate()` is only an async `makeKeyAndOrderFront:` (`window.rs:1212`).
  It does not activate the app. On a PopUp panel it is exactly AltTab's show call.
- App activation today comes solely from three `cx.activate(true)` calls
  (= `NSApp activateIgnoringOtherApps:`): `picker/mod.rs:111` (cycle/show-existing),
  `picker/mod.rs:272` (apply path), `picker/create.rs:261` (finalize).
- Dismiss never deactivates explicitly; the restoration race is WindowServer reacting to
  the active app's window disappearing.
- Already focus-independent: alt-release (ALT_POLL 30ms global modifier poll; every
  release in recent traces came from it) and Tab-cycling (each press is a socket
  `CMD_RECV cmd=open` from qol-tray's hotkey system).
- Focus-dependent surface to preserve: arrows, Enter, Esc, W/Q/R, `on_modifiers_changed`
  fast path, focus-out dismiss.
- No picker logic reads `is_window_active`.
- External witness: qol-tray's `FOCUS_WIN` probe logs `AXFocusedApplication`; today it
  reports the picker app during shows. After the change it must never.
- `force_front` already implements AltTab's focus mechanism (SLPS + synthetic make-key
  events); the delta is purely the panel activation policy.

## Phases

Each phase ends with: `cargo fmt --check`, `clippy -D warnings`, `cargo test`,
`cargo build`, daemon Recompile, manual checklist, trace review. One commit per phase
when told to commit.

### Phase 1: observability baseline (tiny)

Add a debug-only probe `PICKER_APP_ACTIVE active={bool} at={show|dismiss}` sampling
`NSApp.isActive` at show finalize and at dismiss.

Exit: done. Baseline traces read `active=false at=show` (activation is async at
the call site; sample now deferred one frame) and `active=true at=dismiss`. The
dismiss line is the assertion anchor for "the app no longer activates".

### Phase 2: the flip (the actual change, two edits)

1. Make the plugin's `picker_window_kind()` return `WindowKind::PopUp` on macOS via the
   plugin's per-OS `imp` layer. Do NOT change `qol_gpui::ghost_window_kind()`: the
   launcher shares it and keeps its current behavior (zero cfg in business logic,
   per non-negotiable #2).
2. Delete the three `cx.activate(true)` calls. Key acquisition is the existing
   `window.activate_window()` (= `makeKeyAndOrderFront`) in reuse/create paths plus
   `window.focus(&handle)` for gpui focus routing. Confirm the cycle path
   (`mod.rs:111`) still acquires key when cycling starts from ghost state; if not, add
   `activate_window()` there (window-level, still non-activating).

Keep the reassert ladder and all activation machinery unchanged this phase: isolate the
variable. Ghosts are recreated at boot and on `MonitorsChanged`, so a daemon restart
rolls the kind out everywhere; no migration.

Manual checklist:
- show/dismiss on both monitors; arrows/Enter/Esc/W/Q/R; card click; click-outside
  dismiss; alt-release commit; rapid double-tap cycling; minimized target; cross-space
  target; fullscreen-Space target; busy-kitty target; Teams/Firefox slow targets;
  launcher unaffected; over-fullscreen show (panel level survives, `configure_picker_window`
  re-asserts `NSPopUpMenuWindowLevel` by title).

Trace assertions:
- `PICKER_APP_ACTIVE active=false` on every dismiss (today: true; the show line
  reads false in both worlds at the call site).
- `FOCUS_WIN` never reports the picker app again.
- `SHOW_TIMING`/`SHOW_PAINTED` unchanged within noise.
- `ACTIVATE_SETTLED` settles on the first ladder step every time.
- `CG_SLOW tag=settle_check` absent in normal use.

Risk register:
- R1 arrows/Enter dead (gpui key routing to a key panel of an inactive app). Unlikely:
  AppKit routes key events to the key window and AltTab relies on exactly this; gpui
  dispatches from window-level `keyDown`. If hit: inspect gpui first-responder handling
  on `becomeKey`; worst case abort (rollback is two edits).
- R2 focus-out/blur-guard ordering changes (no app deactivation on dismiss). The
  `focus_out_decision` table is pure and tested; verify click-outside and
  monitor-switch dismiss by hand.
- R3 `on_modifiers_changed` not delivered in some state: ALT_POLL is already the
  authoritative release path; MODS_UP becomes best-effort.
- R4 NSPanel default behaviors (becomesKeyOnlyIfNeeded, worksWhenModal) differing:
  gpui sets none of these; verify card click still focuses and Esc works.

Rollback: revert two edits. No persisted state involved.

### Phase 3: activation machinery on a diet (after 2-3 days of clean Phase-2 traces)

- 3a: replace the ladder with a single probe-only verification check (~100ms,
  generation-guarded) that logs settled/unsettled and never reasserts. Also revert the
  interim `[60, ...]` experiment ladder (moot by then).
- 3b: if 3a shows 100% settled over a few days: delete `spawn_reassert_driver` usage,
  the settle predicate hot path (`target_effectively_front` polling), the stuck/settled
  stack probes, and `ns_activate_app` from the hot path. AltTab parity: unminimize +
  AX raise + SLPS + synthetic key records, once. Keep `ACTIVATE_TIMING` and one
  post-hoc `ACTIVATE_KEY_SAMPLE`.

Net effect: an alt-release performs one SLPS call, two synthetic events, one AX raise,
and zero post-activation `CGWindowListCopyWindowInfo` - removing our own contribution to
the WindowServer pressure that the stalls correlate with.

### Phase 4: encode the standard (before applying elsewhere)

- plugin CLAUDE.md: rewrite non-negotiable #11; add: picker is a non-activating key
  panel (`WindowKind::PopUp`); never call `cx.activate` on show; never reintroduce app
  activation. Include the why (race removal, AltTab precedent, stall evidence).
- `macos-window-activation` skill: document key-window vs active-application, the
  AltTab citations, and the one-shot focus recipe.
- qol-gpui CLAUDE.md: note `ghost_window_kind` stays `Normal` on macOS for the launcher
  until the launcher does its own Phase 2 (likely wants the same fix: it is Spotlight-shaped).

## Non-goals

Live-query-per-show stays. Ghost-per-monitor visibility rule stays. Capture pipeline,
MRU ordering fixes, cross-space list bug, AX element cache, stale-binary/socket-takeover
hygiene: all separate, already-tracked work.

## Success criteria

- Zero `CG_SLOW` / ~500ms stalls correlated with alt-tab over a week of use.
- Host `FOCUS SUCCESS` latencies unchanged or better; zero MISDIRECTED FOCUS.
- Picker interaction indistinguishable to the user (except no longer stealing app focus).
- Net-negative code delta after Phase 3.
