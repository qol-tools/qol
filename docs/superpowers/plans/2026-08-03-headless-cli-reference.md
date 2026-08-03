# qol headless CLI reference — every command, every binary

Date: 2026-08-03

One table per binary. Every command row lists: handler, --json, contract
coverage. Contract columns: **A**bout, **U**sage, **D**etail, **O**utput,
**E**xit-behaviour. `✓` = present, `—` = missing, `json` = has `run_json`.

## 1. Plugin CLIs

### alt-tab

App id `alt-tab`, bin `alt-tab`, `.about("Switch desktop windows…")`, default → `daemon`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `daemon` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | no-args default |
| `--show` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | flags `--settings`/`--kill` from extra args |
| `--show-reverse` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `--settings` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `--kill` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 5 checks |

### bluetooth

App id `plugin-bluetooth`, bin `plugin-bluetooth`, `.about("Inspect and reliably reconnect…")`, default → `list`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `list` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | |
| `enable_adapter` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | |
| `disable_adapter` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | |
| `search` | run_plain_text | json | ✓ | ✓ | ✓ | ✓ | ✓ | Ctrl+C stops early |
| `stop_search` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `pair` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | takes address arg |
| `trust` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | takes address arg |
| `untrust` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | takes address arg |
| `connect` | run_plain_text | json | ✓ | ✓ | ✓ | ✓ | ✓ | takes address arg |
| `disconnect` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | takes address arg |
| `remove` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | takes address arg |
| `reconnect` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | |
| `reconnect_trusted` | run_plain_text | json | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `host_fixes` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | |
| `apply_host_fix` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | takes fix-id arg |
| `settings` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 6 checks |

### cli-sessions

App id `cli-sessions`, bin `cli-sessions`, `.about("Track live terminal sessions…")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | alias `daemon` |
| `open` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | signals daemon or starts |
| `next` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | best-effort delivery |
| `snapshot` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 7 checks |

### controllers

App id `plugin-controllers`, bin `plugin-controllers`, `.about("Inspect connected game controllers…")`, default → `status`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `apply_fixes` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | privileged (pkexec) |
| `status` | run_plain_text | json | ✓ | ✓ | — | ✓ | ✓ | |
| `settings` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 3 checks |

### ide-checkout

App id `task-runner`, bin `task-runner`, `.about("Run the Task Runner checkout daemon…")`, default → `daemon`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `daemon` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `status` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | probes daemon, desktop notification |
| `doctor` | doctor_checks | json | — | — | — | — | — | 7 checks |

### keyremap

App id `keyremap`, bin `keyremap`, `.about("Run and control native key…")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `reload` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | alias `--reload` |
| `kill` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | alias `--kill` |
| `doctor` | doctor_checks | json | — | — | — | — | — | 4 checks |

### launcher

App id `launcher`, bin `launcher`, `.about("Search installed applications…")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `--show` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `--settings` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `--kill` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | |

### lights

App id `plugin-lights`, bin `plugin-lights`, `.about("Control Zigbee lights…")`, default → `launch`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `launch` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | no-args default; runtime-entrypoint |
| `daemon` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | alias `run` |
| `toggle_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `on_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `off_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `brighter_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `dimmer_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `warmer_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `cooler_main` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `preset_1`…`preset_8` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | 8 commands |
| `pair` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `stop_pair` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `set_color_main` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `set_brightness_main` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `set_colortemp_main` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `reload` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `settings` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | |

### os-themes

App id `plugin-os-themes`, bin `plugin-os-themes`, `.about("Run cursor effects…")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `toggle_theme` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `settings` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `kill` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | |

### pointz

App id `pointzerver`, bin `pointzerver`, `.about("Run and control the PointZ…")`, default → `server`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `server` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `action` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | alias `--action`; sub: `kill` `settings` `begin_pairing` `ping` `connection_status` `connection_info` |
| `kill` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | direct legacy |
| `settings` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | direct legacy |
| `begin_pairing` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | direct legacy |
| `ping` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | direct legacy |
| `connection_status` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | direct legacy |
| `connection_info` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | direct legacy |
| *legacy* | fallback | — | ✓ | ✓ | — | ✓ | ✓ | fallback_command |
| `doctor` | doctor_checks | json | — | — | — | — | — | |

### qol-shot

App id `qol-shot`, bin `qol-shot`, `.about("Capture screenshots…")`, default → `record`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `record` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `screenshot` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `copy` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `copy-path` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `preview` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | optional path arg |
| `settings` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 6 checks |

### qol-voice

App id `qol-voice`, bin `qol-voice`, `.about("Run provider-neutral speech recognition…")`, default → `session status`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `session` | sub | — | — | — | — | — | — | parent; sub: `start` `stop` `status` `events` |
| `session start` | run_result | json | ✓ | ✓ | — | — | — | |
| `session stop` | run_result | json | ✓ | ✓ | — | — | — | |
| `session status` | run_result | json | ✓ | ✓ | — | — | — | |
| `session events` | run_streaming | — | ✓ | ✓ | — | — | — | |
| `listen` | run_result | json | ✓ | ✓ | — | — | — | |
| `audio` | sub | — | — | — | — | — | — | parent; sub: `devices` `probe` |
| `audio devices` | run_result | json | ✓ | ✓ | — | — | — | |
| `audio probe` | run_result | json | ✓ | ✓ | — | — | — | |
| `stt` | sub | — | — | — | — | — | — | parent; sub: `providers` |
| `stt providers` | run_result | json | ✓ | ✓ | — | — | — | |
| `assistant` | sub | — | — | — | — | — | — | parent; sub: `request` |
| `assistant request` | run_result | json | ✓ | ✓ | — | — | — | |
| `settings` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 8 checks |

### removeapp

App id `removeapp`, bin `removeapp`, `.about("Inspect installed applications…")`, default → `open`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `open` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `scan` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `remove` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | flags: `--dry-run` `--yes` `--force` `--quit` `--package` `--brew` `--trash-anyway` |
| `doctor` | doctor_checks | json | — | — | — | — | — | 4 checks |

### template

App id `plugin-template`, bin `plugin-template`, `.about("Run the canonical qol-tray plugin scaffold.")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `settings` | run_plain_text | — | ✓ | ✓ | — | ✓ | ✓ | |
| `doctor` | doctor_checks | json | — | — | — | — | — | 2 checks |

### window-actions

App id `window-actions`, bin `window-actions`, `.about("Move, resize, minimize…")`, default → `daemon`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `daemon` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | alias `run` |
| `snap-left` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | ordinary |
| `snap-right` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `snap-bottom` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `maximize` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `minimize` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `restore` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `center` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `move-monitor-left` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `move-monitor-right` | run_result | — | ✓ | ✓ | — | ✓ | ✓ | |
| `glide-left` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | continuous |
| `glide-right` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | continuous |
| `glide-up` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | continuous |
| `glide-down` | run_result | — | ✓ | ✓ | ✓ | ✓ | ✓ | continuous |
| `doctor` | doctor_checks | json | — | — | — | — | — | 5 checks |

## 2. Host auxiliary CLIs

### qol-tray-install

App id `qol-tray-install`, bin `qol-tray-install`, `.about("Install or uninstall…")`, default → `install`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `install` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | flags: `--source` `--workspace` `--skip-shell-hook` `--dev` |
| `uninstall` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| *legacy* | fallback | — | ✓ | — | — | — | — | fallback_command |
| `doctor` | doctor_check | — | — | — | — | — | — | 1 check |

### qol-tray-migrate

App id `qol-tray-migrate`, bin `qol-tray-migrate`, `.about("Run QoL Tray config… migrations.")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | `--config-dir` `--post-auth` |
| *legacy* | fallback | — | ✓ | — | — | — | — | fallback_command |
| `doctor` | doctor_check | — | — | — | — | — | — | 1 check |

### qol-tray-doctor

App id `qol-tray`, bin `qol-tray-doctor`, `.about("Quality of Life Tray host.")`, doctor_aggregate_provider.

When invoked as `qol-tray-doctor` (not `qol-tray`):

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `doctor` | aggregate | json | — | — | — | — | — | aggregates all plugin doctors |
| `check` | run_json | json | ✓ | ✓ | — | ✓ | ✓ | legacy host check; `--id` `--quick` |
| `fix` | run_json | json | ✓ | ✓ | — | ✓ | ✓ | legacy fix; `--id` `--apply-host-fixes` |

When invoked as `qol-tray` the same app registers `exec` and `open` instead of `check`/`fix`:

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `doctor` | aggregate | json | — | — | — | — | — | |
| `exec` | — | — | ✓ | ✓ | — | ✓ | ✓ | `qol-tray exec <plugin-id> <action-id>`; dispatches through host daemon |
| `open` | — | — | ✓ | ✓ | — | ✓ | ✓ | `qol-tray open <route>`; opens in-app route |

### qol-tray (host daemon)

The `qol-tray` headless entry is the same HeadlessApp as qol-tray-doctor but the
binary entry point (`apps/qol-tray/src/app/mod.rs`) classifies args into seven
invocations before the daemon starts:

| Invocation | Args | Dispatches to |
|---|---|---|
| Daemon | (none) | host daemon |
| Help | `help` | HeadlessApp help |
| Version | `--version` | prints version |
| WriteMode | `--write-mode=<n>` | dev write-mode selector |
| Headless | `doctor [--json]`, `* help` | host_cli HeadlessApp (doctor/exec/open) |
| Exec | `exec <target> <action>` | daemon IPC action dispatch |
| Open | `open <route>` | in-app route forwarder |
| UrlCourier | `__url-courier qol://<route>` | macOS URL forwarding |
| Url | `qol://<route>` | direct URL routing |
| Invalid | everything else | usage error |

Also: settings surface boot (hidden host arg) and process-tree guardian entry
(gated by env var) are internal headless modes, not user-facing commands.

## 3. Tool CLIs

### qol

App id `qol`, bin `qol`, `.about("Build, inspect, diagnose, and run…")`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `setup` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `dev` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | flags: `--base` `--no-plugins` |
| `env` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | sub: `list doctor up image cancel runs down shot exec drag` |
| `flow` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | sub: `run runs` |
| `emu` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | 20 subcommands |
| `cat` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | flags: `--no-less` `--plain` `--color=` |
| `build` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `check` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | flag: `--staged` |
| `clean` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `install` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `sync` | run_json | json | ✓ | ✓ | ✓ | ✓ | ✓ | only command with JSON handler |
| `trace` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `trace-rs` | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `doctor` | doctor_provider | json | — | — | — | — | — | delegates to host-aggregate front door |

### qol-guest-runner

App id `qol-guest-runner`, bin `qol-guest-runner`, `.about("Serve the guest-control channel…")`, default → `run`.

| Command | Handler | --json | A | U | D | O | E | Notes |
|---|---|---|---|---|---|---|---|---|
| `run` | run_plain_text | — | ✓ | ✓ | ✓ | ✓ | ✓ | `--device` `--identity` `--run-id-path` |
| `doctor` | doctor_checks | json | — | — | — | — | — | 2 checks |

## 4. Contract patterns observed

What every binary gets right:

- **Thin entrypoints.** Every `main.rs` is ≤59 lines, delegates to `cli::exit_code`.
- **`HeadlessApp` everywhere.** Every binary constructs a `qol_headless::HeadlessApp` with `.about()`, registers commands, and attaches doctor checks. No raw argparse.
- **`help`/`help <cmd>`/`<cmd> help`** equivalent in all 21 test suites that check it.
- **`--json doctor`** stable output in both flag positions (`--json doctor` / `doctor --json`), verified by test.
- **Doctor checks lazy.** Tests confirm `help` and `doctor` never invoke operational handlers.
- **JSON rejection before execution.** `--json <cmd>` without a `run_json` handler returns `EXIT_USAGE` before side effects — verified per plugin.
- **`default_command`** on every HeadlessApp — no-args has deterministic behaviour.

Pattern variance worth noting:

| Concern | Pattern | Binaries using it |
|---|---|---|
| Handler style | `run_plain_text` with `reject_args()` | bluetooth, qol-shot, template, guest-runner |
| | `run_result` with `CommandResult` | most others (13 of 21) |
| | `run_streaming` | qol-voice `session events` |
| Doctor registration | `doctor_checks(Vec<DoctorCheck>)` | 18/21 |
| | `doctor_provider` (lazy) | qol CLI |
| | `doctor_aggregate_provider` | qol-tray-doctor |
| | `doctor_check(single)` | qol-tray-install, qol-tray-migrate |
| Command args | `reject_args()` / `no_args()` helper | bluetooth, keyremap, qol-voice, lights-daemon |
| | `context.args()` forwarded directly | most others |
| Legacy compat | `fallback_command` | pointz, qol-tray-install, qol-tray-migrate |
| | argv normalization (`normalize_legacy_argv`) | launcher, qol-tray-install, qol-tray-migrate |
| | `.alias()` for legacy names | cli-sessions (`daemon`→`run`), keyremap (`--reload`/`--kill`), lights (`run`→`daemon`), window-actions (`run`→`daemon`) |
| Subcommands | `.subcommand()` for nested trees | qol-voice (session/audio/stt/assistant), pointz (action sub) |
| JSON surface | dedicated `run_json` handler | bluetooth (most cmds), controllers (status), qol (sync), qol-tray-doctor |
| | no `run_json` on any operational cmd | alt-tab, cli-sessions, ide-checkout, keyremap, launcher, lights, os-themes, pointz, qol-shot, removeapp, template, window-actions, install, migrate, guest-runner |
| Contract gaps | **D**etail column blank | common; 13/15 plugins miss on some cmds |
| | **O**utput / **E**xit_behavior blank on subcommand parents | qol-voice, os-themes, pointz (parents registered as `Command` not `subcommand`) |

## 5. Where plugins differ from the ideal contract

This is the "qol flavor" gap analysis — where existing binaries are inconsistent
with each other, not necessarily wrong.

### Missing JSON support on operational commands

Only **bluetooth** and **controllers** (partially) offer `--json` on operational
(non-doctor) commands. qol-voice has JSON on its hierarchical commands but via
its own internal dispatch, not through `Command::run_json`. Every other plugin
rejects `--json` on every operational command.

This is a design choice — not every command has a stable structured output — but
the qol-headless contract has `run_json` / `run_streaming` / `run_result` and
only `run_json` is gated. Plugins that already produce structured output
(removeapp's JSON removal plan, qol-shot's screenshot path, lights' device
state) could add `run_json` without changing behaviour.

### Unused contract slots

- **`usage`** — some use `format!("{BINARY_NAME} {name}")`, some use a static
  string, some leave it to HeadlessApp auto-generation. Template for `<usage>`
  description is missing (qol-headless could provide a default).
- **`exit_behavior`** — present on every command, but varies widely in tone and
  specificity. Some say "Exits non-zero" (good), some say "Preserves the legacy
  best-effort zero exit behavior" (launcher — intentionally vague because it
  can't know).
- **`detail`** — optional, sparsely used. Some plugins use it for caveats
  (search: "Press Ctrl+C to stop"), some for migration notes
  (ide-checkout: "This preserves the plugin action used by qol-tray").
- **`output`** — always present. Good pattern: "No stdout on success" or
  descriptive output format.

### Subcommand registration

Only qol-voice and pointz use `Command::subcommand()` to build hierarchical
command trees. Others register flat command lists. The Subcommand API works well
but the help output for parent commands (e.g. `qol-voice help session`) lists
children but has no `output`/`exit_behavior` of its own — the parent command
itself has no handler, which is correct but the contract columns look empty.

### Legacy compatibility layers

Six plugins carry legacy argv normalization: launcher, pointz, window-actions,
alt-tab, keyremap (aliases), lights (alias). Three host binaries have
`fallback_command` + `normalize_legacy_argv`. These are necessary for existing
qol-tray integration but add complexity to the CLI surface (`--show` vs `show`,
flag-priority resolution, ignored trailing args). Every legacy layer has test
coverage confirming exact routing.

### Doctor registration style

- Most plugins: `doctor_checks(vec![…])` — checks registered eagerly but
  evaluated lazily (the `DoctorCheck` takes a closure).
- qol CLI: `doctor_provider(|| Ok(vec![]))` — returns empty, delegates to
  `qol-tray-doctor` aggregate.
- qol-tray-doctor: `doctor_aggregate_provider(|| …)` — aggregates all plugin
  doctors.
- qol-tray-install / qol-tray-migrate: `doctor_check(<single>)` — single check,
  not a vec. This is a different API shape.

## 6. qol flavour — the target contract

What every binary SHOULD look like once aligned:

```
HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
    .about("<one-line summary>")
    .default_command(["<default-cmd>"])
    .command(Command::new("<cmd>")
        .about("<what it does>")
        .usage("<binary> <cmd> [args]")
        .detail("<caveat or extended behaviour>")           // optional
        .output("<stdout contract>")                         // required
        .exit_behavior("<when non-zero>")                    // required
        .run_result(|ctx| { … Ok(CommandResult::…) })
        // .run_json(|ctx| { … Ok(serde_json::Value) })     // when structured output makes sense
    )
    // … more commands, possibly .subcommand()
    .doctor_checks(vec![DoctorCheck::new("id", "desc", || …)])
```

The qol flavour additions over raw HeadlessApp:

1. **JSON where meaningful.** If a command produces data (not just side effects
   + human output), add `run_json`. The contract says: "A command supports JSON
   only when it declares a stable structured output contract." Track which
   commands genuinely have one.

2. **Consistent `usage` format.** `<BINARY> <command> [args]` — most already do
   this. The ones that use bare command names can be normalised.

3. **`detail` for non-obvious behaviour.** Ctrl+C stops search early, fallback
   daemon startup, best-effort delivery, continuous action phases — these are
   all good uses of `detail`.

4. **Explicit `reject_args()` for arg-less commands.** bluetooth, keyremap, and
   qol-voice already do this; it prevents silent argument ignoring.

5. **One doctor registration shape.** `doctor_checks(vec![…])` for plugins,
   `doctor_aggregate_provider` for the aggregate host binary, `doctor_provider`
   for front doors. `doctor_check(single)` is acceptable for mini binaries
   (install/migrate) but `doctor_checks(vec![one])` would be more uniform.

6. **Legacy layers isolated.** argv normalization, `fallback_command`, and
   aliases are documented and tested — they're a fact of life for existing
   binaries. New plugins starting from template don't need them.

## 7. qol-tray host feature gap (reminder)

Same as the roadmap doc — eight host-embedded features need headless command
surfaces following the same contract:

plugin-store, profile export/import/backup, task-runner, theme, mode, auth,
launcher-apps, updates.
