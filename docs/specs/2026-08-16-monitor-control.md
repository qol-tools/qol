# Monitor Control Plugin — Concept Spec

Status: proposed, pre-naming.
Owning skills: `qol-mission`, `qol-arch-code`, `qol-shared-libs`.

## Concept

One plugin owns every property of an attached display that software can reach:

- Brightness (DDC/CI hardware value, or GPU-LUT software value)
- Contrast, input source, volume, color presets (DDC/CI command space)
- Resolution and refresh modes
- HDR toggle, orientation

One consistent surface: hotkeys, CLI commands, plugin actions, settings panel.

Phase 1 ships only mature brightness with fallback and hotkeys.
Everything else stays on the capability contract so phase 1 does not paint the architecture into a corner.

## Motivation (grounding)

The Odyssey G5 incident: OSD brightness silently at 0 while macOS reported nothing.
Software brightness control exists per OS, but capability varies per display, per transport, per OS.
The plugin must probe honestly and say when the only path is a GPU LUT, because a LUT never moves the panel's own OSD value.

## Capability contract

```
DisplayHandle        stable display identity (EDID-derived), never a path or index
                     carries id() -> String: stable across boots, used as config key
DisplayCapabilities  what this display supports (brightness_ddc, brightness_gamma, contrast, modes, hdr)
DisplayControl       the strategy facade, one impl per OS, plus typed stubs
```

Facade surface (grown, never shrunk):

```
enumerate() -> Vec<DisplayHandle>
probe(handle) -> DisplayCapabilities
get_brightness(handle) -> BrightnessState   # source + value + hardware/software
set_brightness(handle, value)               # via selected source
list_modes / set_mode / get_hdr / set_hdr   # later phases, typed stubs today
```

`BrightnessState` always reports which source produced the value.
A gamma-produced value is labeled software.
That label is what makes the G5 trap visible instead of silent.

## Platform reality

- Windows: brightness via the High-Level Monitor Configuration API VCP subset (`GetMonitorBrightness` / `SetMonitorBrightness`, `Get/SetVCPFeature` for contrast).
  No raw DDC byte access; the OS wraps it.
  WMI brightness methods cover only internal laptop panels, never external DDC.
  Gamma fallback via `SetDeviceGammaRamp` is partial and legacy; treat as a per-display probe result, not a promise.
  Modes via `SetDisplayConfig`. Public APIs, no extra deps.
- macOS: DDC via IOKit AVService (the Lunar/MonitorControl path), which is also a wrapped API, not raw I2C.
  Apple Silicon needs the arm64 display-I2C transport; DDC over HDMI or HDMI adapters is unreliable, USB-C/Thunderbolt-to-DP is the dependable path.
  Software fallback via CoreGraphics transfer tables (`CGSetDisplayTransferByTable`): dim-only (multiplies at most 1.0), resets on sleep/wake/reconnect/color-profile change, and conflicts with Night Shift and True Tone.
  Resolution change needs private CoreDisplay APIs (`CGDisplaySetDisplayMode` is deprecated and unreliable on Apple Silicon); that is why modes are a later, separately-reviewed phase.
- Linux: the only OS with raw DDC access: speak the protocol over `/dev/i2c-*` directly (0x37 DDC, 0x50 EDID; no ddcutil dependency; mission invariant 5).
  i2c-dev must be loadable, and `/dev/i2c-N` is root:root 0600 by default; a udev rule or i2c group membership is the normal fix, and that permission gap must surface in doctor and in UI, never silently.
  Gamma fallback: X11 via RandR (`SetCrtcGamma`, in-crate x11rb, dim-only); Wayland via `wlr-gamma-control` (wlroots family plus Hyprland, niri, Jay; absent on KWin, Mutter, COSMIC, Weston, Mir), else typed unsupported.

### Phase-1 support matrix

| OS | ddc | gamma | none |
|---|---|---|---|
| Windows | external DDC monitors | partial (`SetDeviceGammaRamp`, legacy) | laptop panels |
| macOS | USB-C/DP external; HDMI unreliable | CG LUT, dim-only | non-DDC, HDMI |
| Linux X11 | yes (permissions) | RandR | - |
| Linux Wayland | yes (permissions) | wlroots family + Hyprland/niri/Jay | KDE/GNOME, COSMIC |
| Odyssey G5 class | vendor-quirked (writes dropped or crash until OSD re-arm) | yes | non-DDC panels |

Phase-1 risk tiers: implementable (Windows DDC, Linux raw I2C DDC, X11 gamma, macOS CG-LUT gamma, macOS AVService DDC); risky (macOS AVService private-API breakage per OS release, i2c-permission UX, gamma conflicts with Night Shift/True Tone and compositors, stable EDID identity on macOS); near-impossible for phase 1 (macOS modes without private APIs, gamma on KDE/GNOME Wayland, dependable DDC on the G5 itself). Wayland modes are feasible later via `wlr-output-management-unstable-v1` (set_mode/set_custom_mode), but only on the wlroots family, Hyprland, niri, COSMIC, and Jay: KWin exposes its own kde-output-management-v2 instead, Mutter exposes nothing public, and the protocol is unstable so minor churn is possible.

### macOS bottleneck mitigations (deep-dive, ranked)

1. **AVService churn (severity 5).** Recurring annual breakage: Sequoia crashes, Tahoe breakage, macOS 27 beta patch churn across MonitorControl/Lunar/BetterDisplay. Mitigation: dlopen/dlsym the AVService symbols so a missing or renamed symbol is a typed error, never a crash; runtime write plus read-back probe before declaring DDC capable; re-probe per wake and hotplug; doctor treats a new macOS minor as a compatibility unknown, never assumed; all churn isolated inside the `avservice` backend.
2. **HDMI unreliability on Apple Silicon (severity 4).** Destructive failure mode (garbled screen, blown-out display), so it violates "host left as found". Mitigation: read the Transport IORegistry property; default HDMI to auto-gamma with DDC discouraged; forced `ddc` policy on HDMI surfaces a doctor error; probe before trusting; per-hotplug transport re-read.
3. **CG LUT resets and conflicts (severity 3).** Night Shift/True Tone/color-profile changes clobber LUTs silently. Mitigation: checksum-guarded restore plus a re-assert enforcer that only runs while the LUT checksum still matches the plugin's write; on mismatch, stop and alert rather than fight the co-owner.
4. **EDID identity on macOS (severity 2).** Collisions on identical dual monitors, not breakage. Mitigation: primary key is the EDID hash (private `IODisplayCreateInfoDictionary` or `CopyDisplayInfo`), connector as collision disambiguator, unstable-identity flag when serial is absent. Note: `CGDisplayCreateUUIDFromDisplayID` is gone on macOS 26 (verified live), so the public-UUID path is dead; EDID-hash identity is the primary key on every OS.

### Linux bottleneck mitigations (deep-dive, ranked)

1. **i2c permissions (severity 4).** `/dev/i2c-N` is root:root 0600 by default (kernel devtmpfs; i2c-dev has no devnode override). ddcutil's own fix is a udev uaccess rule for class 0x030000. Failure modes to distinguish at probe: EACCES (no grant), ENOENT (i2c-dev not loaded), EBUSY (ddcci-driver conflict). Mitigation: doctor tiers these distinctly; a one-time, UI-driven, polkit-prompted grant writes the same uaccess rule plus `udevadm control --reload` and `udevadm trigger`, fully reversible; gamma fallback keeps the user unblocked meanwhile.
2. **Wayland gamma variance (severity 3).** `zwlr_gamma_control_manager_v1` exists on the wlroots family plus Hyprland, niri, Jay; absent on KWin, Mutter, COSMIC, Weston, Mir. The protocol is exclusive per output, so a night-light client steals control. Mitigation: runtime bind plus write plus read-back (spec already mandates); failed control = typed unsupported-or-conflict, surfaced in doctor. Later per-compositor paths: KWin's kde_external_brightness_v1 (DDC brightness through the compositor, inheriting its i2c access, brightness only), GNOME's deprecated SetCrtcGamma.
3. **X11 RandR gamma (severity 2).** `SetCrtcGamma` is a multiplicative 16-bit ramp, so brightening past 100 percent is impossible; ramps die on mode set, X restart, and some drivers on sleep; XWayland implements no gamma, so xrandr-style gamma is inert there. Mitigation: write plus read-back verification (XWayland fails honestly into typed unsupported), checksummed restore, re-apply on RRNotify, clamp at 100.
4. **Wayland modes (severity 2, later phase).** Covered by the corrected wlr-output-management matrix above; KDE/GNOME modes need per-compositor paths and stay gated.

### Gamma bottleneck mitigations (deep-dive, ranked)

1. **LUT co-owners (macOS 4, Windows 3, X11 4, Wayland 2).** MonitorControl's proven detector: read the table back, compare peak ratio to the written value, count mismatches, warn at 3. Attribution is impossible (tables compose), so detection is reactive, not preventive: passive read-back before first gamma use, no startup probe-write.
2. **HDR active (macOS 4, Windows 4, Linux 2).** LUT writes are no-ops under HDR. Detection: macOS via public `NSScreen.maximumExtendedDynamicRangeColorComponentValue`; Windows via `IDXGIOutput6::GetDesc1` ColorSpace G2084; Linux has no unified path (EDID CTA-861 metadata plus DRM properties, not readable via RandR on X11). Refuse gamma while HDR is active.
3. **Resets on sleep/wake/reconnect (macOS/Windows/X11 4, Wayland 2).** Already covered by the sleep/wake re-enumerate and checksum re-assert policy.
4. **Checksum-guarded restore (macOS/X11 5, Windows 3).** Read-back is exact on macOS and X11. Windows gamma writes can fail silently (returns TRUE without setting the ramp), so every Windows write is verified by read-back before its checksum is trusted.

Phase-1 policy: per-write read-back with one retry; mismatch counter; after 3 mismatches a reactive warning with per-display gamma opt-out; refusal only for HDR-active. Never auto-refuse on co-owner mismatch: Night Shift and Night Light are legitimate, and the checksum guard already makes fighting impossible. On Wayland, skip gamma entirely (typed unsupported), preserving DDC.

### Hardware bottleneck mitigations (deep-dive, ranked)

1. **Odyssey G5 class (severity 5).** DDC is not absent; it is hostile and stateful: feature reads answer, but brightness writes crash the monitor or fail verification intermittently, writes only succeed after touching the physical OSD, and opening or closing the OSD re-arms DDC. ddcutil ships a per-model-id `--disable-ddc` workaround and calls Odyssey problematic. Mitigation: test write plus read-back verification; persist the EDID model-id; per-model-id blocklist; auto-gamma with a doctor flag ("vendor-quirked DDC: writes ignored until OSD touched") and a hint to press the OSD menu once after boot.
2. **macOS Apple Silicon HDMI (severity 4).** HDMI-connected panels commonly miss DDC features; USB-C/Thunderbolt-to-DP is the dependable path. Mitigation: per-connection DDC feature probe at enumerate time; default HDMI displays to gamma; honest label ("DDC limited over HDMI on this Mac") plus cable advice in doctor.
3. **Reads OK, writes dropped (severity 4).** Reads pass, writes silently no-op or corrupt (also BenQ EX3210U invalid feature flags). Mitigation: mandatory read-back after every write with settle delay; two consecutive mismatches downgrade the source and set a writes-dropped capability flag; retry once, then downgrade; never loop blindly, because the G5 crashes on repeated writes.
4. **DisplayLink and MST (severity 3).** DDC/CI never traverses a DisplayLink USB link (the DL chip is the DDC endpoint); MST branch writes fail with EIO. Mitigation: transport identity probe via the sysfs/IOKit parent device chain (USB vendor 17e9 = DisplayLink); typed unsupported with an honest reason; gamma fallback.

Detection machinery overlaps: one test-write plus read-back path and one transport-identity probe cover items 2, 3, 4; only the G5 class needs the model-id blocklist.

## Fallback policy (phase 1 core)

Per display, per config: `auto | ddc | gamma | off`.

`auto` prefers DDC when the probe succeeds, else gamma when the OS exposes it, else unsupported.

Selection is per display and persisted in plugin config (ObjectArray keyed by display id, `select = auto|ddc|gamma|off`).
It is not a global guess.

Hard rule: policy `ddc` plus probe failure is a surfaced error in doctor and UI, never a silent fallback to gamma.

Gamma consent: no blocking per-display gate. `auto` applies gamma without a prompt; the first gamma use per display shows a one-time non-blocking notice ("software dimming; the panel's OSD value will not change"), the value is always source-labeled, and interference with other LUT owners triggers a reactive warning. Opt-out stays available per display (`select = gamma|off`). Precedent: MonitorControl applies gamma automatically with only a per-display "Avoid gamma table manipulation" opt-out and a reactive interference alert, never an upfront prompt.

## Architecture decisions

- New capability, plugin-local: no existing brightness/DDC owner in `libs/` or `plugins/`.
  Extract a shared crate only if a second consumer appears.
- No cross-OS I2C abstraction: only Linux has raw DDC bytes. The byte protocol lives inside the Linux backend.
  macOS and Windows wrap OS APIs, each its own backend. Layout follows the headless-first shape:

```
plugins/<name>/
  src/
    main.rs
    lib.rs
    cli.rs
    monitor/
      mod.rs          # facade, probe orchestration, fallback policy, state machine
      platform/
        mod.rs
        linux.rs      # selects the ddc or gamma backend
        macos.rs
        windows.rs
        fallback.rs   # exotic targets, typed errors
      backends/
        i2c_ddc.rs            # Linux: raw DDC protocol bytes over /dev/i2c-*
        hlcm.rs               # Windows: VCP subset via HLMC API
        avservice.rs          # macOS: IOKit AVService
        x11_randr_gamma.rs    # Linux X11 gamma
        wayland_wlr_gamma.rs  # Linux Wayland gamma
```

- Headless CLI first: `list`, `status`, `get`, `set`, `up`, `down`, `doctor`, `doctor --json`, `help`.
  plugin.toml runtime actions map to these commands.
  Doctor reports: platform support, per-display DDC probe result, i2c permissions on Linux, config readability.
- Hotkeys are host-claimed (qol-tray owns its surface).
  The plugin declares actions; `qol-hotkeys` supplies grammar/keycodes if custom parsing is ever needed.
  Brightness up/down are continuous actions (`[daemon]` with `continuous = true`, window-actions precedent) so key hold ramps.
  Feedback on hotkey press: a qol-gpui toast or overlay showing the brightness bar and its source.
- Settings panel via the shared gpui surface kit, not plugin-local UI.

## Edge-case policy

- **Identity.** EDID hash is the primary key, connector as disambiguator, never a path.
  Port is never part of the persistent key, so a port swap re-matches and re-applies session state.
  Unreadable EDID: mark identity unstable, refuse config binding, flag in doctor.
- **Hotplug.** Re-enumerate on hotplug and before every set/get.
  A vanished handle is a no-op that keeps the journal alive for the same identity returning.
- **Crash restore.** Atomic snapshot (temp+rename), per-display checksum plus session UUID, idempotent restore.
  A snapshot with zero recorded mutations is deleted, never restored.
- **Gamma co-owners.** Probe reports LUT conflicts where detectable; refuse gamma while HDR is active (LUT writes are no-ops there); restore only when the LUT checksum matches what the plugin wrote. Detection is reactive: passive read-back, mismatch counter, warning at 3 with a per-display opt-out. Never auto-refuse on co-owner mismatch.
- **DDC writes.** Verify every write by read-back after a settle delay, one retry; unchanged read-back downgrades the source visibly. Permission drift mid-session: per-operation check, doctor alert.
- **Wayland.** Gamma capability is runtime-verified by write plus read-back, never assumed from protocol presence.
  Missing gamma never blocks DDC brightness. Doctor reports per-compositor capability.
  Ships as the `display_server` doctor line: X11 vs Wayland vs headless (detection testable; gamma note carried in the line).
- **Sleep/wake.** Subscribe to power events; invalidate handles and capabilities on wake; re-enumerate before the first post-wake mutation.
  Deferred (2026-08-16, board req-2): every set/get, step, and restore re-enumerates through the control facade, so freshness holds without cached handles; a vanished display keeps its snapshot alive for the same identity returning (connector fallback restore). Power/inhibit events (IOKit on macOS, logind on Linux) cannot be faked through the existing seams on the macOS host, so a subscription would be untestable and behavior-neutral.
- **Hotkey repeat.** Default step ~5 percent, ~70 ms debounce during hold-repeat, clamp at 0/100, toast shows value plus source each step. All values configurable.
- **One restore path.** Exit, plugin disable, and crash recovery all share the same idempotent restore path.
- **uaccess rule-file loss while active.** When the journal is Active but `90-qol-i2c-uaccess.rules` is missing, the next grant re-applies the canonical rule (atomic write, reload + trigger once) and reports success: recovery to the journaled intent, not a busy error. A present rule file with different content is never overwritten: grant refuses naming the file, the expected vs actual sha256, and the manual remedy (remove or restore it, then retry).
- **Operator-modified rule file.** Grant refuses before writing when the rule file exists with non-canonical content, checksum-guarded exactly like revoke. Re-grant over an identical previously-written rule stays idempotent.
- **Stale uaccess tag after revoke (accepted window).** The udev database keeps the uaccess tag until the next i2c device event, so logind re-applies the ACL at the next session activation. Accepted trade-off: purging via `udevadm trigger` would re-run operator rules (e.g. ddcutil's) and re-grant, breaking host-left-as-found. Proper fix deferred to revocation UX.
- **Stale grant temp file.** A `.qol-<pid>` temp left by a killed grant script is inert (udev reads only `.rules` names) and preserved: indistinguishable from an operator file, so qol never deletes it.
- **Revoke without acl tools.** Restore fails closed (exit 5) naming the missing binaries and the `acl` package as the remedy when getfacl or setfacl is absent.

## Mission compliance

- Every brightness mutation is `PortableSession`: snapshot before first change, restore on exit, deterministic recovery after abnormal exit (stale snapshot without clean-exit marker restores on next start).
- Preferred brightness is config in the device scope (host-local, never synced), re-applied at start through the same session-scoped path. Precedence: crash restore first, then preferred brightness as a fresh snapshot, then exit restore; config never overrides restore. Durable brightness while qol-tray is not running would be genuine ResidentPolicy: defer past phase 1.
- No host-side dependencies to install: no ddcutil, no xrandr CLI, no external helpers.
- Unsupported display, missing permission, failed DDC: visible in doctor and in the UI. Silent failure is a bug.
- Every end-of-session (exit, disable, crash) shares one restore path; determinism holds for all three.

## Resolved design decisions (previously open questions)

- **Template first.** `plugins/template` lacks `lib.rs` and its `main.rs` declares modules inline, violating the headless-first layering it propagates. Fix: add `src/lib.rs` (`pub mod cli; pub mod platform;`), shrink `main.rs` to delegate to `plugin_template::cli::exit_code`, keep the manifest test in `main.rs`, no Cargo.toml change (auto-discovery, verified against the plugin-lights precedent). Blast radius is future plugins only; no CI references the template. Bootstrap the monitor-control plugin from the updated template.
- **No gamma consent gate.** See Fallback policy above.
- **Brightness persistence is config, not host policy.** See Mission compliance above.
- **One plugin, modes included.** Modes land in a later phase: Windows (`SetDisplayConfig`, public), X11 (`RRSetCrtcConfig`, in-crate), Wayland (`wlr-output-management-unstable-v1`, wlroots family only) first; macOS gated behind its own review with a contained CoreDisplay backend. Splitting would duplicate EDID identity, hotplug invalidation, doctor probes, and config keying into two sources of truth for one monitor, and force a shared-crate extraction before a real second consumer exists. Revisit the split only on concrete signal: macOS modes churn polluting the brightness cadence, or a second display-capability plugin appearing.

## Phase-2 platforms (verified, 2026-08-16)

- macOS has no public brightness API; private CoreDisplay SPI (`CoreDisplay_Display_SetUserBrightness`/`GetUserBrightness`) is confirmed exported on macOS 26 but churns per release and must be dlopen/dlsym-guarded.
- macOS DDC exists only through the private IOKit AVService I2C path (`IOAVServiceCreateWithService`/`ReadI2C`/`WriteI2C`), absent from SDK stubs, discovered at runtime, with no TCC permission required.
- macOS phase 1 is gamma-only via public `CGSetDisplayTransferByTable` with exact read-back; DDC and CoreDisplay SPI are phase-2, separately reviewed.
- `CGDisplayCreateUUIDFromDisplayID` is gone on macOS 26, so EDID-hash identity (private `IODisplayCreateInfoDictionary` or `CopyDisplayInfo`) becomes the primary key with the unstable-identity flag retained.
- Windows HLMC (`GetMonitorBrightness`/`SetMonitorBrightness`, dxva2.dll) is public, per-monitor, DDC/CI-gated by `GetMonitorCapabilities`, and fails honestly on non-DDC panels, OSD-locked settings (`ERROR_DISABLED_MONITOR_SETTING`), and most DP/MST links.
- Windows gamma via `SetDeviceGammaRamp` is legacy, hardware-dependent, and can return success without effect, so every write needs verified read-back.
- WMI brightness exists only for internal panels; `Win32_DesktopMonitor` carries no brightness, so external monitors on Windows resolve to DDC-or-gamma per monitor.
- Both platforms share the phase-1 contract: gamma is the safe fallback, DDC capability is a per-display probe result, and every typed stub returns a typed error rather than a silent no-op.
