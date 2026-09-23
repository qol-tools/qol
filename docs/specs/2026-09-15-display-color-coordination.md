# Display color ownership and coordination

Architecture, 2026-09-15.
Extends `docs/specs/host-mutation-lifecycle.md` and supersedes the takeover half of `plugins/monitor/src/host_night_light`.
Evidence for every platform claim lives in the three research reports under `~/.local/share/qol-tray/sessions/groups/monitor-night-coordination/rounds/1/` (`night-owners`, `monitor-map`, `cinnamon-levers`).

## 1. Why coordination is needed

The display color transform (the CRTC gamma table on X11, the compositor color transform on Wayland and KDE, the CoreGraphics transfer table on macOS, the GDI ramp on Windows) is a single-writer resource with no operating-system arbitration.
Every owner that writes it believes it is the only writer.
On the reporting host three owners exist at once: qol's night schedule (3800 K, 21:00 to 06:00), Cinnamon's `csd-color` night light (3500 K, off), and the `brightness-and-gamma-applet@cardsurf` panel applet (brightness 0.70, gamma 1.0:0.9:0.8, applied at sunset).

The observed failure has three parts.
qol treats the applet's warm ramp as the display baseline, so qol's night-off restores warmth on the primary display and not on the other.
qol composes its tint on top of the applet ramp, so night-on produces double warmth.
The Cinnamon night-light takeover is a one-shot disable whose result is never re-checked, so a night light that comes back stays on while qol keeps tinting.

The root cause is a single design conflation: one captured table serves both as the state qol restores on release and as the neutral base qol multiplies its brightness and tint onto.
A foreign warm ramp is therefore promoted to "neutral" and can never be undone.
Everything else is a consequence.

## 2. Contract and invariants

1. Two tables per output.
   The `restore_baseline` is the pre-qol host state, captured once into the ledger and written back on release.
   The `compose_base` is the neutral or calibrated table qol multiplies its brightness and tint onto while it owns the output.
   They are different fields, never the same capture.
2. No foreign warmth becomes a `compose_base`.
   A table the classifier calls warmth is not a base; it is an owner to disable or override.
3. Every disable is claimed before it happens and cleared only after a verified restore.
   A claim records the owner, the affected outputs, the prior state, and the restore hint.
   This is the `host-mutation-lifecycle` rule applied to color: qol does not mutate state beyond what it can revert.
4. qol fights warmth, not calibration.
   A VCGT or ICC shaped table is calibration and is folded into the base, never disabled and never classified as an owner.
5. Ownership is enforced, not assumed.
   A one-time disable is not ownership; a bounded drift check with re-assert is.
6. No silent double warmth.
   If qol cannot disable and cannot override a foreign warmth owner, qol does not apply its own warmth through a second channel on that output.
   It reports a conflict instead.
7. Unverifiable transports are not claimed.
   XWayland's RandR gamma calls return success and write nothing, so the X11 transport is only claimed after a readback probe or a session type gate.
8. The host is left as found, and leaving is idempotent.
   Release restores the baseline and every claim, on exit, on recovery, and on residency change.

## 3. Model

```
                 +---------------------------- qol coordinator ----------------------------+
                 | intent: per-output brightness, optional kelvin, schedule, conflict set  |
                 +----------------+--------------------------------+----------------------+
                                  |                                |
                                  v                                v
                       +--------------------+           +----------------------+
                       | ColorTransport     |           | WarmthOwner registry |
                       | read/write/verify  |           | detect/disable/      |
                       | per platform       |           | restore/verify       |
                       +---------+----------+           +----------+-----------+
                                 |                                |
                                 v                                v
                       display color transform        csd-color, gsd-color, KWin
                       (CRTC gamma, CTM, CG, GDI)     NightColor, redshift, Cinnamon
                                                       gamma applets, nvidia-settings
```

The coordinator is a per-output state machine that owns the transform on qol's behalf and holds leases on the owners it has disabled.
It replaces the current split between `Session`/`LutProvider` (per-display baseline and guarded writes) and `host_night_light` (one-shot desktop night-light takeover), and it is the only component allowed to decide who writes a transform.

### Per-output state

| Field | Meaning | Current analogue |
|---|---|---|
| `restore_baseline` | pre-qol table written back on release | `Snapshot.lut` in `session.rs` |
| `compose_base` | neutral or calibrated table qol multiplies onto | implicit in `GammaSession.original` |
| `expected_checksum` | checksum of the last verified qol write | `GammaSession.written_checksum` |
| `intent` | desired brightness percent and tint kelvin | `Snapshot.last_value` / `last_tint` |
| `claims` | owner disable records with restore hints | `host_night_light` journal, one owner |
| `conflict` | per-owner reclaim counters and last drift time | `host_night_light_conflict` boolean |

## 4. Transform classification

Classification is the primitive that makes the rest safe.
It runs on every read the coordinator takes, and it decides whether a foreign table is a base, an owner, or noise.

| Shape | Signature | Meaning | Coordinator action |
|---|---|---|---|
| Identity | all channels equal, linear, full scale | neutral | use as `compose_base` |
| Blackbody | per-channel linear scale, equal exponent, tops match the blackbody table | desktop night light at temperature T | disable the matching owner, then use the unsettled neutral base |
| Ramp gamma | per-channel power curve with differing exponents, equal tops | `xrandr --gamma` writer, panel applet or tool | disable the matching applet or override the table |
| Calibration | non-power-law per-channel curve, or a `vcgt` tag in `_ICC_PROFILE` | ICC calibration | fold into `compose_base`, never disable |
| Calibration times blackbody | calibration curve multiplied by a blackbody scale | desktop night light with VCGT active | disable the night light part, keep the calibration part |
| Unknown | anything else | unrecognized writer | adopt as base when qol is not armed, re-assert once and then report a conflict when qol is armed |

The known tops for the blackbody family come from `cd_color_get_blackbody_rgb_full` with the Planckian flag, which is the exact call `csd-color-state.c` uses, so the classifier compares against measured constants rather than fitting a curve.
The judge between "calibration times blackbody" and "calibration" is the ratio of the three channel tops against the blackbody table, computed after normalizing by the identity endpoints.

## 5. Owner adapters

Each adapter implements four operations: detect presence and applied state, disable while qol owns the output, verify the disable still holds, and restore from the recorded prior state.
The "Level" column is the strongest lever available, and adapters must not use a stronger lever than the level they declare.

| Owner | Platform | Detect | Disable while qol owns | Restore | Level | New code |
|---|---|---|---|---|---|---|
| Cinnamon csd-color night light | Linux X11, Cinnamon | `org.cinnamon.SettingsDaemon.Color` bus name, `Temperature`, `NightLightActive`, gsettings keys | set `night-light-enabled=false` and `Temperature=6500`, record the prior keys | write the prior keys back, restart `csd-color` if it was restarted | disable | generalize `host_night_light/platform/linux` |
| GNOME gsd-color night light | Linux X11 and Wayland, GNOME | `org.gnome.SettingsDaemon.Color` bus name, not schema presence | same as Cinnamon, or `plugins.color active=false` | prior keys, restart the user unit | disable | same module, second settings profile |
| KWin NightColor | Linux X11 and Wayland, KDE | `org.kde.KWin.NightLight` properties `enabled`, `running`, `currentTemperature` | `inhibit()` returns a cookie that forces 6500 K and holds a lock | `uninhibit(cookie)`, or drop the bus connection | disable | new adapter, plus keep the existing kconfig read path |
| wlroots gamma control | Linux Wayland, wlroots family | acquire `zwlr_gamma_control_manager_v1` per output and watch for `failed` | acquiring control takes exclusivity; the previous holder is told `failed` | `destroy` restores the client's original tables | own | new transport, not an owner adapter |
| Mutter display config | Linux Wayland, GNOME | `org.gnome.Mutter.DisplayConfig` with a current `GetResources` serial | `SetCrtcGamma` or `SetOutputCTM` last-writer-wins, no inhibit exists | write the baseline back with `SetCrtcGamma` | clobber | new transport |
| redshift, gammastep | Linux, any session | process name, config file, ramp shape | `SIGUSR1` neutralizes in place, or terminate with `SIGTERM` | `SIGUSR1` again, or restart; `-x` neutralizes without restoring | detect and neutralize | new adapter |
| Cinnamon gamma applets | Linux X11, Cinnamon | enabled applets through `libs/cinnamon` Eval, config file, ramp shape | remove the exact `enabled-applets` entry, record the entry and config bytes | write the config while disabled, re-add the entry | disable | new adapter on the existing Eval bridge |
| nvidia-settings | Linux X11, NVIDIA | `~/.nvidia-settings-rc`, its autostart entry, NV-CONTROL attributes | `libs/host-fixes` takeover of the rc file and autostart, Resident only | clear the marker and reload | disable | new adapter on `host-fixes` |
| csd-color as VCGT applier | Linux X11, Cinnamon | `_ICC_PROFILE` on the root window plus a `vcgt` tag | nothing narrower than stopping `csd-color`; do not do this by default | restart `csd-color` | detect only | classification only |
| macOS Night Shift | macOS | private `CoreBrightness` framework, otherwise undetectable | private API or UI only | not applicable | detect only | documentation plus a conflict state |
| Windows Night Light | Windows | CloudStore registry blob | registry write or UI only | not applicable | detect only | documentation plus a conflict state |
| Generic gamma writers | all | ramp shape, process or module evidence | override the transform by re-assert (bounded) | write the baseline back | clobber | coordinator fallback |

The Cinnamon applet adapter is worth calling out because it is the report's case.
The applet is fully revertible: `gsettings get org.cinnamon enabled-applets` holds the exact entry (`panel1:right:1:brightness-and-gamma-applet@cardsurf:15` today), the config JSON holds the preset list and last values, and `org.Cinnamon.updateSetting` pushes single keys live through the shell.
Disabling the applet does not clear its ramp, so the coordinator must neutralize the transform itself; re-enabling the applet re-applies its current preset within about a second, which is the correct hand-back behavior.

## 6. Lifecycle

The coordinator adds four per-output moments to the three host-mutation moments.
They run on daemon start, on hotplug, and whenever the first qol write touches an output.

1. Discover.
   Enumerate outputs, probe the transport capability, read and classify each transform, and ask every adapter for its presence and applied state.
2. Claim.
   Persist the `restore_baseline` and one claim record per owner that will be disabled, before the first disable.
   This is `SessionStore::claim` from `libs/host-session` with a new owner namespace, reusing the existing generation and handoff machinery.
3. Disable.
   Call each adapter's disable in a fixed order (native night lights first, then applets and tools), and record which succeeded.
   A failed disable removes that claim and downgrades the output to override or conflict.
4. Settle.
   Poll the transform until two consecutive reads agree, bounded at five seconds.
   The existing fixed four-second `HOST_NIGHT_LIGHT_SETTLE_SECS` becomes the bound, not the rule.
5. Compose.
   Classify the settled table.
   Calibration or identity becomes the `compose_base`.
   Warmth is rejected: keep the previous clean base if one exists, otherwise derive the neutral table by dividing out the classified blackbody or ramp-gamma factor.
   Apply `compose_base.dimmed(brightness).tinted(kelvin)` through the guarded write.
6. Watch.
   On every ownership tick, read the transform.
   If the checksum matches `expected_checksum`, do nothing.
   If it differs, classify it.
   Calibration drift is adopted as a new base and re-applied.
   Warmth drift means an owner reclaimed the output: re-disable the adapter if the shape maps to one, re-assert the composed table, and increment that owner's conflict counter.
7. Release.
   Write `restore_baseline` when the live table is still qol's, restore every claim, and clear a claim only after its adapter verifies the restore.
   A foreign table that appeared between qol's last write and the release keeps the baseline record and reports a failed release instead of deleting the record.

The bounded fight is deliberate.
Re-asserting every tick forever against a writer that reclaims every tick produces visible flicker and burns process spawns.
After three consecutive reclaims on one output, the coordinator stops re-asserting, marks the output as conflicted, and leaves the other owner's table in place.
The conflict is visible in the night-mode payload and the settings surface, with actions to keep overriding, disable the other owner, or turn qol's night mode off.

## 7. Existing-code deltas, phased

Each phase is a commit series that leaves the tree green.
Phase 0 alone fixes the reported bug and touches no new subsystems.

### Layout and platform boundary contract

The capability lives in one directory, `plugins/monitor/src/display_color/`, and no new file joins the plugin source root; the existing root modules are pre-existing debt and are edited in place.
Platform differences sit behind one `display_color/platform/` facade whose `linux/`, `macos/`, `windows/` and complement-cfg `fallback/` modules all use directory form.
Substrate implementations live in `display_color/backends/` and are named for the real boundary rather than the OS.
`cfg(target_os)` appears only in the facades and in `mod.rs` re-exports; `mod.rs`, `classifier.rs`, `output_state.rs`, `policy.rs`, `transport.rs` and `owner.rs` stay platform-free.
Every target outside Linux, macOS and Windows compiles through `fallback/`, whose functions return typed unsupported errors instead of panicking.
Platform-only dependencies are declared under `[target.'cfg(target_os = ...)'.dependencies]` in `plugins/monitor/Cargo.toml`.
`plugin.toml` keeps `platforms = ["linux", "macos"]`, so a host never offers the plugin where its runtime transports are stubbed.
The phase gate runs `cargo fmt --check`, `cargo clippy --all-targets --all-features --keep-going -- -D warnings`, `cargo build` and `cargo test` on the host, plus `cargo check` for one exotic target so the fallback path compiles.
Every relocation of existing code lands as a move-only commit before the behavior commit that depends on it.

### Phase 0, correctness

1. Split the base.
   `plugins/monitor/src/monitor/backends/x11_randr_gamma.rs:128-135` adds `compose_base` beside `original`, `set_inner` composes against it at `:221`, and `adopt_baseline` at `:450-465` seeds both.
2. Split the persisted snapshot.
   `plugins/monitor/src/session.rs:64-85` adds a serde-default `compose_base` field; a snapshot without it derives one from `lut` on load, so existing files keep working.
3. Never capture warmth as a base.
   `adopt_persisted` at `session.rs:606-636` and `ensure_snapshot` at `:638-679` classify the live table before adopting it.
   A warmth shape keeps the persisted `restore_baseline` and produces a derived base instead of overwriting the snapshot.
4. Re-assert the host takeover.
   `daemon.rs:914-936` checks the live owner state on every armed tick and disables again when it came back.
   Handoff adoption at `host_night_light/platform/linux/controller.rs:237-247` re-syncs from the live settings before declaring the takeover active.
5. Run the ownership tick.
   `daemon.rs:979-1003` and the loop at `:1658-1700` arm the tick whenever qol owns an output, not only when the night schedule is active.
6. Keep the baseline on a foreign restore.
   `session.rs:966-976` stops deleting the snapshot when `ForeignLutPreserved` is returned, and reports the failed release.
7. Guard the re-assert.
   `session.rs:985-1005` compares the live table against the expected checksum before writing, and surfaces a drift event when they differ.
8. Stop writing `night-light-schedule-mode='always'`.
   `host_night_light/platform/linux/mod.rs` preserves the user's schedule mode and temperature and restores them, so `DisabledUntilTomorrow` stays usable and the host is not left on a permanent 3500 K.
9. Gate the X11 transport.
   `platform/linux.rs` and `x11_randr_gamma.rs` refuse to claim gamma on a Wayland session unless a write and readback probe verifies the driver honors it.

### Phase 1, owners and claims

10. Create `plugins/monitor/src/display_color/` with `mod.rs` (facade and lifecycle), `classifier.rs`, `output_state.rs` and `policy.rs` (platform-neutral), `transport.rs` and `owner.rs` (traits), one `platform/` facade, and `backends/` named by substrate: `x11_randr`, `wlroots_gamma`, `mutter_display_config`, `kwin_nightlight`, `csd_settings`, `gsd_settings`, `redshift`, `cinnamon_applet`, `nvidia_settings`, `coregraphics`, `gdi`, `night_shift_detect`, `windows_night_light_detect`.
    The Linux facade selects transport and owner set by session type and desktop, the macOS and Windows facades select their transport plus detect-only owners, and the fallback facade returns typed unsupported errors.
11. Fold `host_night_light/` into `display_color/backends/` as `csd_settings` and `gsd_settings` in a move-only commit, so the existing takeover behavior is preserved before the coordinator calls it differently.
12. Move the gamma access in `monitor/backends/{x11_randr_gamma,cg_gamma,shared_display}.rs` behind the `ColorTransport` trait in a move-only commit, keeping the current session machine working through the new trait.
13. Claim records in `libs/host-session` under a `display-owner` subdir, one record per owner per output set, with the existing generation and handoff rules.
14. Adapters for KWin NightColor inhibit, redshift and gammastep, Cinnamon gamma applets, and nvidia-settings, with the process and autostart cases going through `libs/host-fixes::takeover` and the applet case through `libs/cinnamon`.

### Phase 2, coordinator policy

15. The coordinator state machine from section 6 wired into `evaluate_night` and the new ownership tick, replacing `reconcile_host_night_light` and the raw tint loops.
16. Conflict accounting per owner, surfaced in the `night_mode` payload, the doctor, and the settings surface, with the yield action.
17. A `night_coordination` config value with two supported modes: `own` (default, disable reachable owners and override the rest) and `detect` (never disable, surface conflicts, apply qol warmth only when the transform is clean).

### Phase 3, other transports

18. Wayland transports: `zwlr_gamma_control_manager_v1` for wlroots, Mutter DisplayConfig gamma and CTM for GNOME, and the KWin inhibit path for KDE Wayland.
19. macOS and Windows owner detection (Night Shift, Night Light), reported as conflicts rather than disabled, plus the existing CG and GDI transports underneath.

## 8. Platform matrix

| Platform | Transport | Native owner | qol policy |
|---|---|---|---|
| Linux X11, Cinnamon | XRandR gamma, verified | csd-color, applets, redshift, nvidia-settings | disable csd and applets, neutralize or terminate tools, override unknown warmth |
| Linux X11, GNOME | XRandR gamma, verified | gsd-color 46 and older, tools | same, gsd settings profile |
| Linux X11, KDE | XRandR gamma plus KWin NightColor | KWin owns warmth, delegates external brightness | inhibit NightColor, own the X11 table |
| Linux Wayland, wlroots | `zwlr_gamma_control_manager_v1` | the compositor and whichever client holds the control | take the control, `destroy` on release |
| Linux Wayland, GNOME | Mutter DisplayConfig gamma or CTM | Mutter owns night light | write gamma while gsd is inactive, conflict if Mutter reclaims |
| Linux Wayland, KDE | no client gamma protocol | KWin owns warmth and gamma | NightColor inhibit is the only lever, brightness stays with KWin |
| macOS | CoreGraphics transfer table | Night Shift, gamma apps | own the table, detect Night Shift and conflict rather than stacking |
| Windows | GDI ramp, driver-clamped | Night Light, gamma apps | own the ramp, detect Night Light and conflict |

## 9. Verification

1. Unit tests for the classifier over synthetic tables: identity, blackbody at each known temperature, `xrandr --gamma` power curves, VCGT-shaped curves, and calibration times blackbody.
   Each case asserts the class and the derived base.
2. Unit tests for the lifecycle: claim before disable, claim cleared only after restore, release with a foreign table keeps the baseline, and the bounded fight stops after three reclaims.
3. Guest lanes on `mint-cinnamon` reproducing the reported scenario end to end: applet night preset applied, qol night on, exactly one warmth owner, applet disabled and restored, csd re-enable during ownership triggers re-assert, `kill -9` recovers the baseline, and Portable versus Resident exit behavior.
4. A wlroots guest lane for the gamma-control transport, and a KDE lane if an environment exists, for the inhibit path.
5. The host NVIDIA round trip is already proven by the research (a table written by one client reads back identically from another), so the host check after Phase 0 is the reported scenario repeated on the physical machine.

## 10. Decisions requested

1. Fight intensity: the recommended default is a bounded fight, three consecutive reclaims before qol stops and reports a conflict.
   The alternative is unconditional ownership, which guarantees qol wins but can flicker against a writer on its own timer.
2. Disabling the Cinnamon gamma applet: the recommended default is automatic while qol's night mode is armed, because the applet is fully revertible through `enabled-applets` and its config file, and that is what "override the host night mode" means to the user.
   The alternative is detect-only with a manual take-over action.
3. `csd-color` as VCGT applier: the recommended default is never to stop it, so display calibration survives; the coordinator neutralizes it through settings and classifies its calibration output as a base.
   The alternative is a Resident-only full take-over that kills the process and masks its autostart through `libs/host-fixes`.
4. macOS Night Shift and Windows Night Light: the recommended default is detect and conflict, because disabling them needs private APIs or registry writes without a clean restore.
   The alternative is to keep qol's own warmth off while they are active so the user never sees double warmth.

## 11. Open questions

1. Does Mutter's `SetCrtcGamma` compose with Mutter's own night light, or does one overwrite the other, and does `org.gnome.settings-daemon.plugins.color active=false` actually stop the Mutter-side transform on Wayland?
   This needs a GNOME guest measurement before Phase 3.
2. Can the VCGT curve be decoded from `_ICC_PROFILE` in-process to separate calibration from a blackbody factor exactly, or is the classifier's top-ratio approximation enough for the disable decision?
   The approximation ships in Phase 0; exact decoding can follow.
3. Does the X11 daemon have a viable event source (`RRNotify`) for drift instead of polling, and what is the cost of an `x11rb` event thread in the monitor daemon?
   The tick is the Phase 0 carrier either way.
4. Does KWin's `inhibit()` also neutralize the ramp on an X11 KWin session, where the compositor writes the CRTC directly?
   The API documentation covers Wayland; X11 needs a measurement.
5. What should qol write instead of `night-light-schedule-mode='always'` when it does use the native renderer on Cinnamon?
   Cinnamon has no per-plugin `active` key, so the honest choices are leaving the user's schedule mode alone and only writing `night-light-enabled` and `night-light-temperature`.
