# Hotkey input path: root cause of the keystroke bursts and the way out

Date: 2026-08-26. Measured on the Linux host (Cinnamon, X11) with a uinput probe
keyboard, a passive reader on the probe node, a reader on the tray's virtual
keyboard, and `xinput test-xi2 --root` for what the X server actually received.
Harness: `2026-08-26-hotkey-input-path-harness/` next to this note.

## Symptom

Right after `qol dev` starts, typing freezes for seconds and then everything
typed arrives at once ("spam"). Sometimes a hotkey does nothing for three
seconds first.

## What was measured

Dispatch stall (before `a38e7bf77`): F12 bound to an action whose daemon cannot
come up, 12 keys injected 150 ms apart on the same keyboard. All 12 emerged
3023 ms later, 11 of them inside one millisecond (`kernel_emit_spread_ms=1`
against a nominal 1650). After the fix: spread 1651 ms, worst gap 151 ms, one
key per millisecond, nothing dropped.

Tray restart (`recompile-self`, twice, 600 keys at 150 ms): the tray path is
dark for 3.0 to 3.2 s (exec at ~7.0 s, new grab at ~9.9 s). During that window
the keys go straight to X from the physical device (18 to 19 keys, `src` = the
probe's own X id), hotkeys are inert, and exactly one key is lost: the one the
old process had read but not yet re-emitted when it exec'd. X received 599 of
600 presses and 599 releases; no stuck keys, no burst.

Virtual keyboard rebuild: hot-plugging a keyboard-capable device whose key set
adds a code the merged set lacks (`KEY_MACRO1` here; F13..F24 are already on the
Logitech receiver) makes `merge_capabilities` destroy and recreate the uinput
device. X re-added it fast enough to keep all 120 keys at 150 ms spacing (and
re-used id 17, so `xinput list` hides the swap); a consumer polling every 20 ms
missed two keys. Keys typed faster than the consumer's re-open, and keys held
across the swap, are at risk.

stderr backpressure: shrinking the tray's stderr pipe to 4 KiB and freezing
`qol dev` (its reader) for 4.5 s did not stall the reader thread, because the
`evdev: hotkey phase` line is filtered by the dev core-log controls before it
reaches the pipe. The reader still logs synchronously
(`evdev_backend.rs`, `process_event`); this path is unproven, not disproven.

Environment leak: the terminal this session runs in was opened by the launcher
daemon and carries `QOL_TRAY_DAEMON_LISTENER_FD=32`, `QOL_TRAY_HTTP_TOKEN`,
`QOL_TRAY_DAEMON_REPLACE_EXISTING=1`, `QOL_TRAY_PLUGIN_ID=plugin-launcher`.
`qol dev` inherited them, the tray inherited them, and every daemon the tray
did not pre-bind a socket for inherited them and adopted fd 32 as its
listener: 384 `listener failed: Bad file descriptor` lines in one `qol dev`
session (plugin-controllers 354, qol-voice 30) and 370
`Failed to start window-actions daemon listener`. Running the window-actions
binary by hand with that variable exits 1; without it, it serves.

## Root cause chain

1. `evdev_backend.rs` ran `on_fire` (plugin dispatch, up to the 3 s daemon
   ready wait from `d25885189`) inline on the reader thread that holds the
   keyboard under `EVIOCGRAB`. Present since the backend landed in
   `52be60290` (2026-05-02). Fixed structurally in `a38e7bf77`: readers hold
   a `Sender<CaptureEvent>` and nothing else.
2. Dev-linked daemons start cold by design (`.qol-tray-dev-autostart` opt-in),
   so the first hotkey after `qol dev` pays the spawn plus ready wait.
3. The leaked listener-fd variable made some daemons unable to start at all,
   so that wait always ran to its 3 s timeout, on every press. Same class as
   the settings host regression fixed in `c1183fd07`.

Links 1 and 3 are fixed; link 2 is a deliberate dev-mode trade-off and is now
harmless to typing because of link 1's fix.

## Fixes landed

- `a38e7bf77` dispatch channel: the reader thread cannot call plugin code.
- `c1183fd07` settings host no longer inherits the tray's listener fd.
- This change: qol-tray drops every handoff variable from its own environment
  at startup (`main.rs`), `qol-plugin-daemon` refuses an inherited fd that is
  not a listening socket (`SO_ACCEPTCONN`) and binds its own path with a
  warning, and the launcher strips the handoff variables from every app it
  starts (`qol_conventions::scrub_daemon_handoff_env`).

## Still open

- Restart gap: 3 s with hotkeys inert and one key lost. Handing the grabbed fds
  across `exec` would turn the gap into a delayed burst (nothing forwards
  between exec and the successor's readers), which is worse than fail-open.
- Virtual keyboard rebuild on capability growth (`merge_capabilities`).
  Cheapest fix: build the virtual device once with every key code, never
  rebuild.
- The launcher passes `QOL_TRAY_HTTP_TOKEN` and `QOL_TRAY_STATE_SOCKET` to
  launched apps. Not a keyboard issue; worth a separate decision.
- Reader thread still logs synchronously to stderr on every hotkey match.

## Proposal: take the tray out of the keyboard's critical path

Invariant to guarantee: every physical key event reaches the desktop exactly
once, in order, within a few milliseconds, unless it is part of a matched
chord; and nothing qol-tray does (block, restart, crash, get debugged) can
violate that.

Today the process that holds `EVIOCGRAB` also runs HTTP servers, plugin
dispatch, gpui, dev recompiles and `exec` restarts. Every one of those is a
way to stall or drop input. The dispatch channel closed one door; the
architecture keeps the others open.

Split out `qol-input-shim`, a separate long-lived process whose only job is
read, match, re-emit:

- Owns the grabs and one virtual keyboard for the whole session. Built with
  every key code up front, so it is never rebuilt. Survives tray restarts and
  reloads; X never sees a device swap.
- Hot loop touches nothing that can block: no logging to pipes, no locks
  shared with slow work, no plugin code. Chord matches leave over a datagram
  socket with `MSG_DONTWAIT`; if the tray is gone or slow the datagram is
  dropped, typing is not.
- Binding table pushed by the tray over that socket, versioned. With no tray
  connected the shim forwards everything (fail-open), so Alt+Tab falls back to
  the desktop's own switcher instead of vanishing.
- Supervised by the tray; if the shim dies the kernel releases the grabs and
  input goes direct. Shim upgrades hand the grabbed fds and the uinput fd
  across `exec` (the lifeline handoff pattern already in the tree), which is
  safe there because the shim's startup is a few milliseconds.
- Tray side keeps `CaptureDispatch`: the socket reader feeds the same
  `on_fire`; `capture::install` becomes "connect to the shim".

Cheaper interim steps if the split waits: build the virtual device with all
key codes once; route reader diagnostics through the non-blocking
`TraceSink` instead of `log`; keep `qol dev`'s restart as fail-open.

## Squash request

`a38e7bf77` cannot be folded into the commit that introduced the inline call:
that is `52be60290`, 3281 pushed commits back. The 3 s wait that made it
visible is `d25885189` (1225 commits back, also pushed). Either squash rewrites
months of `main` and needs a force-push.
