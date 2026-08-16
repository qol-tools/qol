# Monitor Plugin — Library Readiness Plan

Status: LANDED. All 7 steps verified in tree; review board fixed forward (identity 59f086eba, headless a4031258f, host-fixes fc954a098, adversarial storm 59d2f647f, grant guards 8ea480f97). Next work: `2026-08-16-monitor-plugin-bootstrap.md`.
Scope: mature the shared libraries first; the monitor plugin bootstraps only after its general needs have owners.
Grounding: library gap audit of `libs/` and consuming plugins (bridge report marker e041e716).

## Needs and current state

| Need | Owner today | Gap |
|---|---|---|
| Display enumeration + EDID identity | `qol-runtime::display::x11` (X11 only, connector name) | No EDID identity, no config-key stable id, no Wayland/macOS enumeration |
| Host device-access grant (udev uaccess + reload/trigger, reversible) | `qol-host-fixes::elevation` (pkexec only) | No udev rule writer, no reload/trigger, no reversible lifecycle |
| PortableSession mutation journal (atomic snapshot, checksum, session UUID, one restore path) | `qol-host-fixes::policy::PolicyJournal` | NVIDIA-coupled payload, no session UUID, no content checksum, no crash auto-restore |
| Device-local config that never syncs | `qol-profile-sync::scope` allowlist (`*/device/` rejected) | No path-building API |
| Doctor device-permission checks | `qol-headless::doctor` | None structural; new checks plug in |
| Template headless-first layering | `plugins/template` (main.rs + cli.rs only) | Diverges from `docs/plugin-layout.md` (lib.rs facade mandated) |

## Red flags to fix along the way

- `MonitorBounds` triplicated: `qol-runtime::types`, `qol-windowing::geometry`, window-actions. Pick one owner before identity work.
- `qol-host-fixes::policy::managed.rs` is 2042 lines and NVIDIA-coupled; the journal is buried inside it.
- Journal identity is dev/ino only; a rewritten file passes as unchanged. Needs content checksum.
- `qol-headless/src/doctor/check.rs` has zero tests while sibling modules carry 25+.
- Template and `docs/plugin-layout.md` disagree.

## Work order (smallest first, dependency-aware)

1. Template `lib.rs` facade per `docs/plugin-layout.md`. Zero library dependencies, unblocks every future plugin bootstrap including this one; matches the resolved "template first" decision.
2. Device-scope path API in `qol-profile-sync::scope`. Tiny; unblocks per-display config keys.
3. Extract a payload-generic journal core from the NVIDIA policy journal. Refactor only; existing tests buffer it.
4. Add session UUID, content checksum, and the single shared restore path to that core. This is the PortableSession primitive the monitor spec assumes.
5. Unify `MonitorBounds` into one owner (`qol-windowing`), then add EDID-derived display identity there.
6. Udev uaccess rule writer on `qol-host-fixes` elevation, journaled through step 4, reversible.
7. Doctor checks for device permissions, reusing `qol-headless` DoctorCheck.

Each step: one conventional commit, full gate (`fmt`, `clippy -D warnings`, `build`, `test`) before it lands.
The monitor plugin is bootstrapped after step 1 from the fixed template, consuming steps 2, 4, 5, 6, 7.

## Deferred (plugin-local, not libraries)

AVService churn isolation, LUT co-owner detection, G5 model-id blocklist. These stay inside the plugin per the monitor spec.
