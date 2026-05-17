# ARCHITECTURE-WASM · target architecture

Companion to `ARCHITECTURE.md`. Where `ARCHITECTURE.md` audits what qol-tray
**is today**, this doc describes what it **should become**: a WASM-first host
kernel with three runtime kinds, a host capability layer that mediates every
OS call, and host-rendered surface recipes that plugins request rather than
build themselves.

Sister diagrams live in `diagram/WASM Architecture.html`.

The target is not "WASM everywhere". It is **WASM-first with host capabilities
and rare native sidecars**. The mistake to avoid is also explicit (see "The
trap" below) because a prior WASM attempt fell into it.

---

## 0. Three states this doc keeps separate

| State | Status | What it is |
| --- | --- | --- |
| Current main | shipped, today | Native process plugins (daemon and runtime both optional in the manifest), per-OS binaries, Unix-only IPC asymmetry, no capability boundary. No `runtime.kind`. |
| Prior WASM branch | not on main, abandoned | A WASM attempt that absorbed launcher and alt-tab behavior into the host (the `host_ui.rs` trap). Source of the "don't do that" lesson. |
| Target (this doc) | proposed | WASM-first host kernel, three runtime kinds, host capabilities, host-rendered surfaces. |

Whenever a section reads as "today", it means current main. The prior WASM
branch is referenced only in Section 2 (the trap), and is never mixed into
"today".

---

## 1. Why this exists (current main)

Current main has four problems that compound:

1. **Per-plugin native process paths.** Current plugins execute through
   native process paths. Manifest has both `daemon` and `runtime` as
   `Option` in `src/plugins/manifest/schema.rs`, so a plugin can be
   daemon-only (long-lived native process with its own Unix socket and
   supervisor entry), runtime-only (one-shot subprocess per action), or
   both. Daemon-enabled plugins multiply: N enabled daemon plugins means
   N processes, N sockets, N supervision states.
2. **Per-OS plugin builds.** A plugin ships a native binary per OS. Adding
   a plugin is N × 3 cross-compile work, not 1. Plugin authors have to learn
   each platform's gotchas.
3. **No capability boundary.** A plugin process can do anything the user can
   do. There is no way to grant "this plugin may read the clipboard but not
   the network", because there is no host-mediated layer to grant against.
4. **Per-OS IPC asymmetry.** The daemon-socket path and the runtime state
   socket are Unix-only. Windows runs only the ephemeral-spawn path. The
   architecture itself is uneven across OSes.

None of these are about UI baked into the host. Current main does not have
plugin UI baked into the host. That was a separate, abandoned attempt -
see Section 2.

---

## 2. The trap to avoid (from the prior WASM branch)

A prior WASM branch (not on main) tried to delete native plugin processes
and absorb plugin behavior into the host. The result: `host_ui.rs` grew
frecency, file mode, navigation momentum, boost badges, GPUI window reuse,
focus races. The host became a pile of pasted plugin apps. The TRAY-54
audit confirms that the more launcher parity was chased, the more
`host_ui.rs` looked like the launcher itself, just rehoused.

The lesson: **moving plugin UI code into the host is not the same as making
plugins guests.** A real guest can't reach for `gpui` directly. A real guest
ships data and policy, and the host renders.

The prior branch's `host_ui.rs` is not on main and is not the starting point
for the target. Reusable pieces can be salvaged after being split into
proper surface modules, but the file is not imported as-is.

---

## 3. The target shape

Five rules.

### 3.1 qol-tray is the host kernel

- Owns: tray, hotkeys, profile / config, plugin registry, capability grants,
  runtime state, platform adapters.
- No plugin owns OS integration directly unless it is a sidecar-class plugin
  (see 3.5).

### 3.2 Most plugins are WASM components

- Each plugin is one `.wasm` (component model), a manifest, optional assets,
  and a typed config schema.
- No per-OS binary artifact. The `.wasm` is the artifact.
- No daemon socket. The host loads the component into a wasmtime engine and
  invokes exports.
- Install becomes: download, cache, register. No spawn ceremony.
- The action flow becomes: input → `action_executor` → `RuntimeRouter` → wasm
  invocation.

### 3.3 Host capabilities replace plugin syscalls

- A capability is a WIT interface (`window`, `apps`, `files`, `clipboard`,
  `process`, `capture`, `serial`, `network`, `settings`, `state`).
- The platform-specific implementation of each capability lives in a host
  adapter (`linux`, `macos`, `windows`).
- The host links only the imports the manifest grant approved. Imports the
  manifest did not request fail at link or instantiation; the plugin can
  never acquire them at runtime, because they were never wired in.
- No plugin sees `x11rb`, `objc2-app-kit`, or `windows-sys` directly.

### 3.4 UI is host-rendered surface recipes

- The host exposes a small set of stable surfaces: `picker.grid`,
  `switcher.strip`, `settings.form`, `notification`, `detail.panel`.
- A plugin provides **data, ranking policy, labels, actions, config**.
- The host owns **focus, window placement, keyboard routing, theming, window
  reuse, rendering**.
- Launcher becomes "WASM coordinator + `picker.grid`".
- Alt-tab becomes "WASM policy + `switcher.strip` backed by host capture".
- Adding a new picker-shaped plugin no longer means writing a new GPUI app.

### 3.5 Native sidecars exist, but only for hard cases

- A sidecar is a long-lived native process talking the same IPC contract the
  current daemon plugins use.
- Keep sidecars for: kernel input capture (keyremap), persistent device state
  (pointz), high-bandwidth I/O (parts of screen-recorder).
- Sidecars are the escape hatch, not the default. The default is WASM.

---

## 4. Runtime kinds

| Manifest `runtime.kind` | Rust impl | Examples | Reason |
| --- | --- | --- | --- |
| `wasm` | `WasmRuntime` | most plugins | sandbox, distribution, hot reload, language-agnostic |
| `sidecar` | `SidecarRuntime` | keyremap, pointz, screen-recorder backend | requires raw kernel input or persistent OS state the WASM sandbox can't usefully run |
| `builtin` | `HostBuiltinRuntime` | profile sync, updates, plugin store | first-class host features, no plugin overhead, ship with the host binary |
| `process` (transitional) | `ProcessRuntime` | every native daemon plugin not yet migrated | drops away once migration completes; the trait that keeps current main shipping during the transition |

All four implement a `PluginRuntime` trait. The `RuntimeRouter` dispatches on
manifest `runtime.kind`. Manifest values are lowercase strings; the Rust type
names are implementation detail.

---

## 5. Capability catalog

| Capability | Grants | Adapter coverage |
| --- | --- | --- |
| `window` | list, focus, move, close | linux (x11 / wayland), macos (NSWindow), windows (win32) |
| `apps` | launch, enumerate, focus | desktop entries · LSWorkspace · UWP / win32 |
| `files` | scoped read / write / watch | std::fs + filesystem-event APIs |
| `clipboard` | get / set / watch | arboard or platform |
| `process` | spawn / signal / kill, **allowlist declared in manifest, host enforces** | std::process |
| `capture` | desktop / audio / webcam | platform-specific |
| `serial` | open / read / write | serialport-rs |
| `network` | scoped HTTP only | reqwest with allowlist from manifest |
| `settings` | profile read / write | qol-tray profile feature |
| `state` | cursor, focus feed, displays | runtime/server.rs (existing) |

Grants are declared in the plugin manifest and confirmed at install time. A
grant is binary per capability today (granted or not); per-method scoping is a
later refinement.

---

## 6. Surface catalog

| Surface | Plugin gives | Host owns | Primary consumer |
| --- | --- | --- | --- |
| `picker.grid` | items, ranking, labels, actions | rendering, focus, keyboard, window reuse | launcher |
| `switcher.strip` | window list, sort policy, preview source | rendering, focus, keyboard, capture | alt-tab |
| `settings.form` | typed schema, validation hooks | rendering, persistence binding | every plugin |
| `notification` | text, level, action callback | toast vs OS notif decision, theming | most plugins |
| `detail.panel` (speculative) | item id, content recipe | layout, transitions | no committed first consumer yet; included as a placeholder for shapes that turn out to need an inspector-style side panel. Drop if it has no consumer by the time launcher and alt-tab are migrated. |

If a plugin wants a UI shape that no surface offers, the answer is to design
a new surface (and host-implement it), not to ship custom GPUI from a plugin.
Custom GPUI from a plugin is the failure mode this whole architecture
prevents.

---

## 7. Migration order

1. **Coexist.** Keep process plugins working. Add `runtime.kind` field to the
   manifest schema on main. **Missing `runtime.kind` defaults to `process`**
   so every existing manifest keeps loading unchanged and routes through
   `ProcessRuntime`. New plugins set `runtime.kind = "wasm"` (or `sidecar` /
   `builtin`) explicitly. No behavior change for existing plugins.
2. **Trait split.** Introduce `PluginRuntime` trait. Concrete impls:
   `WasmRuntime`, `SidecarRuntime`, `ProcessRuntime` (transitional). All
   today's daemon plugins go through `ProcessRuntime` until migrated.
3. **Engine in main, sidecar still ours.** Move the wasmtime engine from the
   experiment branch into qol-tray, behind a feature flag. Do not delete
   sidecar infrastructure yet.
4. **Don't import the prior `host_ui.rs` as-is.** The prior WASM branch's
   `host_ui.rs` is not on main; salvaging reusable pieces means splitting
   them into surface modules (`surfaces/picker.rs`, `surfaces/switcher.rs`,
   `surfaces/form.rs`, ...) FIRST, then porting only what fits the surface
   contract. The host never owns plugin-shaped code; it owns surface-shaped
   code. This is the step that keeps the TRAY-54 trap from recurring.
5. **Easy plugins first.** Migrate plugins with no UI and minimal OS surface
   to WASM: window-actions, simple command-style plugins, clipboard helpers.
   Each migration is one PR.
6. **Launcher migration.** Redesign launcher as "WASM coordinator returning
   items + ranking policy" plus the host's `picker.grid` surface. Frecency,
   file mode, boost badges become plugin policy expressed through the
   surface API, not host-pasted code.
7. **Alt-tab migration.** Redesign alt-tab as "WASM policy + host
   `switcher.strip`" backed by host capture and window capabilities. The
   preview pipeline becomes a host capability, not a plugin-owned daemon.
8. **Sidecars stay sidecars.** Keyremap, pointz, screen-recorder backend
   remain `SidecarRuntime` kind. Document the threshold: a plugin earns
   sidecar status only when WASM cannot meet a hard requirement.

---

## 8. Open questions

- **WIT versioning.** How do capability interfaces evolve without breaking
  shipped plugins? Probably semantic-versioned worlds with deprecation
  windows, but mechanism TBD.
- **AOT cache scope.** Per-host or per-user? Cache invalidation rules?
- **Grant UX.** Implicit from manifest at install, explicit per-install
  consent, or both? Plugin-store policy needs to decide.
- **Sidecar transport.** Same Unix-socket / ndjson contract the current
  daemon plugins use, or a new shared IPC layer alongside the WASM host
  imports?
- **HostBuiltin vs Sidecar boundary.** When does a feature graduate from
  sidecar to HostBuiltin? When does a HostBuiltin demote to sidecar to keep
  the host binary slim?
- **GPUI dependency in the host.** GPUI is in early stages. The surface
  layer is bet on GPUI continuing to be the right native renderer. If GPUI
  stalls, the surface layer is the contract that lets the renderer swap.

---

## 9. What this doc explicitly does NOT do

- Specify the exact WIT signatures for each capability (separate spec).
- Define the surface recipe DSL or its JSON / RON / WIT representation
  (separate design).
- Pin a specific wasmtime version or Component Model revision.
- Plan the cutover date, release strategy, or breaking-change schedule.
- Replace or supersede `ARCHITECTURE.md`. That doc remains the audit of
  current reality; this one is the target.
