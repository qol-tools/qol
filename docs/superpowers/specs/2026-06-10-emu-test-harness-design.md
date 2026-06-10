# emu: Cross-Platform Capability Test Harness - Design

## Status

Design approved 2026-06-10, amended in review the same day: medium promoted from a
single mechanism to an injector strategy, with the smartphone as the flagship carrier.
This records the architecture and scopes Milestone 1 as the next implementation plan.
Items marked "(default, redline in review)" are my calls on forks that were left open
during brainstorming.

M1 implemented 2026-06-10. M2 control surface implemented 2026-06-10
(shot/key/insert/pull/snap/down, plan `2026-06-10-emu-m2-control-surface.md`);
the hvf/whpx finish remains as its own follow-up plan.

## Purpose

qol-tools promises: walk up to any machine, inject qol-tray, it becomes yours, pull the
medium, the host is left exactly as found. Nothing proves that promise per-OS today.

emu is a developer test harness that boots a clean throwaway VM of a target OS, injects
qol, exercises it, and yields a machine-readable verdict on a proposition (for example:
"does the OS retain ANY qol data after boot, insert, use, and reboot?"). It lives in
`tools/qol-cli`, is surfaced interactively in `qol dev`, and is also usable headless. It
never ships to end users; the mission non-negotiables are what emu tests, not constraints
that bind emu itself.

## Core abstraction: capability x platform -> verdict

- A **Workflow** is a named, platform-blind proposition. Written once.
- A **Platform** is a target OS (plus an optional desktop environment), realized by a VM.
  Written once.
- A **Medium** is an injection mechanism (USB stick, phone delivery). Written once.
- You author W workflows + P platforms + M media, never W x P x M. The cells of the grid
  (results) are produced, not authored.
- "Supported platforms" is derived, never hand-maintained: a platform supports a workflow
  when its adapter provides every verb that workflow requires.

## The contract

```rust
// Newtypes to define: Trace, DiskSnapshot, Hotkey, Frame, Input, Verdict.

// MEDIUM: how qol gets into the guest. A strategy, not a single thing.
// attach/detach semantics per variant: USB hot-plug for UsbStick and PhoneMtp,
// start/stop serving for NetworkServe.
pub enum Medium {
    UsbStick { image: PathBuf },        // QMP device_add usb-storage
    PhoneMtp { rootdir: PathBuf },      // QMP device_add usb-mtp (Android file-drop)
    NetworkServe { payload: PathBuf },  // host serves; guest pulls from 10.0.2.2
}

// SUBSTRATE: one running emulated machine. Default impl = QEMU. Host-side.
pub trait Machine {
    fn boot(&mut self) -> Result<()>;
    fn reboot(&mut self) -> Result<()>;
    fn attach_medium(&mut self, m: &Medium) -> Result<()>;   // "insert"
    fn detach_medium(&mut self, m: &Medium) -> Result<()>;   // "pull"
    fn snapshot(&self) -> Result<DiskSnapshot>;              // host reads the qcow2
    fn send_input(&mut self, i: Input) -> Result<()>;        // keys/mouse over QMP
    fn screenshot(&self) -> Result<Frame>;                   // QMP screendump
}

// GUEST OS layer: DE-blind guest knowledge. One impl per OS family.
pub trait GuestOs {
    fn launch_qol(&self, m: &mut dyn Machine, medium: &Medium) -> Result<()>;
    fn list_qol_traces(&self, snap: &DiskSnapshot) -> Result<Vec<Trace>>;
    fn is_qol_running(&self, m: &dyn Machine) -> Result<bool>;
}

// DE/UI layer: only UI-bound observations. One impl per desktop environment.
pub trait DesktopEnv {
    fn hotkey_fired(&self, m: &dyn Machine, combo: Hotkey) -> Result<bool>;
    fn tray_icon_visible(&self, m: &dyn Machine) -> Result<bool>;
    fn window_focused(&self, m: &dyn Machine, app: &str) -> Result<bool>;
}

// PLATFORM = OS composed with optional DE.
pub struct Platform { pub os: Box<dyn GuestOs>, pub de: Option<Box<dyn DesktopEnv>> }

#[derive(Clone, Copy)]
pub enum Verb {
    Boot, Reboot, Insert, Pull, LaunchQol, Use, ListTraces, IsRunning,
    HotkeyFired, TrayVisible, WindowFocused,
}

impl Platform {
    fn provides(&self, v: Verb) -> bool {
        match v {
            Verb::Boot | Verb::Reboot | Verb::Insert | Verb::Pull | Verb::Use => true, // substrate
            Verb::LaunchQol | Verb::ListTraces | Verb::IsRunning => true,              // GuestOs
            Verb::HotkeyFired | Verb::TrayVisible | Verb::WindowFocused => self.de.is_some(),
        }
    }
    pub fn supports(&self, wf: &dyn Workflow) -> bool {
        wf.required_verbs().iter().all(|v| self.provides(*v))
    }
}

// RUN: the only vocabulary a workflow ever sees. Never QEMU, never an OS name.
pub struct Run<'a> { machine: &'a mut dyn Machine, platform: &'a Platform, medium: Medium }
impl Run<'_> {
    fn boot(&mut self) -> Result<()> { self.machine.boot() }
    fn insert(&mut self) -> Result<()> { self.machine.attach_medium(&self.medium) }
    fn launch_qol(&mut self) -> Result<()> { self.platform.os.launch_qol(self.machine, &self.medium) }
    fn pull(&mut self) -> Result<()> { self.machine.detach_medium(&self.medium) }
    fn reboot(&mut self) -> Result<()> { self.machine.reboot() }
    fn list_traces(&self) -> Result<Vec<Trace>> { self.platform.os.list_qol_traces(&self.machine.snapshot()?) }
    // DE verbs error with CapabilityMissing when platform.de is None.
}

// WORKFLOW: platform-blind. Composes verbs, returns a Verdict.
pub trait Workflow {
    fn id(&self) -> &str;
    fn required_verbs(&self) -> &[Verb];   // drives Platform::supports
    fn run(&self, run: &mut Run) -> Result<Verdict>;
}

struct LeavesNoTrace;
impl Workflow for LeavesNoTrace {
    fn id(&self) -> &str { "leaves-no-trace" }
    fn required_verbs(&self) -> &[Verb] {
        &[Verb::Boot, Verb::Insert, Verb::LaunchQol, Verb::Use, Verb::Pull, Verb::Reboot, Verb::ListTraces]
        // No DE verbs, so every OS platform supports it, GNOME or Cinnamon alike.
    }
    fn run(&self, r: &mut Run) -> Result<Verdict> {
        r.boot()?; r.insert()?; r.launch_qol()?; /* use qol briefly */ r.pull()?; r.reboot()?;
        Ok(Verdict::from_traces(r.list_traces()?))   // empty => pass
    }
}
```

## Layering

```
Scenario / Grid       compose workflows x platforms  ->  report.json verdict
   |
Workflow              platform-blind proposition (LeavesNoTrace, CoreWorks, ...)
   |
Run (verb facade)     the only vocabulary a workflow sees
   |
Platform = OS (+DE)   GuestOs adapter, optional DesktopEnv adapter
   |
Machine (substrate)   QEMU by default; a trait, so other engines slot in
```

"Platform" has two axes: OS (filesystem, launch, where traces live) and DE/UI (hotkey
owner, tray, window focus). Most verbs, and the whole `leaves-no-trace` workflow, are
OS-level and DE-blind. Only UI-bound verbs need a `DesktopEnv`, and the DE set is bounded
by the desktops qol-tray itself supports. A DE with no observer can fall back to
screenshot comparison.

## Substrate is a strategy

- `Machine` is a trait. **QEMU is the default impl** and the only universal one:
  cross-host (Linux / macOS / Windows), cross-guest (widest arch range), uniformly
  controllable through QMP (one protocol on every host), free, and disposable via qcow2
  overlays.
- **Cheapest-honest-substrate rule:** a workflow's required verbs choose the substrate.
  Container / microVM (Firecracker) / Apple Virtualization.framework are deferred,
  specialized impls. They cannot be chosen for verbs they cannot honestly provide (a
  container has no real reboot, no real desktop, no foreign OS), so a serious proposition
  can never silently run on a too-weak substrate.
- **Per-host acceleration:** kvm (Linux), hvf (macOS), whpx (Windows), tcg fallback. The
  real performance constraint is guest-arch vs host-arch, not host OS (an x86 guest on
  Apple Silicon is emulated and slow).
- **macOS guests are Mac-host-only** (Apple's license permits macOS only on Apple
  hardware). The macOS column stays blank on Linux / Windows hosts; covering it needs a
  Mac in the loop.

## Medium is a strategy (the flagship carrier is a phone)

Asserted in review: the most ubiquitous carrier is a smartphone, so phone delivery is the
flagship target. A phone cannot impersonate a USB stick - modern Android exposes MTP only
(the mass-storage gadget needs root) and iPhone exposes no filesystem at all - so the
medium is a strategy:

```
medium        real-world shape                           emu emulation (zero physical phones)
UsbStick      a real USB stick                           QMP device_add usb-storage
PhoneMtp      Android file-drop over MTP, run from host  QMP device_add usb-mtp
NetworkServe  phone tethers/hotspots; host runs one      host serves the payload; guest
              bootstrap action, pulls binary + synced    pulls from 10.0.2.2 over user-net
              profile, runs
```

- Rooted-phone mass-storage gadgets (DriveDroid-style) are rejected: root kills the
  ubiquity argument.
- `NetworkServe` is the flagship because it is the only shape every phone (including
  iPhone) supports, the profile stays on or near the phone, and "pull the medium" becomes
  "walk away with your phone".
- Its no-trace reality is harder, not softer: Windows cannot exec a binary from a pipe,
  so the payload lands in a temp dir and cleanup is active deletion, not never-written.
  Bootstrap residue appears too (shell history, browser downloads, Run MRU). These are
  exactly the trace species `list_qol_traces` must learn, per medium.
- Phone app tiers for `NetworkServe`: none (phone is tether + identity, host pulls from
  cloud sync), PWA (holds token, shows QR and status; no app store; cannot listen on a
  socket, so never a LAN file server), native app (serves binary + profile over its own
  hotspot; the only fully-offline tier). Default: PWA tier; go native only if fully
  offline matters.
- Medium is a run parameter: the same workflow runs per medium, and grid cells extend to
  (workflow, platform, medium).

## Decisions made (default, redline in review)

- **Medium = injector strategy** (`UsbStick | PhoneMtp | NetworkServe`). `UsbStick` is
  the first implemented injector (simplest: QMP `device_add` / `device_del`);
  `NetworkServe` is the flagship target. Shared folders (9p / virtiofs) stay rejected:
  unfaithful to any real medium and need guest drivers.
- **Trace check = host-side read of the qcow2 overlay**, no guest agent. Start with
  known-location checks per OS; whole-disk diff (snapshot before/after, subtract OS
  baseline noise) is a later enhancement. Both are sealed inside `GuestOs::list_qol_traces`.
- **Default `Machine` = QEMU.** Native engines (Apple vz, Hyper-V) are deferred behind the
  trait, added only if QEMU underperforms a real matrix.

## How it fits the code today

In `tools/qol-cli/src/commands/emu/`:

```
LAYER                  CONTRACT                        TODAY                              STATUS
Surface / Grid         qol emu run <wf> <platform>     list/doctor/up; emu row in qol dev partial
Workflow               trait Workflow + required_verbs (none)                             greenfield
Run (verb facade)      Run{ boot, insert, ... }        (none)                             greenfield
Platform = OS (+DE)    GuestOs / DesktopEnv traits     Environment knows image, not verbs greenfield
Discovery              find candidate machines         discovery/{config,libvirt,fs}      done
Substrate: Machine     boot/reboot/QMP/snapshot/USB    up builds overlay + qemu command,  half
                                                       then stops before booting
Host probe             accel / uris / search roots     platform/{linux,macos,windows}     done (but HOST)
Artifact               Verdict -> report.json          report_json(ReportInput)           done (extend)
```

Naming: the existing `platform/` module is HOST detection (where is KVM, where are images
on this machine). The new guest adapters belong under a separate `guest/` tree
(`guest/os/`, `guest/de/`) so the word "platform" does not collide. The `macos.rs` and
`windows.rs` host-probe `acceleration()` functions currently return `tcg` and should
return `hvf` / `whpx`.

## Milestones

- **M1 (next implementation plan): Launch.** `qol emu up <id>` boots the prepared overlay
  via QEMU with per-host acceleration, opens a QMP socket, and the VM is visible on screen.
  Teardown discards the overlay, so no-trace holds by construction. This turns today's
  "prepare a command" into "a running, controllable VM".
- **M2: Control surface.** QMP-driven snapshot, screendump, sendkey, and USB attach /
  detach (the `Machine` half of `insert` / `pull` / `use`). Finish hvf and whpx.
- **M3: First cell end-to-end.** One `GuestOs` adapter (a single Linux distro), the `Run`
  verb facade, and the `leaves-no-trace` workflow producing a verdict in `report.json`.
  Proves the contract on one cell.
- **M4: Grid + qol dev.** The workflows x platforms grid, the interactive emu menu in
  `qol dev`, and headless `qol emu run <wf> <platform>`.
- **M5 and beyond:** more platforms; more media (`PhoneMtp`, `NetworkServe` with its
  bootstrap-residue traces); `DesktopEnv` adapters and UI-bound workflows; container /
  microVM / vz substrates.

## Non-goals (for now)

- Shipping emu to end users.
- The phone-side delivery surface itself (PWA, bootstrap endpoint): product work, not
  part of the emu harness.
- DE-bound workflows (hotkey / tray / window) before M5.
- macOS guests on non-Apple hardware.
