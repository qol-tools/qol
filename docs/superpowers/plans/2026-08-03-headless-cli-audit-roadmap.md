# Headless-CLI coverage audit and roadmap to 100%

Date: 2026-08-03

Grand goal: every feature — including plugins — is a headless CLI tool at its
base. qol-tray orchestrates those tools; UI (gpui popups, web settings,
native overlays) is a strategy layered on top, never the home of the feature.

This document is the audit of where the monorepo stands against that goal and
the roadmap for closing the remaining gap. It was produced in the
`headless-cli-audit` worktree.

## What "headless at base" means (the invariant)

Per `qol-arch-code` ("Headless-first feature shape" + "Headless CLI contract"):

1. The unit is a standalone binary with a thin entrypoint (`main.rs` → `cli::exit_code`).
2. It speaks the `qol-headless` contract: `help`, `help <path>`, `<path> help`,
   `doctor`, `--json doctor`, `version`, stdout/stderr split, exit codes.
3. Domain logic lives in library/capability modules; adapters own argv,
   host-injected config, and presentation.
4. Host actions map to CLI argv (`catalog_runtime_args` in `plugin.toml`);
   qol-tray dispatches `binary <action-args>` or a daemon-socket message.
5. Daemon mode is an optional layer over the CLI, never a replacement.
6. UI is an adapter over the headless core, via the shared gpui surface kit
   (`qol-plugin-gpui-surfaces`) or host-served web UI.

## Method

The audit is regenerable: `docs/headless-cli-audit/audit.sh` walks every
release unit under `plugins/*` and `tools/*` plus the host `[[bin]]` targets,
and reports headless coverage per unit. It exits non-zero on any gap, so it is
also the seed of the Phase 3 gate. Run it from the repo root; `--json` emits
machine-readable records.

Per-unit criteria:
- plugin: manifest declares `[runtime]` command, `capabilities.doctor = true`,
  and a `qol-headless` CLI surface in source
- tool / host bin: depends on `qol-headless` and has a documented headless entry

## Findings

Snapshot at commit `057c461e` (regenerate with the script; the numbers below
are the script's output, not maintained prose).

### Standalone feature units: 21/21 headless (100%)

- **Plugins: 15/15.** Every directory under `plugins/*` with a `plugin.toml`
  declares `[runtime]`, `doctor = true`, maps actions to argv, and builds its
  CLI on `qol-headless`. 13/15 layer a `[daemon]` on top; the two without
  (`removeapp`, `template`) are on-demand CLI by design.
- **Tools: 2/2.** `qol` (dev, env, flow, sync, doctor front door, trace) and
  `qol-guest-runner` both implement the `qol-headless` contract.
- **Host auxiliary binaries: 3/3.** `qol-tray-install`, `qol-tray-doctor`,
  `qol-tray-migrate` are standalone headless CLIs built on `qol-headless`.
- **`qol-tray` itself: headless entry points + daemon.** With no args it is the
  daemon; with args it dispatches `doctor` (+ `--json`), `help`, `--version`,
  `exec <target> <action>`, `open <route>`, URL courier, and `--write-mode=`.
  Two internal headless modes exist for spawned work: the settings-surface
  boot route and the process-tree guardian entry (task execution supervision).
  Neither is user-facing.

Headless profile sync is already real: `qol sync` drives `qol-profile-sync`
without the tray (the `ecosystem-features.md` P0-1 entry calling it a gap is
stale). `qol doctor` is the headless front door onto the host's aggregate
doctor, which invokes every installed plugin's `doctor --json`.

### The only non-headless remainder: host-embedded features

Eight user-facing features live inside the qol-tray daemon with **no headless
command surface** — this is the entire gap to 100%:

| Feature | Capability today | Headless surface |
|---|---|---|
| plugin store | install / update / remove via host IPC + dev-links API | none |
| profile | export / import / backup in host (sync itself is headless via `qol sync`) | none |
| task runner | host-side execution; guardian entry is spawned-internal | none user-facing |
| theme | host theming | none |
| mode toggle | dev-mode switch | none |
| auth / github auth | OAuth state | none |
| launcher apps | host app inventory (overlaps launcher plugin discovery) | none |
| updates | update check / apply | none |

Host surface layers (tray, menu, hotkeys, shortcuts, settings surface, world
canvas) are presentation/orchestration, not features — but their state should
be headless-queryable where the mission makes it meaningful (e.g. the hotkey
claim inventory behind "qol-tray takes the hotkey back").

### Shared libs

Infrastructure, not features, and already correctly shaped: `qol-headless`
owns the CLI contract (including the serialized doctor types), `qol-gpui` the
UI kit, `qol-plugin-daemon` the daemon helper. No action beyond consuming.

## Roadmap

### Phase 0 — Codify (this worktree)

- Commit `docs/headless-cli-audit/audit.sh` (the measure) and this plan.
- Encode the host-feature corollary into `qol-arch-code` via
  `qol-workflow:standards-evolution`: "every user-facing feature — including
  host-embedded ones — exposes headless commands; presentation surfaces are
  adapters." The plugin-side standard already exists; the host side does not.

### Phase 1 — Close the host-embedded gap

Give each of the eight host features a headless command surface, using the
existing `qol doctor` / `qol-tray-doctor` precedent: feature logic stays in
qol-tray's lib; the command route is either `qol-tray <cmd>` dispatching
in-process (daemon running) or a `qol` CLI front door that routes to the
daemon or executes the same lib code standalone. Every command honors the
`qol-headless` contract (`help`, `--json` where meaningful).

Order by user value:

1. **plugin store**: `list`, `install <id>`, `update [id]`, `remove <id>` —
   the "machine becomes yours" flow, and the missing headless half of the
   plugin store.
2. **profile**: `export`, `import`, `backup` — completes the headless profile
   story next to the existing `qol sync`.
3. **theme / mode**: `theme get|set`, `mode get|set` — scriptable QoL.
4. **updates**: `check`, `apply`.
5. **task runner**: `run <task>`, `list`, `status` — surface the guardian
   entry as a user-facing command.
6. **auth**: `status`, `login`, `logout`.
7. **launcher apps**: `list` — or fold into the launcher plugin's discovery
   and delete the host copy; decide once, don't build two surfaces.

### Phase 2 — UI-strategy hardening

Plugins already layer UI as adapters (gpui retained surfaces, web settings,
native overlays). Make that a checked invariant for plugin and host UI alike:
UI layers may only consume headless APIs; no domain logic behind a UI
boundary. The gallery/production parity rule in `qol-tray-page-creation` is
the existing mechanism for host UI; extend the same rule to plugin surfaces
via `qol-plugin-gpui-surfaces`.

### Phase 3 — The 100% gate

- Grow `audit.sh` into a CI-enforced gate: add per-plugin action mapping
  coverage (every `[action.*]` resolves to a declared command), `doctor --json`
  parse check, `help` exit-0 check, and "no host feature without a command
  route".
- Extend `qol doctor` with a "headless contract" check group so the gate is
  visible to the user (mission: failures are visible), not only to CI.

### Phase 4 — New features start headless

- `qol plugin new` scaffolder (ecosystem P3-1) generates headless-first
  plugins from `template`.
- New ecosystem candidates (clipboard-history, text-expander, window-tiling,
  ...) are built headless-first; UI strategy is picked from the shared kit.
- Definition of done: a feature without a headless command is not done — the
  Phase 3 gate is what makes that a fact, not a preference.

## Definition of 100%

- Every feature unit answers `help` and `doctor --json`, and every user
  capability is invocable as `<binary> <command>` under the `qol-headless`
  contract.
- Every host-embedded feature has a command route that works without the tray
  UI (in-process or via the `qol` front door).
- No domain logic lives in UI/presentation layers (gate-enforced).
- `qol doctor` aggregates the whole ecosystem headlessly — one command
  exercises everything, host and plugins alike.

## Open decisions for later sessions

- **Command ownership**: which host-feature commands live on `qol-tray` vs the
  `qol` front door. Precedent (`qol doctor`) favors: qol-tray owns logic,
  `qol` is the front door for user-scriptable features; record the choice per
  feature in Phase 1.
- **launcher apps**: dedupe host inventory with launcher plugin discovery
  before building its command surface.
- **Host-surface state queries**: whether `hotkeys list` / `shortcuts list`
  commands are worth adding behind the "qol-tray owns its surface" promise.
