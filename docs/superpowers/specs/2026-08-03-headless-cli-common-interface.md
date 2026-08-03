# Headless CLI common interface — incremental design specification

Date: 2026-08-03

Five-version design journey from the real ecosystem state to a unified qol
flavour headless-CLI contract. Each version: mock interface, simulation against
the 18-flavour mock lab (`docs/headless-cli-mock/`), bug found, no-regression
evolution to the next.

## V1 — The naive uniform surface

**Mock:** The naive common interface says every feature binary exposes:
`help`, `doctor`, `--json doctor`, `version`, `status`, `settings`, `daemon`,
`kill`, `config show/get`, action-specific verbs. No-args = `status` (safe,
read-only).

**Pros** of a common interface:
- Learnability: one mental model across 21+ binaries.
- Scriptability: `for f in plugins/*; do $f status; done`.
- Host integration: action executor, doctor aggregate, and daemon supervisor
  speak one dialect.
- qol-headless implements once. Template enforces it. New features conform
  cheaply.
- Doctor coverage: every binary's invariants are machine-checked.

**Cons:**
- Feature natures differ radically: daemon (window-actions) vs on-demand
  (removeapp) vs action-only (lights, window-actions) vs data (bluetooth) vs
  UI-dependent (alt-tab needs cinnamon/muffin). Forced verbs become no-ops or
  lies.
- Migration cost: 21 binaries, `plugin.toml` action args, host action
  executor, legacy argv normalization — renaming verbs breaks the host
  contract.
- The mission's "host owns config" tension: a `config set` would write
  standalone config that the host overrides on next launch.
- Lowest-common-denominator pressure: `--show` vs `show` shows the ecosystem
  already split on naming.

**Simulation V1 — bare invocation** (the actual mock-lab run, exact transcripts):

```
alt-tab            exit=1    alt-tab: no compatible UI host available (cinnamon/muffin)
bluetooth          exit=0    AA:BB:CC:DD:EE:01  Headphones  paired=true...
cli-sessions       exit=0    cli-sessions: daemon started
controllers        exit=0    Gamepad [USB; xpadneo]: fix pending
ide-checkout       exit=0    task-runner: daemon started
keyremap           exit=0    keyremap: daemon started
launcher           exit=0    launcher: daemon started
lights             exit=0    plugin-lights: no daemon socket; opening settings
os-themes          exit=0    plugin-os-themes: daemon started
pointz             exit=0    pointzerver: daemon started
qol-shot           exit=0    qol-shot: opening region selection UI and starting capture
qol-voice          exit=0    {"session":"idle"}
removeapp          exit=0    removeapp: picker opened
template           exit=0    Hello from My Plugin
window-actions     exit=0    window-actions: daemon started
qol-tray-install   exit=0    qol-tray-install: installing QoL Tray (mock)...
qol-tray-migrate   exit=0    qol-tray-migrate: applied 2 pending migration(s)
qol                exit=0    qol — Build, inspect, diagnose, and run...
```

**Bug V1:** 11 of 18 features have unsafe or inappropriate bare-invocation
defaults. 7 start daemons (would block a session), qol-shot opens region
selection UI (side effect), qol-tray-install installs (mutation),
qol-tray-migrate runs migrations (mutation), removeapp opens picker (UI).
The V1 contract's "no-args = status" is violated by the load-bearing host
daemon-spawn path.

**No-regression check:** every feature's existing verbs continue to work.
Adding canonical verbs alongside original ones means no breakage.

**V2 evolution — safe defaults + lifecycle:** Add to framework:
- `mock_lifecycle <feature> <start-cmd> <kill-fn> <status-fn>` — auto-registers
  `status` and `kill` commands for every daemon feature.
- No-args dispatch: lifecycle status → status; env-gated daemon
  (QOL_MOCK_DAEMON=1 → start verb) → default → help. Preserves host spawn path
  exactly as qol-shot's real ENV_DAEMON_SOCKET pattern.
- Non-daemon features declare no lifecycle → default or help.

**V2 simulation — lifecycle probe** (selected outputs):

```
Status across daemon features: all work, exit=0, reporting daemon not running.
Kill across daemon features:    all work, exit=0.
Bare invocation now:            all show safe status/help, 0 mutations.
QOL_MOCK_DAEMON=1 alt-tab:      alt-tab: no compatible UI host (preserved).
QOL_MOCK_DAEMON=1 cli-sessions:  cli-sessions: daemon started (host spawn preserved).
```

**Bug V2 — canonical start verbs missing:**

```
$ cli-sessions daemon   → cli-sessions: daemon started  (via alias daemon→run)
$ launcher daemon        → unknown command 'daemon'       (start is 'run')
$ keyremap daemon        → unknown command 'daemon'       (start is 'run')
$ pointz daemon          → legacy fallback swallowed it   (exit 0, no-op!)
$ os-themes daemon       → unknown command 'daemon'       (start is 'run')
```

A script wanting to start every daemon generically (`$feature daemon`) breaks on
5 of 9 daemons. Pointz's legacy fallback swallows the typo without error.

**V3 evolution — canonical aliases + dashed→bare:**
- `mock_lifecycle` auto-registers `daemon` as a command that shares the start
  verb's handler (when start verb ≠ "daemon"). No rename, no regression.
- Framework auto-registers bare aliases for dashed command names
  (`--show` → `show`, `--kill` → `kill`).

**V3 simulation:**

```
$ cli-sessions daemon   → cli-sessions: daemon started  ✓
$ launcher daemon        → launcher: daemon started       ✓
$ keyremap daemon        → keyremap: daemon started       ✓
$ os-themes daemon       → plugin-os-themes: daemon started ✓
$ lights daemon          → plugin-lights: daemon started  ✓
$ pointz daemon          → pointzerver: daemon started    ✓
$ alt-tab show           → alt-tab: ... (bare alias from --show) ✓
$ launcher show          → launcher: shown (daemon)       ✓
```

**Bug V3 — zero config surface:** `config show` and `config get` return
"unknown command" on every feature. The only config query is `doctor`'s
`config_readable` check — a boolean. A script can't answer "what are my
managed Bluetooth devices" or "what theme is os-themes set to".

**V4 evolution — read-only config surface:**
- `mock_config <provider>` — auto-registers `config show [--json]` and
  `config get <key>`. Provider echoes a JSON object. Read-only by design.
- No `config set`: writes are host-owned (settings UI). Standalone writes
  are silently overridden by host-injected config on next launch — the
  mission owns config, not the binary. Documented as a conscious trade-off.

**V4 simulation — config + doctor --fix probe:**

```
Config show: all 12 features exit 0, echo JSON (config works) ✓
Doctor --fix:
  bluetooth          → [ok/warn] checks listed, same as regular doctor
  controllers        → [ok/warn] checks listed, same as regular doctor
  os-themes          → [ok] checks listed, same as regular doctor
```

**Bug V4 — repairs are not scriptable.** `doctor --fix` produces the same output
as plain `doctor`; no checks change status. Doctor reports give prose
instructions ("run: plugin-controllers apply_fixes") but a script can't execute
them generically. The `--fix` flag is accepted but has no effect — the
`DoctorCheck` model has no fix handler.

**V5 evolution — doctor --fix with handler:**
- Provider runs with `MOCK_FIX=1` when `--fix` is present. Checks that can
  auto-repair call their feature's own headless command (`apply_fixes`,
  `apply_host_fix`, `toggle_theme`) — the architecture payoff: repairs reuse
  the feature's headless verbs.
- Read-only `doctor` unchanged: no regression.
- Per-check `"fixed": true` marker in the JSON report when a handler ran
  successfully.

**V5 simulation — full journey + residual friction:**

```
Doctor --json parses for every feature: all 18 exit 0/1/2, valid JSON ✓
Doctor --fix where a fix exists:
  controllers:  controller_fixes → ok, fixed=true   ✓
  bluetooth:    managed_devices → ok (after auto-fix) ✓

Residual friction (accepted, deferred to V6):
  1. Legacy fallback swallows typos:
     $ pointzerver stting          → action 'stting' sent (legacy forward)  exit=0
     $ pointzerver --action stting → action 'stting' sent                   exit=0
     A typo in a script = silent forward. install/migrate have the same issue
     through their fallback_command.

  2. Dashed command names persist in help output (alt-tab --show,
     launcher --settings). Bare aliases work at dispatch but canonical names
     aren't shown.

  3. Exit code 2 means three things: doctor-fail, removeapp guard-refusal,
     and general operational failure. Documented per command via exit_behavior;
     standardizing global codes would break existing scripts.

  4. `settings` remains a UI-summon verb by design. The headless config surface
     is `config show/get`.

  5. Host-embedded 8 features still lack headless CLIs entirely — the roadmap
     covers this; the V-series evolves the contract, not the inventory.
```

## Contract summary — V5 target

Every feature binary:

| Surface | Contract |
|---|---|
| No-args | `status` (or `help` when no status exists) |
| `help [path]` | contextual help, mirroring the real `help`/`--help`/`help <cmd>`/`<cmd> help` equivalence |
| `doctor [--json]` | read-only, exit 0/1/2, stable JSON schema |
| `doctor --fix` | runs only checks with fix handlers; reports `"fixed": true` on success |
| `version` | optional but recommended |
| `status` | lifecycle status (auto-registered for daemon features) |
| `daemon` | canonical start verb (aliased to feature's real verb) |
| `kill` | canonical stop verb (auto-registered) |
| `config show [--json]` | read-only effective config, JSON object |
| `config get <key>` | single key, JSON value |
| `<action verbs>` | feature-specific, unchanged |
| `settings` | UI summon (by design — headless config is `config`) |
| Global `--json` | before or after command path; rejected with usage error for non-JSON commands |
| Exit codes | 0 success/cancel, 1 runtime error, 2 failure/refusal, 64 usage |

**What stays unique per feature:** action verbs, daemon lifecycle
characteristics (long-running vs on-demand), UI-host dependencies,
destructive-safety guards (removeapp's confirmation, doctor's --fix gate),
config schema, and legacy argv normalization layers.
