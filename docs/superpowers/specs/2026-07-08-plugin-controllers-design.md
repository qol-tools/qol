# plugin-controllers design

Date: 2026-07-08
Status: draft for review

## Purpose

Game controllers with buggy firmware need driver-side workarounds that today live in per-PC config files.
The motivating case: the GuliKit Controller XW rumbles forever over Bluetooth until xpadneo gets quirk `263` for the pad's MAC.
plugin-controllers makes such fixes part of the qol profile, so every PC set up with qol-tools gets them without hand-editing `/etc/modprobe.d`.

## v1 scope

Fix applier only.

- Curated fix database shipped inside the plugin binary.
- Detect known-broken controllers on hotplug.
- Report fix state; apply fixes only on explicit user action.
- Doctor output describing driver and fix state per detected pad.

Non-goals for v1: rumble tester, firmware update helper, battery levels, controller list UI, remapping, macOS/Windows support.

## Fix database

A static table in Rust, one entry per known controller defect.

Entry fields:

- `id`: stable slug, e.g. `gulikit-xw-bt-rumble`.
- `match`: transport (bluetooth/usb), HID vendor:product, device name substring, optional MAC prefix.
- `driver`: kernel module the fix targets, e.g. `hid_xpadneo`.
- `fix`: modprobe option line and the equivalent runtime sysfs write.
  For the GuliKit entry: `quirks=<MAC>:263` written to `/etc/modprobe.d/qol-controllers.conf` and to `/sys/module/hid_xpadneo/parameters/quirks`.
  The MAC is read from the connected device at apply time, not hardcoded.
- `docs`: one-line human explanation shown in notifications and doctor.

## Daemon behavior

Follows the plugin-lights availability pattern: the daemon never escalates privileges and never prompts.

1. On start and on udev hotplug events, enumerate input devices and match against the fix database.
2. For each match, compute fix state read-only:
   - `driver-missing`: target module not installed (e.g. no xpadneo).
   - `pending`: driver present, fix neither persisted nor live.
   - `live-only`: sysfs applied this boot, not persisted.
   - `applied`: persisted in modprobe.d (live state verified when readable).
3. `pending` and `driver-missing` produce one tray notification per pad per daemon lifetime, plus status output.
4. No background writes, ever.

## Apply action

A runtime action (`apply-fixes`) the user triggers from the tray or the notification.

- Runs `pkexec` with a small helper invocation that writes `/etc/modprobe.d/qol-controllers.conf` and the sysfs parameter.
- One authorization prompt per PC, at a user-chosen moment.
- After success the fix state flips to `applied` and later boots need nothing, because modprobe options load with the module.
- `driver-missing` is not fixed by the plugin; the action reports the xpadneo install command instead.

## Doctor

Per detected known pad, report: match id, driver present or missing, fix state, and the remediation command when the state is not `applied`.
When no known pad is connected, report the database size and that nothing needs attention.

## Platform

`platforms = ["linux"]` in plugin.toml.
Source still uses the standard `platform/` layout so macOS support can be added later without restructuring.

## Structure

Bootstrapped from plugin-template.

- `src/fixes/`: fix database types and the static table.
- `src/detect/`: udev watch and device matching.
- `src/state/`: read-only fix state computation.
- `src/apply/`: privileged apply invocation and result parsing.
- `src/daemon/`: socket daemon wiring via qol-plugin-daemon, `DaemonRuntime`-style availability.
- `src/cli.rs`: help, doctor, apply-fixes entry points.

## Testing

- Table-driven tests for device matching (name, vendor:product, MAC prefix cases).
- Fix state computation tests against fake filesystem roots (modprobe.d present/absent, sysfs present/absent).
- Doctor output tests per state.
- No tests for the pkexec shell-out beyond argument construction.

## Risks

- The pad reports a locally administered MAC (`06:71:10:...`); if it changes across re-pairings the persisted quirk goes stale.
  Mitigation: the daemon re-checks on every connect, so a MAC change surfaces as `pending` again and one more apply fixes it.
- Distro differences in polkit/pkexec availability; doctor reports when pkexec is missing.
