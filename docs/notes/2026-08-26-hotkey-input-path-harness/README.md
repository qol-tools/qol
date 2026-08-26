# On-metal keyboard path harness

Companion to `../2026-08-26-hotkey-input-path-architecture.md`. Not a workspace
member on purpose: it grabs real input devices and must never run in CI.

- `evprobe.rs`: standalone binary (`evdev = "0.13"`). Creates a uinput keyboard
  named `qol-evdev-stall-probe`, waits for the tray's rescan to grab it, injects
  `PROBE_COUNT` keys `PROBE_GAP_MS` apart, and reports per-key fate: forwarded by
  the tray, delivered straight to the desktop (ungrabbed), or lost. Options:
  `PROBE_TRIGGER=1 PROBE_TRIGGER_COUNT=n` fires F12 first,
  `PROBE_NEWCAP_AT_MS=t PROBE_NEWCAP_CODE=c` hot-plugs a second keyboard that
  forces a virtual keyboard rebuild.
- `xi2watch.py <log>`: records what the X server received
  (`xinput test-xi2 --root`), with the source device id per key.

Run both, then restart or stall the tray mid-injection and compare the three
views. X keycode = evdev code + 8 (F13..F18 = 191..196).
