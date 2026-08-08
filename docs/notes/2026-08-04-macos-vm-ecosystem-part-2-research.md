# Temporary research note: macOS VM ecosystem, part 2

Status: research note; not yet an implementation plan.

This note captures the current direction for adding a native macOS guest to
qol's disposable environment system. It is grounded in the current owners in
the repository and in Apple's public Virtualization framework documentation.
The next implementation pass should turn the decisions below into contracts,
tests, and a small vertical slice.

## Executive conclusion

The strategy scaffold in `tools/qol-cli/src/commands/emu/strategy/` is a useful
selection seam, but it is not yet a backend contract. The next step should be
to make lifecycle, artifacts, control, and cleanup backend-neutral before
implementing Apple-specific VM calls.

The recommended sequence is:

1. Introduce a backend-neutral `MachinePlan`, with QEMU and Apple
   Virtualization launch plans behind it.
2. Introduce a `GuestArtifact` contract that can represent both a QEMU image
   and a macOS VM bundle.
3. Generalize machine identity, endpoints, lifecycle observation, and cleanup
   evidence so reports do not require QEMU fields.
4. Add Apple host preflight and doctor output.
5. Implement one real Apple Silicon start/stop/cleanup lane with no desktop
   workflow, workspace mount, or guest network.
6. Only then add macOS guest control and desktop automation.

## Current qol seam and remaining coupling

The landed scaffold provides:

- `MachineBackend::{Qemu, AppleVirtualization}`;
- `MachineStrategy` selection;
- separate guest and desktop strategy enums;
- an explicit unsupported result for Apple Virtualization.

The following surfaces remain QEMU-shaped and should not become the Apple
backend's interface:

- `BackendSpec`, `ReadyBackend`, `Resolution`, and `BootedVm` in
  `tools/qol-cli/src/commands/emu/mod.rs`;
- QMP, serial, and QEMU guest-control ports in
  `tools/qol-cli/src/commands/emu/machine.rs`;
- QEMU process and QMP assumptions in `tools/qol-cli/src/commands/emu/live.rs`;
- qcow2 and `qemu-img` assumptions in
  `tools/qol-cli/src/commands/emu/image_import/`;
- QEMU-specific cleanup fields in `libs/qol-dev-env/src/report.rs`;
- Linux session assumptions in `libs/qol-dev-guest/src/lib.rs` and the Linux
  implementation of `tools/qol-guest-runner`.

The target shape is:

```text
EnvironmentPlan
  -> MachinePlan
       -> QemuLaunchSpec
       -> AppleVirtualizationSpec
  -> GuestArtifact
       -> verified qcow2 file
       -> verified macOS VM bundle tree
  -> MachineHandle
       -> backend-neutral identity, endpoints, lifecycle, cleanup proof
  -> GuestControlTransport
       -> TCP / QEMU transport
       -> VZ socket or another Apple-specific transport
  -> DesktopControl
       -> exec, keyboard, pointer, screenshot, desktop evidence
```

QEMU should remain an implementation of those contracts. Apple Virtualization
should not manufacture fake QMP ports or inherit QEMU image mechanics.

## Apple Virtualization lifecycle

Apple's documented macOS guest path is:

```text
local .ipsw
  -> compatible VZMacPlatformConfiguration
  -> VZMacOSInstaller
  -> persistent VM bundle
  -> VZVirtualMachine start/stop
```

The restore image is an installation input, not the runtime image. Apple's
sample VM bundle contains the main disk image, auxiliary storage, machine
identifier, hardware model, and optionally the restore image. See:

- [Installing macOS on a Virtual Machine](https://developer.apple.com/documentation/virtualization/installing-macos-on-a-virtual-machine)
- [Running macOS in a virtual machine on Apple silicon](https://developer.apple.com/documentation/virtualization/running-macos-in-a-virtual-machine-on-apple-silicon)
- [VZMacPlatformConfiguration](https://developer.apple.com/documentation/virtualization/vzmacplatformconfiguration)

When a VM is reloaded, the original hardware model, machine identifier, and
auxiliary storage must be restored. Concurrent lanes need distinct auxiliary
storage and machine identifiers. This means the artifact identity is a
content-addressed tree and metadata, not a single image-file SHA.

The prepared template should be immutable. Each run should materialize
disposable state without mutating the verified template. The exact cloning
mechanism—copy, APFS clone, or another supported operation—needs a focused
implementation spike and must be recorded in the artifact report.

## Entitlements and native helper boundary

Any process using Virtualization.framework requires the
`com.apple.security.virtualization` entitlement. Keep this behind a dedicated
macOS helper rather than making the cross-platform `qol` CLI itself the native
VM process:

- the helper owns VZ objects, the AppKit/Swift/Objective-C runtime boundary,
  and entitlement-aware signing;
- `qol` owns environment resolution, resource admission, reports, and the
  helper's lifecycle;
- the helper speaks a small versioned local protocol using structured argv or
  a private local socket;
- non-macOS builds retain a typed unsupported implementation.

See [Adding the Virtualization Entitlement to Your Project](https://developer.apple.com/documentation/virtualization/adding-the-virtualization-entitlement-to-your-project).

The repository already uses `objc2` and small Swift helpers on macOS. The
`objc2-virtualization` crate is a possible direct Rust binding, but it should
still remain inside the macOS helper boundary:

- [objc2-virtualization](https://docs.rs/objc2-virtualization/0.3.2)

Local development may use an explicitly signed or ad-hoc-signed helper. The
distribution and release signing policy is a separate decision and should not
be silently inferred from local development behavior.

## Artifact and cache model

Keep these identities separate:

| Artifact | Role | Suggested identity |
| --- | --- | --- |
| `.ipsw` | Installation input | Apple build/version, file digest, source URL, schema |
| Prepared VM template | Immutable runtime base | Canonical member-tree digest, hardware-model digest, guest revision |
| Per-run VM state | Disposable execution state | Run id, lane id, unique machine identifier, owned paths |
| Development payload | Read-only qol runtime bundle | Existing payload manifest and digest contract |

Do not route a VM bundle through the qcow2 importer. Add a sibling artifact
preparation path that verifies every required bundle member, records the guest
build and hardware compatibility, and only publishes a template after a real
boot/cleanup verification.

Do not download `latestSupported` during ordinary `env up`. Fetch or accept a
pinned local restore image during an explicit preparation step, then reuse the
verified revision. Keep restore images cached separately from prepared VM
templates.

## Guest control and desktop control

The current `qol-dev-guest` framing and request actions are reusable, but the
transport and identity model are not fully generic:

- `GuestControlClient` is hard-coded to `TcpStream`;
- the default run identity comes from QEMU fw_cfg;
- `GuestHello` requires Linux-oriented D-Bus, display, and session fields;
- `qol-guest-runner` has an explicit unsupported macOS implementation.

The shared protocol should retain the common identity and process operations,
while moving transport and session details behind capabilities:

```text
required: environment, artifact revision, run identity, guest user, runner identity
optional: desktop, display, session type, D-Bus/XDG metadata, platform metadata
actions: ping, exec, spawn, wait, terminate
```

Apple's [`VZVirtioSocketDevice`](https://developer.apple.com/documentation/virtualization/vzvirtiosocketdevice)
is the leading transport candidate for a host to reach a guest-side runner,
but it needs a real macOS guest spike before being made a contract. Do not
force this path to look like localhost TCP or a virtio-serial device if the
framework exposes a different identity model.

Separate machine control from desktop control:

- `MachineControl`: start, stop, request-stop, state, identity, recovery;
- `DesktopControl`: guest exec, keyboard, pointer, screenshot, and desktop
  evidence.

This lets the first Apple slice prove lifecycle without pretending that desktop
automation is ready.

## The headless input and screenshot risk

Apple documents `VZVirtualMachineView` as an `NSView` that displays the VM
framebuffer and forwards keyboard and pointing-device events. It also exposes
system-key capture. Apple's public documentation does not describe a QMP-like
headless screenshot API or direct programmatic key/pointer injection API:

- [VZVirtualMachineView](https://developer.apple.com/documentation/virtualization/vzvirtualmachineview)
- [Graphics](https://developer.apple.com/documentation/virtualization/graphics)
- [Keyboard and pointing-device configuration](https://developer.apple.com/documentation/virtualization/keyboards-and-pointing-devices)

This is the highest-risk open spike for the alt-tab workflow. Before adding
`shot`, `key`, or `drag` support for macOS guests, prove one of these paths
without private APIs or host-session input injection:

1. an explicitly supported offscreen/native view driver;
2. a guest-side control and capture service that remains inside the guest;
3. another public Virtualization capability that supplies equivalent evidence.

Until then, macOS should expose lifecycle and readiness states but keep desktop
automation visibly unsupported.

## Alt-tab implication

The current source suggests the original `w` versus `q` asymmetry is likely in
the macOS accessibility close action rather than in key routing:

- `w` attempts to find and press `AXCloseButton`;
- `q` terminates the selected application directly;
- a missing close button and an accessibility action failure can collapse into
  an unsupported/false result.

That is a source-based inference, not a runtime reproduction. The eventual
guest workflow should preserve decision evidence rather than treating a failed
close as success:

```text
KEY_RECV key=w
CLOSE_WINDOW result=closed|unsupported|failed
postcondition: window count changed or explicit failure

KEY_RECV key=q
QUIT_APP result=terminated|failed
postcondition: selected process/app state changed
```

The guest bridge must deliver input and collect evidence; it must never invoke
host activation, host window enumeration, or host input APIs.

## First vertical slice

The first implementation should be intentionally boring:

1. Add backend-neutral artifact and machine-plan types.
2. Add Apple host preflight and typed `ready`, `missing`, and `unsupported`
   outcomes.
3. Add a signed helper that can validate a VM bundle and report its identity.
4. Start one prepared VM on an Apple Silicon host.
5. Record VZ state transitions, helper identity, bundle identity, and guest
   stop/cleanup proof.
6. Stop the VM and prove all owned processes and disposable artifacts are
   terminal before releasing resources.

No desktop workflow, workspace mount, or guest network is required for this
slice. The existing qol environment engine, report family, resource ledger,
and cleanup rules remain the owners; do not create a second registry or
reporting path.

## Verification and rollout

Linux and ordinary hosted macOS CI can cover:

- artifact manifest parsing and tree hashing;
- strategy selection and unsupported-host diagnostics;
- fake machine lifecycle and failure injection;
- report and cleanup state machines;
- cross-platform compilation and linting;
- helper entitlement inspection where the runner supports it.

Real macOS guest coverage requires a dedicated Apple Silicon host with a
prepared artifact, a signed helper, and the matching host/guest versions. It
should begin as a manual or self-hosted integration lane, not a required
hosted-CI job.

The `qol dev` surface should only advertise a macOS desktop workflow after the
guest control, display/input, and cleanup contracts have passed one real lane.

## Decisions still open

- exact minimum host macOS version and pinned guest build;
- Rust `objc2-virtualization` versus Swift/Objective-C helper implementation;
- local ad-hoc signing versus release Developer ID signing;
- bundle clone/materialization strategy for parallel lanes;
- VZ socket versus another guest-control transport;
- supported headless screenshot and input mechanism;
- whether Apple's beta `VZMacGuestProvisioningOptions` is ever acceptable.

The provisioning API should not be foundational while it remains beta and is
limited to newer guest versions. See [VZMacGuestProvisioningOptions](https://developer.apple.com/documentation/virtualization/vzmacguestprovisioningoptions).

## Repository owners to extend

- `libs/qol-dev-env/`: artifact identity, registry, report, resource admission;
- `libs/qol-dev-guest/`: transport-neutral guest protocol and identity;
- `tools/qol-cli/src/commands/emu/strategy/`: machine and guest strategy
  facades;
- `tools/qol-cli/src/commands/emu/`: orchestration only;
- `tools/qol-cli/src/commands/emu/image_import/`: QEMU importer plus a
  separate VM-bundle preparation path;
- `tools/qol-guest-runner/`: macOS guest-side runner implementation;
- `tools/qol-cli/src/commands/emu/workflow/`: capability-specific desktop
  workflows;
- `.github/workflows/`: compile/fake coverage and an optional self-hosted
  Apple Silicon lane.
