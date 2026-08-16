# Monitor plugin bootstrap — phase 1 plan (parallel lanes)

Status: A-lanes LANDED (A1a 10c4c1a8c, A1b dd3450ed8, A2a 6ae266d69, A2b 30d877053, A2c f837321ae); D board COMPLETE (7 lanes, conditional, 0 blockers — fix-forward 81814056c + 62c60cb65); E BLOCKED on this host (needs Linux/KVM guest host); B/F landed (3e3caa800, 4a1803c6c, b8fe0dfec).
Parent: `2026-08-16-monitor-library-readiness.md` — LANDED (all 7 steps verified in tree).
Spec: `docs/specs/2026-08-16-monitor-control.md` (capability contract, edge-case policy current).
Plugin: unnamed by user decision. Branch topics short, no PID prefix. All lanes flash-tier, background:true, worktree route, squash-to-one-commit on main.

## Goal

Phase-1 vertical slice: DDC/CI brightness on Linux via the hardened grant, gamma fallback, host-claimed hotkeys, failures visible. macOS/Windows brightness APIs researched, not implemented. Real-hardware DDC round-trip is a named post-phase-1 manual check (not guest-provable: virtio-gpu has no i2c client behind /dev/i2c-N).

## Lane map

| Lane | Owner | Crate/dir | Inputs | Outputs | Depends on |
|---|---|---|---|---|---|
| A1a | plugin skeleton | new plugin crate | template-fixed main, spec, plugin-contract.md | plugin.toml contract, lib.rs facade with typed stubs (list_modes/set_mode/get_hdr/set_hdr), CLI commands, manifest test | — |
| A1b | probe + grant + doctor | plugin crate | A1a | i2c probe tiers, grant wiring (pins live udev API), doctor wiring (CLI doctor, qol doctor integration, tray visibility), EDID identity | A1a |
| A2a | DDC backend | plugin crate | A1b | connector→/dev/i2c-N resolution (sysfs i2c link), i2c_ddc read/write, read-back verify, settle, one retry, downgrade; fake-transport unit tests | A1b |
| A2b | gamma fallback | plugin crate | A2a | gamma LUT write + checksum-guarded re-assert restore, mismatch counter, auto\|ddc\|gamma\|off policy, source labels, transport probe, MST/DisplayLink typed-unsupported | A2a |
| A2c | hotkeys + restore + config | plugin crate | A2b | hotkey continuous actions + debounce + toast, session restore precedence (crash → preferred → exit), device-scope config | A2b |
| B | sandbox macOS fix | apps/qol-tray | policy.rs:208 break at HEAD | compile fix + gate | — |
| C | research: macOS/Windows brightness | spec (architect-owned) | kernel/API docs | verdict notes appended to spec phase-2 section | — |
| D | review board | plugin crate | A2c | pass/conditional/block verdict (7 lanes) | A2c |
| E | guest-VM verify | plugin crate | A2c | probe/grant/gamma/hotkeys/restore proven in guest; DDC round-trip via i2c-stub or real hardware | D |
| F | deferred single-source items | libs/qol-host-fixes (+ apps/qol-tray if M5 home lands there) | review report | M5 restore-ordering decision (home: wherever enforced; if tray gpu_driver_sync feature, sequence after B), lock squatting (SO_PEERCRED or stale-detecting fs lock), UdevUaccess arm out of nvidia::rendered_hash_of | B if tray home |

## Lane cards

### A1a — template bootstrap + contract + facade

- Bootstrap from fixed template (lib.rs facade, thin main.rs, manifest test).
- plugin.toml contract identity, `continuous = true` (schema confirmed); no name decision yet.
- Facade ships typed stubs today: list_modes/set_mode/get_hdr/set_hdr (spec contract), plus brightness + gamma.
- CLI commands (per template pattern, ~7), headless doctor skeleton.
- Gate + commit; report.

### A1b — probe tiers + grant wiring + doctor

- i2c probe tiers: EACCES (no grant) / ENOENT (i2c-dev not loaded) / EBUSY (ddcci-driver conflict), per qol-headless probe.
- Grant wiring pins the LIVE udev API (qol-host-fixes/src/udev/mod.rs: RuleConflict/Busy/resume, rule-loss re-apply, operator-file sha256 refusal) — verify, don't renegotiate. F only relocates the nvidia arm later.
- EDID identity via qol-windowing DisplayHandle; `identity_unstable` refused for config binding.
- Doctor wiring named on all three surfaces: plugin CLI doctor (headless), qol doctor integration, tray visibility. Spec: "visible in doctor and UI".
- Gate + commit; report.

### A2a — DDC backend

- connector→/dev/i2c-N resolution via the sysfs i2c link (heart of Linux DDC; unnamed in first revision).
- i2c_ddc read/write brightness, read-back verify after settle, one retry, source downgrade on unchanged read-back.
- Fake-transport unit tests; no real-monitor dependency.
- Gate + commit; report.

### A2b — gamma fallback

- Gamma LUT write, read-back verify; checksum-guarded re-assert on restore (only when LUT checksum matches what the plugin wrote).
- Mismatch counter, warning at 3, per-display opt-out; auto|ddc|gamma|off policy; source-labeled values.
- Transport probe: MST/DisplayLink EIO → typed unsupported + gamma.
- Gate + commit; report.

### A2c — hotkeys + restore + config

- Hotkeys host-claimed: ~5 % step, ~70 ms debounce, toast with value + source; config-level chord-collision check (duplicate_enabled_chord) + registration status in doctor.
- Session restore precedence: crash-restore → preferred → exit, single idempotent restore path; device-scope config via device_local_dir.
- Gate + commit; report.

### B — qol-tray sandbox macOS compile break

- `apps/qol-tray/src/features/gpu_driver_sync/policy.rs:208` calls Linux-gated `serialized_env_tests` un-gated; confirmed at HEAD.
- Fix gating, full gate, commit, report.

### C — macOS/Windows brightness research

- macOS: CoreDisplay private API status, DDC on macOS, permission model. Windows: GetMonitorBrightness/SetMonitorBrightness, gamma ramp.
- Output: verdict notes appended to spec (architect-owned; lane reports, architect applies), feed phase 2.

### D — review board on A2c

- 7 lanes: security, correctness, architecture, tests-qa, requirements, history + one adversarial. Identity folds into correctness/history; races into adversarial.
- Verdict gates A2c; fix-forward rounds until conditional/pass.

### E — guest-VM verification

- Packaging named: host debug build before `qol env up` (guest never compiles), plugin id required at the install boundary, ready-check verifies guest plugin ids automatically.
- Guest proves: probe tiers, grant/revoke live, gamma round-trip (Cinnamon X11 SetCrtcGamma + read-back), hotkeys + chord collision, all three restore paths (exit, disable, crash kill -9).
- DDC value round-trip: i2c-stub integration (image-prep modprobe step) or the named real-hardware manual check after phase 1 — never implied as guest-provable.
- Cost: host debug build 10-20 min + VM boot 2-5 min + scenarios 20-30 min.

### F — deferred single-source items

- M5: nvidia-vs-udev restore ordering decision — home is wherever restore ordering is enforced; if that is the tray gpu_driver_sync feature, F sequences after B.
- Lock squatting: SO_PEERCRED or stale-detecting fs lock.
- UdevUaccess arm moved out of `nvidia::rendered_hash_of`.
- Must land BEFORE the D board on A2c (unconditional; A2 may start DDC/gamma/hotkeys before F, but A2c's restore work does not finalize before F).
- Gate + commit; report.

## Dependency rules

- A1a → A1b → A2a → A2b → A2c sequential (same crate; strict).
- A2c → D → E strict sequential (review before guest spend).
- B, C parallel to the A chain; F parallel but must land before D; if F touches the tray feature, after B.
- C's spec edits are architect-owned: the lane reports, the architect applies.
- Docs (readiness status, spec) updated at each landing by the architect.

## Gates

- Per commit: fmt, clippy `--all-targets --all-features -D warnings` (host + linux where the crate touches platform code), build, test.
- FreeBSD check is qol-host-fixes-only (local `rustup target add x86_64-unknown-freebsd; cargo check --target`); NOT in the plugin's per-commit gate (no plugin freebsd code).
- Linux runtime tests: `scripts/container-test` runner (to be added — gate owner), rust:1.96-bookworm digest pinned to host toolchain 1.96.0, `--tmpfs /tmp:rw,exec` (noexec breaks the udev shim), base-baseline run recorded, failures diffed against base (machine-id/dpkg env failures are known).
- E requires a real guest, not a container.

## Acceptance (phase 1 done)

- Guest-verified: probe tiers, grant/revoke live, gamma round-trip, hotkeys + chord collision, restore proven for exit/disable/crash.
- Real-hardware DDC round-trip: named manual post-phase-1 check on the user's monitors (not guest-provable; "silent failure is a bug" cuts both ways).
- macOS/Windows: research verdicts in spec, no code claims.
- Review board conditional/pass on the A2c slice; all blockers fixed forward.
