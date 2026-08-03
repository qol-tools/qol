# Headless CLI contract freeze — design spec

Date: 2026-08-04

Status: **blocks Phase 1** of the headless-CLI roadmap. A design review plus
guest-VM execution evidence upgraded the roadmap from "conditional" to
"blocked for implementation" until this contract-freeze phase lands. This
spec freezes the contract that Phase 1 (and the CI gate) must obey. It is the
companion to `2026-08-03-headless-cli-common-interface.md` (the five-version
interface evolution) and `2026-08-03-headless-cli-audit-roadmap.md` (the
phased roadmap). The inventory in section E is the single source of truth for
every count.

## Why this phase exists

Review + guest-VM findings that invalidated the Phase-0 evidence:

| Claim | Reality (verified) |
|---|---|
| audit.sh "21 units, 0 not headless" | grep/filename checks only; `plugin-bluetooth doctor --json` hangs >25 s in a guest, `doctor --fix` rejected rc=64 — both invisible to the audit (C.1) |
| V5 `doctor --fix` transcripts | designed/unsimulated: `DoctorCheck` has no fix handler; mock controllers registers its V5 doctor after `mock_dispatch`; `sim/05-journey.sh` truncates with `head -c 60`, parses nothing (B.1, C.1) |
| universal exit-code table | 2 and 64 both serve usage errors today; `qol-tray <unknown>` exits 2, `doctor --fix` exits 64 (B.1) |
| host features reachable headless | 0/8: every host-feature command returns "Invalid qol-tray invocation"; features sit behind the authenticated HTTP server on 127.0.0.1:42700 (A) |
| daemon no-args compatibility | works today via the env gate, but no matrix and no host-spawn regression test exist (D) |
| destructive commands safe | HTTP server executes immediately while the UI confirms; no `--yes`/dry-run/rollback contract (B.6) |

The six prescribed items are frozen here: host namespace and ownership (A),
per-command transport (A), JSON schemas / timeouts / locking / reload /
destructive policy (B), an executable audit (C), the daemon compatibility
matrix (D), and the complete inventory (E). Exit-code normalization (B.1),
the Bluetooth-doctor acceptance rule (C.2/C.4), and guest-restriction
classification (C.4) are resolved inline.

## A. Host command namespace, ownership, and transport model

Scope: the 8 host-embedded features with 0 headless commands (roadmap table
`docs/superpowers/plans/2026-08-03-headless-cli-audit-roadmap.md:28-40`). `qol-tray`'s argv
classifier only knows daemon/help/version/`--write-mode=`/exec/open/`qol://`/doctor
(`apps/qol-tray/src/app/host_cli.rs:19`); anything else exits rc=2 "Invalid qol-tray
invocation" (`apps/qol-tray/src/app/mod.rs:204`). All 8 features sit behind the authenticated
loopback HTTP server on 127.0.0.1:42700 (`libs/qol-conventions/src/lib.rs:8`,
`features/plugin_store/mod.rs:18`), token + Host protected (`server/security.rs:36,47,60`);
`qol-tray exec` is the proven client primitive (`app/mod.rs:290,348`, `commands/local_http.rs:13-27`).
Front-door rule (PROPOSED): all commands ship on `qol-tray` — it owns the config store, HTTP
token, and the pub(crate) helpers commands call; `qol` keeps `sync` (precedent:
`tools/qol-cli/src/commands/sync/mod.rs:9-13`).

## 1. Plugin store
Routes: GET `/api/plugins`, `/api/installed`, POST `/api/install|update|uninstall/{id}`
(`server/plugin_handlers.rs:19-36`) under `require_api_access` (`server/mod.rs:303-318`);
server ops also `reload_plugin_and_notify` the live manager (`plugin_services/operations/install.rs:9-26`);
uninstall removes the dir (`installer/operations.rs:91-95`). Namespace (PROPOSED): `qol-tray plugin-store list|install|update|remove <id>`.

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `plugin-store list` | `features/plugin_store` | authenticated IPC to an already-running tray (GET `/api/plugins`, `/api/installed`); PROPOSED: degrade to direct one-shot host execution reading the plugins dir when tray down (read-only) | read-only |
| `plugin-store install <id>` | `features/plugin_store` | authenticated IPC to an already-running tray (POST `/api/install/{id}`); refuse "qol-tray is not running" when down (PROPOSED; message path `app/mod.rs:362`) | state-mutating |
| `plugin-store update <id>` | `features/plugin_store` | authenticated IPC to an already-running tray (POST `/api/update/{id}`) | state-mutating |
| `plugin-store remove <id>` | `features/plugin_store` | authenticated IPC to an already-running tray (POST `/api/uninstall/{id}`) | destructive |

## 2. Profile
Export is pure config-dir reads (`core/bundle.rs:7,22`); import reconciles plugins via
`PluginInstaller` and replaces plugin configs + lock (`core/import.rs:8,25-33`); `qol sync` is
the delegation precedent — running tray first, else engine under cross-process `SyncLock`
(`tools/qol-cli/src/commands/sync/mod.rs:124-141`). Namespace (PROPOSED): `qol-tray profile export|import|backup`; `qol sync` stays the sync front door.

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `profile export` | `features/profile/core` | direct one-shot host execution (`build_export_bundle_json`, `core/bundle.rs:22`) | read-only |
| `profile backup` | `features/profile/core` | direct one-shot host execution (writes timestamped bundle into the tracked backups dir; PROPOSED) | state-mutating |
| `profile import <file>` | `features/profile/core` | authenticated IPC to an already-running tray (POST `/api/config/import`, `profile/http/mod.rs:32-33`); refuse when tray down (PROPOSED) | destructive (replaces live configs + lock; export first is recovery) |

## 3. Task runner
Router `/api/task-runner/actions|execute|defaults|config` (`handlers.rs:50-54`) mounted with
auth (`server/mod.rs:347-350`); execution spawns `CommandTask` (`execution/mod.rs:66`).
Namespace (PROPOSED): `qol-tray task list|run|status`.

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `task list` / `task status` | `features/task_runner` | direct one-shot host execution reading `TaskRunnerConfig` (`config.rs`) when tray down; authenticated IPC (GET `/api/task-runner/actions`, `/config`) when up (PROPOSED) | read-only |
| `task run <action> [k=v...]` | `features/task_runner` | authenticated IPC to an already-running tray (POST `/api/task-runner/execute`, `handlers.rs:51`) | state-mutating |

## 4. Theme
One JSON file in the shared config dir (`theme.rs:96-98`), pure read/write + validation
(`theme.rs:30,39,59,100`); daemon spawns already read it for env (`spawn.rs:99-100`). Namespace
(PROPOSED): `qol-tray theme get|set <key> [--accent]`.

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `theme get` | `features/theme.rs` | direct one-shot host execution | read-only |
| `theme set <key>` / `theme set --accent <key>` | `features/theme.rs` | direct one-shot host execution (unknown keys rejected, `theme.rs:59,100`) | state-mutating |

## 5. Mode toggle
`ModeConfig::load/set` + `ModeFlag::parse_cli` (`installer/mode.rs:28,43,53`, re-export
`lib.rs:32`); `--write-mode=` writes mode.json but then starts the tray (`app/mod.rs:210-223`).
Namespace (PROPOSED): `qol-tray mode get|set dev|prod` — `mode set` writes and exits (no tray).

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `mode get` | `installer/mode.rs` (`crate::mode`) | direct one-shot host execution | read-only |
| `mode set dev\|prod` | `installer/mode.rs` (`crate::mode`) | direct one-shot host execution | state-mutating |

## 6. Auth / GitHub auth
Credential reads (`storage.rs:20,24`) + in-crate health (`auth/health.rs:12-21`); device flow
lives in `GitHubAuthService` with in-memory sessions (`service.rs:57,95-103,114`); tray HTTP
has only status/poll/disconnect (`github_auth/http.rs:23-25`). Namespace (PROPOSED): `qol-tray auth status|login|logout`.

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `auth status` | `features/auth` + `features/github_auth` | direct one-shot host execution (stored credential + scopes → `AuthHealth`) | read-only |
| `auth login` | `features/github_auth` | direct one-shot host execution: CLI runs the device flow itself (prints `verification_uri` + `user_code`, polls GitHub, persists via storage write) — PROPOSED, no HTTP session-start route exists today; new tray route deferred | state-mutating |
| `auth logout` | `features/github_auth` | authenticated IPC to an already-running tray (DELETE `/api/github-auth`, `github_auth/http.rs:25`) so credential + sync coupling are torn down by the owner; refuse when tray down (PROPOSED) | destructive |

## 7. Launcher apps
`collect_*` builders are pure reads (`launcher_apps/mod.rs:21-70`); sync writes OS entries and
needs the live PluginManager (`mod.rs:75-88`). Namespace (PROPOSED): `qol-tray launcher-apps
list|sync` (inventory is host state; launcher plugin discovery is separate).

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `launcher-apps list` | `features/launcher_apps` | direct one-shot host execution (PROPOSED: read plugin manifests from the plugins dir instead of PluginManager) | read-only |
| `launcher-apps sync` | `features/launcher_apps` | authenticated IPC to an already-running tray (`trigger_full_sync_with_manager`, `mod.rs:81-88`) | state-mutating |

## 8. Updates
`check_for_updates` = GitHub query + version compare, no tray state (`updates/mod.rs:24-43`);
server has GET `/api/check-update`, POST `/api/self-update` (`meta_handlers.rs:20-21,99,105`);
`download_and_install` needs the daemon EventBus (`updates/mod.rs:64-66`). Namespace (PROPOSED): `qol-tray updates check|apply`.

| Command | Owner | Transport | Kind |
|---|---|---|---|
| `updates check` | `src/updates` | direct one-shot host execution (network read; no tray needed) | read-only |
| `updates apply` | `src/updates` | authenticated IPC to an already-running tray (POST `/api/self-update`); refuse when tray down (PROPOSED) | destructive (replaces the running host binary) |

## Transport rules
- **Direct one-shot host execution** applies when the command reads or replaces a file the
  tray accesses via the same library fns (theme `theme.rs:96`, mode `mode.rs:28-45`, profile
  bundle, credential storage) or makes a pure network query (`updates check`); it must never
  write files the running tray guards with in-process state (plugins lock, runtime config
  cache, plugin configs) — those are IPC-only. Locking: atomic writes only
  (`qol_fs::atomic_write_private`, used for the HTTP token, `security.rs:60-72`); no timeouts.
- **Authenticated IPC to an already-running tray** applies when the op touches live tray
  state: PluginManager reload/notify (`plugin_services/operations/install.rs:9-26`),
  task-runner runtime, profile-import reconciliation, launcher sync, self-update EventBus.
  Client: token from disk, loopback HTTP (`commands/local_http.rs:13-27`), 2s connect / 2s io
  defaults (`libs/qol-runtime/src/local_http.rs:36-41`), tray wrapper raises io to 5s
  (`commands/local_http.rs:16-17`). Server: 401 without token, Host + CSRF checks
  (`security.rs:36,47,60-72`); mutating ops stay POST/DELETE (`plugin_handlers.rs:34-36`).
  OPEN (PROPOSED): install/update/self-update exceed the 5s io timeout (clone + staging,
  `installer/operations.rs:13-40`) — freeze a raised timeout or async-job response.
- **Controlled tray autostart** applies only to ops needing the daemon's manager while the
  tray UI may appear; none proposed here (PROPOSED) — mutating IPC commands report "qol-tray
  is not running" (`app/mod.rs:362`) and exit 1 instead.
- **Separate helper process** applies when an op must outlive the invoking command or run
  under a different privilege/identity; none of the 8 qualify (PROPOSED). Precedent: `qol
  sync` standalone mode (`sync/mod.rs:132-141`) drives a shared engine under a cross-process
  lock (`SyncLock::acquire`), no helper.
## B. Execution semantics

Contract-freeze section for the headless CLI. Basis: spec
`docs/superpowers/specs/2026-08-03-headless-cli-common-interface.md`, audit roadmap
`docs/superpowers/plans/2026-08-03-headless-cli-audit-roadmap.md:69-77` (Phase 1 order:
plugin-store → profile → theme/mode → updates → task → auth → launcher-apps), and
guest-VM execution evidence (confirmed). Existing-code claims cite path:line.

## B.1 Exit-code resolution

**Evidence.** The V5 contract table advertises `0 success/cancel, 1 runtime error,
2 failure/refusal, 64 usage` (spec, "Contract summary — V5 target"), yet 2 and 64 both
serve usage-class failures today (the spec flags this: "Exit code 2 means three
things", residual friction 3). Implementation is split:

- qol-headless defines `EXIT_SUCCESS=0`, `EXIT_RUNTIME_ERROR=1`, `EXIT_USAGE=64`
  (libs/qol-headless/src/lib.rs:14-16); doctor aggregates map `Ok→0, Warn→1, Fail→2`
  (libs/qol-headless/src/doctor/render.rs:48-54).
- qol-tray front door returns **2** for a malformed invocation — `Invocation::Invalid`
  prints "Invalid qol-tray invocation" and exits 2 (apps/qol-tray/src/app/mod.rs:203-206).
- `plugin-controllers doctor --fix` exits **64** with "Unknown doctor check `--fix`":
  `split_output_format` extracts only `--json` (libs/qol-headless/src/lib.rs:907-917),
  so `--fix` dispatches as a check id, misses, and `DispatchError::Usage` → `EXIT_USAGE`
  (libs/qol-headless/src/lib.rs:729, 879, 962-966). `DoctorCheck` has no fix handler
  (libs/qol-headless/src/doctor/check.rs:7-12); `DoctorCheckResult.fix` is advisory prose
  (libs/qol-headless/src/doctor/contract.rs, `with_fix`).

**Rule B1 (normalize on 64).** All usage errors across all binaries — including the
qol-tray front door — exit **64**: the framework already implements 64, the V5 table
advertises it, and the lone outlier is `Invocation::Invalid` at 2. rc=2 stays
reserved for failure/refusal (guard refusals, doctor fail); 0/1 retain success and
runtime error.

**Migration note (PROPOSED timing: Phase 1).** `qol-tray <bad-args>` changes 2 → 64;
scripts branching on `== 2` for "bad invocation" must test 64; zero/nonzero branching
is unaffected. Doctor aggregates (0/1/2) are status, not usage — unchanged.

## B.2 JSON response envelope (PROPOSED)

Only `--json` commands emit JSON; the envelope is identical on every binary. Doctor
reports keep their existing shape (`DoctorReport`: `plugin_id,status,checks[]` with
`id,status,message,fix,details` — libs/qol-headless/src/doctor/contract.rs) and are
NOT wrapped. PROPOSED schema for host commands:

```json
{ "ok": true,  "command": "plugins remove <id>", "result": { } }
{ "ok": false, "command": "plugins remove <id>", "error": { "class": "usage|runtime|refusal", "message": "...", "details": { } } }
```

- `ok`: bool. `command`: echoed canonical command path. `result`: command-specific
  object, required when `ok`. `error.class` mirrors the exit code (usage=64, runtime=1,
  refusal=2); `details` optional, JSON-serializable. Unknown fields are rejected on
  parse — schema evolution is additive-only.

## B.3 Timeout policy

**Rule B3a.** Every command terminates within a defined deadline or exits 1 (runtime)
with the timeout in `error.details`. Defaults PROPOSED: host commands 30s; doctor
checks 5s (precedent: `PLUGIN_DOCTOR_TIMEOUT = Duration::from_secs(5)`,
apps/qol-tray/src/doctor/aggregation/plugin_runner.rs:7).

**Rule B3b.** Doctor checks probing hardware/environment report absence as a
structured `warn`/`fail` `DoctorCheckResult`, never block. Guest-VM evidence:
`plugin-bluetooth doctor` hung when BlueZ was absent — the `bluez_available`/
`adapter_powered` probes (plugins/bluetooth/src/cli.rs:281-314) query a system
service with no deadline. Fix: probes run under the B3a deadline, mapping
absence to `warn` ("service unavailable") or `fail` per check semantics.

## B.4 Locking

Config writes are host-owned; binaries expose `config show/get` only (spec V4
decision). The host's existing guard is `ProfileConfigWriteGuard`
(apps/qol-tray/src/plugins/config/mod.rs:146-151), backed by a process-wide
`static OnceLock<RwLock<()>>` (mod.rs:126-128).

**Rule B4.** Any command mutating host config takes the write guard for the full
mutation and marks its invalidation scope (`profile_config_write_guard()` = All,
`_for_plugin` = Plugins(vec) — mod.rs:169-171, 173-182, 194-201; callers
mod.rs:448, 562, 936).

- Scope: whole tray process — the lock is process-local, and standalone binaries
  never write config, so no cross-process lock exists. Blocking semantics:
  `RwLock::write()` blocks until readers drain (mod.rs:187-192); no try-lock, no
  timeout on the guard; never hold it across network waits, daemon IPC, or UI.
- Non-config host state (plugin store, updates, auth tokens) gets named locks
  (PROPOSED: one per service, same blocking semantics), never the config guard.

## B.5 Reload behavior

**Rule B5.** A command whose mutation changes the effective plugin set or config
triggers reload before reporting success. Precedent: uninstall calls
`reload_plugin_and_notify` (plugin_services/operations/uninstall.rs:27,
helpers.rs:94) — reload = re-read configs + notify listeners; exit 0 is the
completion signal, no background drift. `config show/get` never reloads.

## B.6 Destructive-operation policy

**Gap.** The plugin-store HTTP server executes install/update/uninstall immediately
on POST (server/plugin_handlers.rs:87-113), while the UI gates uninstall behind a
confirm subpage (ui/views/plugins/uninstall-confirm-subpage.js:26-39). Headless
commands must NOT inherit that gap — the CLI layer adds the gate itself.

**Rule B6 (common).** Every destructive command: (a) requires `--yes` in non-TTY
contexts or an interactive confirm in a TTY — never silent; (b) is idempotent —
re-running after a partial failure completes the operation (precedent: uninstall
tolerates already-missing installs, operations/uninstall.rs:38-40); (c) waits for
completion before exit (B5); (d) fails under B3a, never hangs.

Per class (PROPOSED details):

| Class | Confirm | Preview | Backup | Rollback | Wait |
|---|---|---|---|---|---|
| plugin remove | `--yes`/TTY | `plugins list` + `--dry-run` | removed plugin kept in trash | re-install from backup on failure | reload before exit 0 |
| profile import | `--yes`/TTY | import diff (PROPOSED) | current profile snapshot | restore snapshot on failure | reload before exit 0 |
| updates apply | `--yes`/TTY | `updates list` staging | prior artifact retained | revert to prior version | completion before exit 0 |
| auth logout | `--yes`/TTY | n/a | n/a (token revocation) | n/a — idempotent by design | token revoked before exit 0 |
| task execution | explicit command | `--dry-run` where task schema allows | n/a | task-defined | completion or B3a failure |

## B.7 Decisions

1. **B1** — Usage errors exit 64 on every binary; qol-tray `Invocation::Invalid`
   migrates 2 → 64 in Phase 1; 0/1/2 retain success/runtime/failure-refusal.
2. **B2** — All `--json` host output uses `{ok, command, result|error{class,message,details}}`;
   doctor reports stay unwrapped.
3. **B3** — Every command has a deadline (30s host / 5s doctor, PROPOSED); doctor
   probes report absent hardware/environment as structured warn/fail, never hang.
4. **B4** — Config mutations take `profile_config_write_guard` for the whole
   mutation; blocking RwLock; never held across long I/O; named locks for other state.
5. **B5** — Mutating commands reload (and notify) before reporting success.
6. **B6** — Destructive commands are confirm-gated, idempotent, preview-able, backup +
   rollback where the class table says so, and always wait for completion.## C. Executable audit (contract gate)

Status: section C of the contract-freeze spec. Everything in C.2–C.6 beyond the existing
`audit.sh` and the in-tree bluetooth action test is **PROPOSED**: roadmap Phase 3 ("Grow
audit.sh into CI gate + `qol doctor` headless contract check group") is not implemented —
`audit.sh` appears in no `.github/workflows/*` file, and `qol doctor` only forwards to the
qol-tray-doctor aggregate (tools/qol-cli/src/commands/doctor.rs:8-21).

## C.1 Why the structural audit alone cannot certify headless

`audit.sh` is grep/filename-only: it counts `^\[runtime\]` / `^doctor = true` in
`plugin.toml`, `qol-headless` in `Cargo.toml`, and presence of `cli.rs` / `HeadlessApp`
(docs/headless-cli-audit/audit.sh:48-61), then checks the `qol-headless` dep for tool/app
crates (audit.sh:71-87). It never executes a binary, yet reports "21 units, 0 not headless"
(audit.sh:91-93). Guest-VM execution disproved several of those verdicts:

| audit verdict | Guest-VM evidence | Root cause |
|---|---|---|
| `plugin-bluetooth` headless | `doctor --json` hangs >25 s (timeout rc=124, no output) in a guest without BlueZ services | `bluez_available` check calls `adapter_health()` (plugins/bluetooth/src/cli.rs:316-317) → `default_adapter().await` with no deadline (plugins/bluetooth/src/platform/linux.rs:369-378); the bluer D-Bus call blocks when `org.bluez` is absent |
| `plugin-controllers` headless | `doctor --fix` rejected, rc=64, "Unknown doctor check `--fix`" | doctor accepts only registered check ids; unknown ids are `DispatchError::Usage` → `EXIT_USAGE=64` (libs/qol-headless/src/lib.rs:16,281; rejection in `selected_doctor_checks`, lib.rs:875-880). `--fix` is not a flag; fixes are the separate `apply_fixes` command (plugins/controllers/src/cli.rs:26-36) |
| `qol-tray` headless | `doctor --json` works: rc=2, valid JSON `{status,host,plugins}` | aggregate shape is `DoctorAggregateReport{status,host,plugins}` (libs/qol-headless/src/doctor/contract.rs:120-123); exit 2 = failures (tools/qol-cli/src/commands/doctor.rs:25 help text). Plugins fail in-guest with "current process is outside the configured cgroup delegation" (libs/qol-process/src/platform/linux/containment/mod.rs:1227) — a guest restriction, NOT a product regression |

Conclusion: structural checks prove *shape* (manifest declares runtime/doctor/daemon, crate
depends on qol-headless); they cannot prove *behavior* (binary builds, `help` exits 0,
`doctor --json` parses, action argv is accepted, commands terminate).

## C.2 Gate shape: three layers + one inventory

### Layer 1 — structural source audit (keep `audit.sh` unchanged)
Runs on source with no build; fast pre-filter for CI. Its "yes" is a *claim to be proven*
by layers 2–3, never a certification. Exit contract stays (0 = all claimed, 1 = gap,
audit.sh:96).

### Layer 2 — executable contract smoke (PROPOSED; the core of the gate)
Runner: small Rust tool `tools/qol-contract-gate` (see C.3). For every unit layer 1 marks
headless:

- `help` → must exit 0 (`EXIT_SUCCESS`, libs/qol-headless/src/lib.rs:14; help dispatch
  returns before any command handler runs, lib.rs:413-419). Non-zero exit or empty stdout = FAIL.
- `doctor --json` → must exit within `DOCTOR_TIMEOUT` and print exactly one valid JSON
  document matching the schema: `status: "ok"|"warn"|"fail"`, `host: DoctorReport`,
  `plugins: [PluginDoctorReport]` (libs/qol-headless/src/doctor/contract.rs:18-25,120-123).
  Exit 0/1/2 are all valid for doctor (0 healthy, 1 warnings, 2 failures —
  tools/qol-cli/src/commands/doctor.rs:25); any other exit code or unparseable output = FAIL.
- **FAIL-fast rule**: a doctor command that produces no output within `DOCTOR_TIMEOUT` is a
  FAILURE, never a warning. Rationale: the bluetooth hang (C.1) emitted nothing at all; a
  silent hang is the exact failure mode this gate exists to catch. Precedent: the qol-tray
  aggregate runner already classifies timeouts as a distinct outcome (`Invocation::TimedOut`,
  apps/qol-tray/src/doctor/aggregation/plugin_runner.rs:79-82) with
  `PLUGIN_DOCTOR_TIMEOUT=5 s` (plugin_runner.rs:7); the gate's direct-execution limit must be ≥ that.

| Constant | Value | Basis |
|---|---|---|
| `HELP_TIMEOUT` | 10 s | help path never touches platform code (lib.rs:413-419) |
| `DOCTOR_TIMEOUT` | 10 s | 2× the in-host aggregate limit (plugin_runner.rs:7); the bluetooth hang exceeded 25 s, so 10 s catches it with cold-start margin |
| `DOCTOR_OUTPUT_LIMIT` | 1 MiB | mirrors the aggregate runner's `PLUGIN_DOCTOR_OUTPUT_LIMIT` |
| per-unit budget | 30 s | 3 invocations × 10 s worst case |

### Layer 3 — manifest action→argv validation (PROPOSED)
- Parse every `[action.*]` block in each `plugin.toml`, take `args` — e.g.
  `[action.reconnect] args = ["reconnect"]` (plugins/bluetooth/plugin.toml:10-13), settings
  kind included (plugin.toml:35-38).
- For each argv list run `binary help <arg1> [<arg2>…]` and require exit 0. Safe: help
  returns before command handlers run (lib.rs:413-419). Multi-token paths (`qol-voice
  session start`, pointz `action kill`) are verified as one help path.
- In-tree precedent exists as a unit test: `manifest_actions_have_cli_commands` executes
  `help <command>` for every manifest action and asserts `EXIT_SUCCESS`
  (plugins/bluetooth/src/cli.rs:516-537). The gate re-runs the same check against the
  *built binary* — unit tests run in-process and only when the crate's tests run, and
  bluetooth is the only plugin with this test today.

### Host-feature inventory (PROPOSED; ledger, not a pass/fail gate)
- The 8 host-embedded features (plugin store, profile, task runner, theme, mode, auth,
  launcher apps, updates — roadmap §2) are 0/8 headless today; qol-tray's headless surface
  is `doctor`/`help`/`--version`/`exec`/`open`/`--write-mode=`
  (apps/qol-tray/src/app/mod.rs:192,240-247); only profile has partial coverage via `qol sync`.
- One row per feature, value `COVERED` (its `<command> help` exits 0, verified) or
  `NOT_YET`. The check fails only on regressions: a `COVERED` feature whose command
  disappeared, or a command in help that belongs to no feature. Expected current result:
  8× `NOT_YET`. As Phase 1 lands commands, rows flip to `COVERED` and the gate starts
  executing their help.

## C.3 Runner shape: Rust tool for layers 2–3, bash stays for layer 1

Layers 2–3 need spawn-with-timeout, exit-code capture, JSON schema validation, and
per-unit classification — precisely where shell `timeout(1)` (rc=124 ambiguity), grep-based
JSON checks, and exit-code arithmetic produce false verdicts. `tools/qol-contract-gate` is
a single binary (std `Command` + `serde_json`; no new framework):
`qol-contract-gate run --target <dir> [--guest]` — JSON report on stdout, human report on
stderr. Platform awareness: execute only units whose `plugin.toml` `platforms` include the
runner's OS (today: bluetooth/controllers/os-themes/qol-voice linux-only, keyremap
macos-only, the rest linux+macos); other units get structural-only coverage. The `qol
doctor` headless-contract check group (Phase 3) surfaces the same three layers; `qol doctor`
today only forwards to qol-tray-doctor (tools/qol-cli/src/commands/doctor.rs:8-21).

## C.4 Environment classification

On failure, classify by stderr signature before assigning the verdict. Classification only
changes *how* a failure is reported — the timeout rule has no exception:

| Signature | Class | Gate verdict |
|---|---|---|
| "current process is outside the configured cgroup delegation" (libs/qol-process/src/platform/linux/containment/mod.rs:1227) | `GUEST_RESTRICTION` | SKIP, recorded; accepted only with `--guest` |
| `org.bluez` / D-Bus absent (bluetooth case) | `ENV_MISSING` | FAIL with structured detail `env: no BlueZ` — but only if the binary *produced output*; a silent hang is still FAIL |
| any other non-zero exit / bad JSON / timeout with output | `PRODUCT_REGRESSION` | FAIL (blocks) |

Contract: a doctor command must terminate and report its environment as a structured
warning or failure; "no output within `DOCTOR_TIMEOUT`" has no env exception.

## C.5 Gate exit codes

| Code | Meaning |
|---|---|
| 0 | all layers pass (SKIPs recorded) |
| 1 | ≥1 `PRODUCT_REGRESSION` or unclassified failure |
| 2 | ≥1 failure, all classified `ENV_MISSING` / `GUEST_RESTRICTION` — never silent; CI policy decides (guest runs may treat 2 as pass-with-annotations, host runs may not) |
| 64 | gate usage error (mirrors `EXIT_USAGE`, libs/qol-headless/src/lib.rs:16) |

## C.6 CI wiring (PROPOSED)

- New `headless-gate` job in `.github/workflows/ci.yml`, `ubuntu-latest`: `cargo build` the
  21 units (or consume `qol check` artifacts — `qol check` already builds affected crates
  into `target/`, tools/qol-cli/src/commands/check/mod.rs:15-35), then
  `qol-contract-gate run --target target/debug`; job fails on exit 1/2. Runs after
  "Compute affected crates" (ci.yml:55-57), parallel to the lint/test matrix.
- Guest-VM runs (qol env guests) use `--guest`: `GUEST_RESTRICTION` skips are expected
  there; `ENV_MISSING` and `PRODUCT_REGRESSION` still fail.
- `audit.sh` stays as the no-build pre-filter; its "yes" alone must never be the gate (C.1).
## D. Daemon compatibility matrix

Contract-freeze section for the headless CLI. Basis: spec
`docs/superpowers/specs/2026-08-03-headless-cli-common-interface.md` (V2/V3 canonical
`daemon` start verb, `status`, `kill`, no-args=status-or-help), audit roadmap
`docs/superpowers/plans/2026-08-03-headless-cli-audit-roadmap.md:14-32` (per-plugin daemon
yes/no + commands), host supervisor `apps/qol-tray/src/plugins/daemon_lifecycle/spawn.rs`,
and guest-VM execution evidence (confirmed). Every cell was verified against source;
nothing below is inferred. All 13 daemon plugins declare `[daemon] enabled = true`.

## D.1 The env gate (how the host spawns daemons)

The supervisor launches each daemon binary with **no arguments** (spawn.rs:89-107) and
injects `QOL_TRAY_PLUGIN_ID`, `QOL_TRAY_PLUGIN_DIR`, `QOL_TRAY_DAEMON_SOCKET` (rewritten
from the manifest path to `<runtime_dir>/sockets/<basename>`,
apps/qol-tray/src/dev_generation/mod.rs:83-93), `QOL_TRAY_DAEMON_REPLACE_EXISTING=1`,
`QOL_TRAY_STATE_SOCKET`, optional `QOL_TRAY_HTTP_TOKEN`, theme env (`QOL_TRAY_THEME_ACCENT`,
`QOL_TRAY_THEME_NAME`), `RUST_LOG` (warn prod / debug dev), and removes `XMODIFIERS`
(spawn.rs:96-125). Listeners are pre-bound by the host and passed as inherited fds
(`QOL_TRAY_DAEMON_LISTENER_FD`, `QOL_TRAY_DAEMON_PORT_FD` + per-extra-port),
plugins/daemon_lifecycle/listener/platform/unix.rs:195-216; daemons adopt them and skip
bind() (libs/qol-plugin-daemon/src/daemon/platform/unix.rs:328-332).

Two gate mechanisms exist in the daemons:

1. **Explicit main-rs gate** — no args + `QOL_TRAY_DAEMON_SOCKET` set → daemon mode,
   else the CLI: bluetooth main.rs:14, controllers main.rs:5, qol-voice main.rs:5.
   qol-shot gates on `QOL_TRAY_DAEMON_REPLACE_EXISTING` instead (main.rs:14, 25-27);
   lights gates inside its default command (runtime/mod.rs:14-17).
2. **Implicit bind gate** — the daemon is the no-args *default command*, but the
   listener uses `SocketSource::EnvRequired` (unix.rs:49) and fails cleanly when the env
   is absent (`"QOL_TRAY_DAEMON_SOCKET is not set"`, unix.rs:339-347), so the process
   exits without daemonizing. Used by alt-tab, cli-sessions, keyremap, launcher,
   os-themes, pointz, window-actions.

Guest-VM evidence: with the supervisor env + no args, every tested daemon entered daemon
mode (stderr `[daemon] binding to ...`, unix.rs:350; `[task-runner] listening on
127.0.0.1:42720`, ide-checkout/src/daemon/server.rs:20; `[pointz] daemon started`,
pointz/src/app/mod.rs:42) and stayed long-running. Re-spawning `task-runner` while the
guest supervisor's instance already served returned rc=1 "Address already in use" —
`bind_with_takeover` is a plain `TcpListener::bind` with no takeover
(ide-checkout/src/daemon/takeover.rs:9-11).

## D.2 The matrix

| Plugin (id) | Daemon binary | No-args behavior | Start verb(s) | Canonical `daemon` alias | Kill verb | Status verb | Socket (plugin.toml) |
|---|---|---|---|---|---|---|---|
| plugin-alt-tab | alt-tab | default_command `daemon` (runtime/cli/mod.rs:100); no env → listener fails → **silent exit 0**, no daemon (runtime/operational.rs:67-69); env → retained GPUI picker | `daemon` | yes — native | `--kill` (cli/mod.rs:39) | — | /tmp/qol-alt-tab.sock (:29) |
| plugin-bluetooth | plugin-bluetooth | **env-gated** (main.rs:14); bare → default_command `list` (cli.rs:22), read-only | none (host-spawn only) | no | — | — | /tmp/plugin-bluetooth.sock (:73) |
| plugin-cli-sessions | cli-sessions | default_command `run` (cli.rs:39); no env → rc=1 "action listener failed to bind" (ui/run.rs:62-63); env → GPUI panel daemon | `run` | yes — alias (cli.rs:42) | — | — | /tmp/qol-cli-sessions.sock (:28) |
| plugin-controllers | plugin-controllers | **env-gated** (main.rs:5); bare → default_command `status` (cli.rs:18) | none (host-spawn only) | no | — | `status` (cli.rs:42) | /tmp/plugin-controllers.sock (:25) |
| plugin-ide-checkout | task-runner | default_command `daemon` (cli.rs:26) with **no env gate** (daemon/mod.rs:14-22) → bare invocation binds TCP 42720 and blocks (server.rs:18-29). **GAP (D.3)** | `daemon` | yes — native | — | `status` (cli.rs:37, /health probe) | none; port 42720 (:27) |
| plugin-keyremap | keyremap | default_command `run` (cli.rs:26); macOS no env → listener fails → silent exit 0 (platform/macos/app/mod.rs:24-28); non-macOS → rc=1 (main.rs:29-38 test); env → event-tap daemon | `run` | no | `kill` / `--kill` (cli.rs:53-54) | — | /tmp/qol-keyremap.sock (:20) |
| plugin-launcher | launcher | default_command `run` (cli.rs:68); no env → "[launcher] daemon listener failed, exiting", exit 0 (ui/run.rs:30-32); env → retained GPUI launcher | `run` | no | `--kill` (cli.rs:94) | — | /tmp/qol-launcher.sock (:25) |
| plugin-lights | plugin-lights | **env-gated inside default** `launch` (cli.rs:37; runtime/mod.rs:14-17): no env → open_settings (safe); env → daemon | `daemon` | yes — native (`run` is alias, cli.rs:65-66) | — | — | /tmp/plugin-lights.sock (:109) |
| plugin-os-themes | plugin-os-themes | default_command `run` (cli.rs:32); no env → rc=1 "failed to start daemon listener" (app/daemon_run.rs:17-19); env → cursor-effect daemon | `run` | no | `kill` (cli.rs:55) | — | /tmp/qol-os-themes.sock (:33) |
| plugin-pointz | pointzerver | default_command `server` (cli.rs:73); no env → silent exit 0 (app/mod.rs:35-40); env → input server | `server` | no — `daemon` swallowed by legacy fallback, forwarded as action (cli.rs:151-160) | `kill` (cli.rs:77,127) | `connection_status` (action, not `status`) | /tmp/qol-pointz.sock (:28) |
| qol-shot | qol-shot | **env-gated on `QOL_TRAY_DAEMON_REPLACE_EXISTING`** (main.rs:14,25-27); bare → default_command `record` (cli.rs:25), capture — not a daemon | none (host-spawn only) | no | — | — | /tmp/qol-shot.sock (:54) |
| qol-voice | qol-voice | **env-gated** (main.rs:5); bare → default `session status` → rc=1 "qol-voice daemon is not reachable" (cli/mod.rs:23; app/mod.rs:41-44) | none (host-spawn only) | no | — | `session status` (cli/session.rs:45) | /tmp/qol-voice.sock (:36) |
| plugin-window-actions | window-actions | default_command `daemon` (cli.rs:72); no env → rc=1 "Failed to start window-actions daemon listener" (app/mod.rs:138-141); env → glide daemon | `daemon` | yes — native (`run` is alias, cli.rs:89-90) | — | — | /tmp/qol-window-actions.sock (:75) |

Reads consistent with the roadmap table (roadmap:14-32). UNVERIFIED: none — every cell
above was read from the cited source. The manifest socket path is the *declared* name;
the injected `QOL_TRAY_DAEMON_SOCKET` is the dev-generation rewrite (D.1).

## D.3 Host-spawn regression test (PROPOSED)

**Where it lives.** A `daemon-spawn` subcommand of the proposed C.3 runner,
`tools/qol-contract-gate` — same spawn-with-timeout machinery, same JSON report shape.

**How it runs.**
- **Guest VM (primary, full 13-unit matrix):** `qol env up <guest> --dev-worktree`
  (qol-dev-environments rule — daemons are long-running desktop processes; the GPUI
  daemons alt-tab/launcher/cli-sessions/os-themes/keyremap need a display). Uses its own
  temp socket path per unit so it never collides with the guest supervisor's daemons;
  task-runner collides on the fixed port 42720 → structured-error branch, which is pass.
- **CI (`.github/workflows/ci.yml`, ubuntu-latest, non-GPUI subset):** bluetooth,
  controllers, ide-checkout, lights, pointz, qol-shot, qol-voice, window-actions (plain
  thread daemons; no display needed).

**Assertion 1 — with supervisor env, no args** (env set exactly as spawn.rs:96-125:
`QOL_TRAY_PLUGIN_ID`, `QOL_TRAY_DAEMON_SOCKET`→temp path, `QOL_TRAY_DAEMON_REPLACE_EXISTING=1`,
`QOL_TRAY_STATE_SOCKET`, `RUST_LOG`, theme env). Within `N = 10 s` (aligned with C.2
`DOCTOR_TIMEOUT`) the process must either (a) stay alive and emit a daemon-mode marker on
stderr (unix.rs:350 / server.rs:20 / pointz app/mod.rs:42), or (b) exit rc=1 with non-empty
stderr (structured error — e.g. task-runner "Address already in use"). **FAIL** if it
prints help/usage/status text instead of daemonizing (the V1 bug class), or exits 0
silently, or produces no daemon signal within N.

**Assertion 2 — no env, no args.** The process must exit within N and must not emit
daemon-mode markers. **FAIL** if it blocks (does not exit within N) or daemonizes —
this is the rule D.4 check. Known today: task-runner FAILS (blocks on the port); every
other daemon exits (0, 1, or safe no-op).

**Assertion 3 (guest only) — replace.** With `QOL_TRAY_DAEMON_REPLACE_EXISTING=1` against
a live instance, the fresh spawn takes over the socket (unix.rs:377-393) or exits rc=1;
never leaves two instances serving the same socket. Not applicable to task-runner (port,
no takeover — takeover.rs:9-11).

## D.4 The compatibility rule

**Rule D4a (env gate is mandatory).** Every daemon MUST enter daemon mode only when the
supervisor env is present (`QOL_TRAY_DAEMON_SOCKET` or inherited listener fd;
qol-shot: `QOL_TRAY_DAEMON_REPLACE_EXISTING`, which the supervisor always injects
alongside the socket). One of the two mechanisms in D.1 is required; a daemon must never
bind its socket/port on its own authority.

**Rule D4b (bare invocation).** No-args invocation without the gate must never start a
daemon and must never block a session. Safe outcomes: status/read-only default (bluetooth,
controllers, qol-voice), safe action (lights opens settings, qol-shot `record`), or a
structured error. rc=1 + stderr message is the preferred no-gate outcome; the silent
exit-0 rows (alt-tab, launcher, pointz, keyremap-macOS) are compliant (non-blocking) but
mask failure — **PROPOSED** convergence to rc=1 + structured stderr in Phase 1.

**Known gap.** plugin-ide-checkout violates D4a/D4b: `task-runner` with no args starts the
daemon unconditionally (daemon/mod.rs:14-22) and blocks the session. **PROPOSED fix
(Phase 1):** gate `daemon::run` on the supervisor env (pattern: bluetooth main.rs:14) and
route bare `task-runner` to `status`. Until fixed, the D.3 gate records it as an expected
GAP and does not block CI.

## D.5 Decisions

1. **D1** — The matrix is the frozen daemon inventory: 13 plugins, binaries, verbs, and
   socket paths as verified above; roadmap:14-32 is consistent.
2. **D2** — Every daemon honors one of the two env-gate mechanisms; qol-shot's
   REPLACE_EXISTING gate is accepted (supervisor always injects it).
3. **D3** — The `daemon-spawn` regression test ships inside `tools/qol-contract-gate`
   (C.3), guest-VM full matrix + CI non-GPUI subset, `N = 10 s`, three assertions.
4. **D4** — Env gate mandatory; bare invocation never daemonizes or blocks; task-runner
   gap fixed in Phase 1; silent exit-0 rows converge on rc=1 + structured stderr.
## E. Canonical inventory

Status: section E of the contract-freeze spec. Single source of truth for every
headless-CLI count: sections, gates, and docs quote counts from here with the category
named (E.4); categories are mutually exclusive (E.1), so no id appears twice in E.2.

## E.1 Membership rules (mutually exclusive, decide in this order)

1. **standalone plugin** — a release unit under `plugins/<id>/` with a `plugin.toml`
   declaring `[runtime]`/`[daemon]` and a crate binary; the audit's `check_plugin` loop
   over `plugins/*/` (docs/headless-cli-audit/audit.sh:79-80).
2. **host surface** — the `qol-tray` application binary itself, the orchestrating
   surface (tray, menu, hotkeys, shortcuts, settings surface, world canvas,
   notifications — roadmap §3, roadmap:45-51); its non-binary layers are not units.
3. **consumer CLI** — a standalone binary that drives plugins or host surfaces but is
   not a plugin and not the tray app: `qol`, `qol-guest-runner` (tools/), and the
   `qol-tray-install` / `qol-tray-doctor` / `qol-tray-migrate` `[[bin]]` targets
   (apps/qol-tray/Cargo.toml:9-22).
4. **host-embedded feature** — a capability owned by qol-tray with no standalone
   binary: modules under `apps/qol-tray/src/features/` (plugin_store, profile,
   task_runner, theme.rs, mode_toggle.rs, auth, github_auth, launcher_apps) or
   `apps/qol-tray/src/updates/` (updates); exactly the 8 of roadmap:32-44
   (auth+github_auth = the single "auth" feature).
5. **mock-only** — a shell-script stand-in under `docs/headless-cli-mock/bins/`;
   a doc artifact, not a release unit.

Test: plugin dir → plugin; `qol-tray` → host surface; install/doctor/migrate/`qol`/`qol-guest-runner` → consumer CLI; feature module → host-embedded feature; else mock → mock-only.

## E.2 Canonical inventory (one row per id; mock compressed)

| Category | id | Headless status today (verification) | Overlap notes |
|---|---|---|---|
| standalone plugin | alt-tab | yes — audit-grep | — |
| standalone plugin | bluetooth | claimed yes — **disproven**: `doctor --json` hangs >25 s in guest (03 C.1) | linux-only |
| standalone plugin | cli-sessions | yes — audit-grep | — |
| standalone plugin | controllers | claimed yes — **disproven**: `doctor --fix` rc=64 (03 C.1) | linux-only |
| standalone plugin | ide-checkout | yes — audit-grep | display name "Task Runner" collides with host task runner (E.3.3) |
| standalone plugin | keyremap | yes — audit-grep | macos-only; never in Linux guest payload (E.5) |
| standalone plugin | launcher | yes — audit-grep | discovery overlaps host launcher-apps inventory (E.3.1) |
| standalone plugin | lights | yes — audit-grep | — |
| standalone plugin | os-themes | yes — audit-grep | name collides with host theme feature (E.3.2); linux-only |
| standalone plugin | pointz | yes — audit-grep | — |
| standalone plugin | qol-shot | yes — audit-grep | — |
| standalone plugin | qol-voice | yes — audit-grep | — |
| standalone plugin | removeapp | yes — audit-grep | no daemon (audit: daemon=0) |
| standalone plugin | template | yes — audit-grep | reserved id, scaffold, no daemon; never in guest payload (E.5) |
| standalone plugin | window-actions | yes — audit-grep | — |
| host-embedded feature | plugin store | no — source layout (features/plugin_store) | — |
| host-embedded feature | profile | partial (`qol sync` only) — roadmap:35 | — |
| host-embedded feature | task runner | no — source layout (features/task_runner) | generic runner vs ide-checkout plugin (E.3.3) |
| host-embedded feature | theme | no — source layout (features/theme.rs) | name collides with plugin-os-themes (E.3.2) |
| host-embedded feature | mode toggle | no — source layout (features/mode_toggle.rs) | — |
| host-embedded feature | auth (incl. github_auth) | no — source layout (features/auth, github_auth) | — |
| host-embedded feature | launcher apps | no — source layout (features/launcher_apps) | app inventory overlaps plugin-launcher discovery (E.3.1) |
| host-embedded feature | updates | no — source layout (src/updates/) | — |
| consumer CLI | qol | yes — audit-grep (audit.sh:83) | — |
| consumer CLI | qol-guest-runner | yes — audit-grep (audit.sh:84) | — |
| consumer CLI | qol-tray-install | yes — audit-grep (audit.sh:86-87) | shares apps/qol-tray manifest |
| consumer CLI | qol-tray-doctor | yes — audit-grep + **executed** in guest (03 C.1) | shares apps/qol-tray manifest |
| consumer CLI | qol-tray-migrate | yes — audit-grep (audit.sh:86-87) | shares apps/qol-tray manifest |
| host surface | qol-tray | yes — **executed** in guest (03 C.1); front door `doctor`/`help`/`exec`/`open` (apps/qol-tray/src/app/mod.rs:192,240-247) | — |
| mock-only | alt-tab, bluetooth, cli-sessions, controllers, ide-checkout, keyremap, launcher, lights, os-themes, pointz, qol, qol-shot, qol-tray-install, qol-tray-migrate, qol-voice, removeapp, template, window-actions (18) | n/a — doc artifact (shell stand-ins in docs/headless-cli-mock/bins/) | set lacks qol-guest-runner, qol-tray, qol-tray-doctor; includes template + keyremap, which ship in no guest payload (E.5) |

## E.3 Overlap resolutions

**E.3.1 launcher plugin vs launcher-apps host feature — distinguish.** The plugin is
"Universal search with action modifiers" (plugins/launcher/plugin.toml:4) with its own
discovery/ranking/launch (plugins/launcher/src/discovery/). The host feature is the app
**inventory**: `LauncherEntry{file_stem, display_name, bundle_id, exec_args,
shortcut_action}` (apps/qol-tray/src/features/launcher_apps/mod.rs:15-24), consumed by
the host runtime server. Rule: inventory is owned by the host feature; the plugin
consumes it for search/rank/launch; `launcher-apps list` lives on the host front door
(resolves the roadmap's "or fold into launcher" fork, roadmap:43).

**E.3.2 os-themes plugin vs theme host feature — distinguish, naming collision only.**
Host theme = qol-tray's own UI appearance (`theme.json`, accent/theme —
apps/qol-tray/src/features/theme.rs:6-15). Plugin os-themes = OS-wide theming + cursor
effects (plugins/os-themes/plugin.toml:4), linux-only (plugin.toml:8). Different
target surfaces (own UI vs OS); no merge — the overlap is the word "theme" only.

**E.3.3 ide-checkout plugin vs host task runner — distinguish by scope; fix the name.**
Host task runner is a generic config-driven action runner: `ActionConfig{command,
timeout, cwd}` (apps/qol-tray/src/features/task_runner/config.rs:14-27), axum routes
`/actions` `/execute` `/defaults` `/config` (handlers.rs:50-54). The ide-checkout plugin
is a git-checkout domain API (`POST /checkout`, plugins/ide-checkout/src/daemon/server.rs:138)
named "Task Runner" (plugins/ide-checkout/plugin.toml:4). Rule: the generic execution
primitive is the host feature, the git-checkout-for-IDE domain is the plugin; rename
the plugin's display name to "IDE Checkout" (id `plugin-ide-checkout` unchanged); the
roadmap's `task run|list|status` commands (roadmap:37) belong to the host feature.

## E.4 Denominator rule and exact counts

**Rule E1.** Any gate or doc that states a count must name the category (E.1) and its
verification method; counts from different categories are never summed or compared —
"21/21 headless" and "15/15 plugins" describe different sets, and a sentence may
carry both fractions only if each names its category.

| Count | Set | Verification method |
|---|---|---|
| 15 | standalone plugins (E.2 rows 1-15) | audit-grep, `check_plugin` over `plugins/*/` (audit.sh:79-80); shape-only — two verdicts disproven by execution (03 C.1) |
| 8 | host-embedded features | source layout (features/ + updates/) cross-checked with roadmap:32-44; 0/8 headless (profile partial via `qol sync`) |
| 5 | consumer CLIs | audit-grep, `check_bin` (audit.sh:83-87); 4 of 5 rows share one manifest (apps/qol-tray/Cargo.toml:9-22) — a single `qol-headless` dep yields four "yes" rows |
| 1 | host surface (qol-tray) | executed in guest (03 C.1) |
| **21** | audit.sh total = 15 plugins + 1 qol + 1 qol-guest-runner + 4 app rows | audit-grep only (audit.sh:79-87, 91-93); the 8 host features are **not** in the 21 |
| 18 | mock bins | docs/headless-cli-mock/bins/ listing; doc artifact — never a gate count |
| 13 | guest-verifiable plugin set on Linux | 15 − 2 bundle exclusions (E.5) |

## E.5 Inventory-vs-bundle discrepancy (confirmed, expected)

Guest `/run/qol-payload/plugins` had **13** dirs (observed in a guest session; payload
root is `/run/qol-payload`, tools/qol-cli/src/commands/env/dev_session.rs:19-20) against
the roadmap's 15 plugins (roadmap:9). Mechanism confirmed in-tree: the dev bundle stages
only buildable plugins — `scan_buildable_plugins` (libs/qol-workspace/src/lib.rs:277-292)
skips reserved ids (`plugin-template`, libs/qol-conventions/src/lib.rs:263) and
platforms other than the current one (`keyremap` is macos-only,
plugins/keyremap/plugin.toml:8; eligibility at libs/qol-workspace/src/lib.rs:263-268).
On Linux: 15 − template − keyremap = **13** — the expected payload, not a packaging
bug. Consequence: "15/15 plugins headless" is a **source-shape** claim; guest execution
covers only 13 on Linux, so "all 15 verified in a guest" is false —
guest-verification statements must name the 13-unit subset (or that OS's equivalent).

## F. Trace coverage for new workflows

Phase-1 workflows cross persistence and process boundaries (one-shot CLI →
config store; IPC CLI → tray → plugin manager / auth / updater / task runner),
so each needs decision-level `qol_runtime::probe!` targets before shipping.
Precedent: `CLI_SESSIONS_RECON` emits per-pane evidence and transitions
(plugins/cli-sessions/src/daemon/reconcile.rs). Required targets (PROPOSED):

| Target | Emits | Workflow |
|---|---|---|
| `HOST_CMD_*` | path taken (one-shot vs IPC), command id, exit class | every `qol-tray <host-feature>` command |
| `PROFILE_*` | export/import phases, bundle hash, guard acquisition, reload | profile export/import/backup |
| `PLUGIN_STORE_*` | install/update/remove phases, staging dir, reload notify | plugin store commands |
| `UPDATES_*` | check result, apply phases, artifact identity | updates check/apply |
| `TASK_RUNNER_*` | action id, spawn, completion, timeout | task list/run/status |
| `DAEMON_SPAWN_*` | env-gate verdict, listener adoption, bind outcome | the D.3 regression test |

Rule: entry probes record which dispatch path ran (one-shot vs IPC) — a
mislabeled path is a false repro (qol-dev-environments rule). Decision lines
(`phase=… outcome=…`) beat screenshots; every mutating command emits
`phase=reload outcome=…` before exiting 0 (B.5).

## Roadmap amendment (replaces roadmap §6 phase 0/1 boundary)

| Phase | Work | Criterion |
|---|---|---|
| 0 | Audit script + this doc + the freeze spec | measure exists, gateable exit code (done) |
| **0.5** | **Contract freeze: land this spec; normalize exit codes (B.1); add the executable gate `tools/qol-contract-gate` (C.2–C.6) with `daemon-spawn` (D.3); fix the task-runner env gate (D.4); rename ide-checkout display name to "IDE Checkout" (E.3.3)** | **gate runs green on the 13-unit guest set with the 8 host features listed NOT_YET; inventory counts (E.4) quoted with categories** |
| 1 | 8 host-feature command surfaces per A (order: plugin-store → profile → theme/mode → updates → task → auth → launcher-apps) | 8/8 host features COVERED in the ledger (C.2); each command obeys B (timeouts, locking, reload, destructive policy) |
| 2 | UI-adapter hardening: UI layers consume headless APIs only; testable dependency rule (no `features/*` imports in UI beyond the command surface) | `qol doctor` headless-contract check group green; dependency rule enforced by the gate |
| 3 | `qol doctor` "headless contract" check group surfaces C.1–C.6; audit.sh demoted to the no-build pre-filter | gate fails on any regression |
| 4 | `qol plugin new` scaffolder; new ecosystem features headless-first | no new feature ships without a command |

## Decisions (the frozen rules)

1. **A** — All 8 host-feature commands ship on the `qol-tray` front door; `qol`
   keeps `sync`. Transport per command as tabled in A: read-only file/network
   queries are direct one-shot; anything touching live tray state is
   authenticated IPC to a running tray and refuses ("qol-tray is not running",
   exit 1) when down; no controlled autostart, no helper process.
2. **B1** — Usage errors exit 64 on every binary; qol-tray
   `Invocation::Invalid` migrates 2 → 64 in Phase 1; 0/1/2 keep
   success/runtime/failure-refusal; doctor aggregates stay 0/1/2.
3. **B2** — Host `--json` output uses `{ok, command, result|error{class,
   message, details}}`; doctor reports keep their existing unwrapped shape.
4. **B3** — Every command has a deadline (30 s host / 5 s doctor); a doctor
   command with no output within the timeout is a FAILURE with no env
   exception; unavailable hardware/environment is a structured warn/fail,
   never a hang.
5. **B4** — Config mutations take `profile_config_write_guard` for the whole
   mutation (blocking RwLock, never across long I/O); named locks for
   non-config state.
6. **B5** — Mutating commands reload and notify before exit 0; `config
   show/get` never reloads.
7. **B6** — Destructive commands: `--yes`/TTY confirm, idempotent,
   preview-able, backup + rollback per the class table, wait for completion.
8. **C** — Three-layer gate (structural audit → executable smoke → manifest
   action→argv) + host-feature ledger; `tools/qol-contract-gate` (Rust);
   env classification with the timeout rule having no exception.
9. **D** — Every daemon honors one of the two env-gate mechanisms; bare
   invocation never daemonizes or blocks; task-runner gap fixed in Phase 1;
   silent exit-0 rows converge on rc=1 + structured stderr.
10. **E** — Inventory is the single source of truth; every count names its
    category and verification method; guest-verification statements name the
    13-unit subset on Linux.
