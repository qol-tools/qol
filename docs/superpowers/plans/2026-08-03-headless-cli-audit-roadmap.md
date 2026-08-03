# Headless-CLI coverage audit and roadmap to 100%

Date: 2026-08-03

Goal: every feature is a headless CLI tool at its base; qol-tray orchestrates;
UI is an adapter. Regenerate the coverage check with
`docs/headless-cli-audit/audit.sh` (exits non-zero on any gap).

## 1. Plugin features (15/15 headless CLI)

All: `[runtime]` command + `capabilities.doctor` + action→argv mapping
(`catalog_runtime_args`), built on `qol-headless` (`help`/`doctor`/`--json`).

| Plugin | Headless CLI commands | Daemon | UI strategy |
|---|---|---|---|
| alt-tab | `daemon`, `--show`, `--show-reverse`, `--settings`, `--kill` | yes | retained GPUI picker |
| bluetooth | `list`, `search`, `stop_search`, `pair`, `trust`, `untrust`, `connect`, `disconnect`, `remove`, `reconnect`, `reconnect_trusted`, `host_fixes`, `apply_host_fix`, `enable_adapter`, `disable_adapter`, `settings` | yes | settings surface |
| cli-sessions | `run`, `open`, `next`, `snapshot` | yes | GPUI overview panel |
| controllers | `status`, `apply_fixes`, `settings` | yes | settings surface |
| ide-checkout | `daemon`, `status` | yes | settings surface |
| keyremap | `run`, `reload`, `kill` | yes | settings surface |
| launcher | `run` (default), `--show`, `--settings`, `--kill` | yes | retained GPUI popup |
| lights | `launch`, `daemon` + 22 action cmds (`toggle_main`, `on_main`, `off_main`, `brighter_main`, `dimmer_main`, `warmer_main`, `cooler_main`, `preset_1..8`, `set_color_main`, `set_brightness_main`, `set_colortemp_main`, `pair`, `stop_pair`, `reload`, `settings`) | yes | settings surface |
| os-themes | `run`, `toggle_theme`, `settings`, `kill` | yes | settings surface |
| pointz | `server`, `action {kill,settings,begin_pairing,ping,connection_status,connection_info}` | yes | settings surface |
| qol-shot | `record`, `screenshot`, `copy`, `copy-path`, `preview`, `settings` | yes | GPUI overlay/preview + native overlay |
| qol-voice | `session {start,stop,status,events}`, `listen`, `audio {devices,probe}`, `stt {providers}`, `assistant {request}`, `settings` | yes | web UI + GPUI |
| removeapp | `scan`, `remove`, `open` | no (on-demand) | GPUI removal UI |
| template | `run`, `settings` | no (scaffold) | scaffold |
| window-actions | `daemon` + 13 action cmds (`snap-left`, `snap-right`, `snap-bottom`, `maximize`, `minimize`, `restore`, `center`, `move-monitor-left`, `move-monitor-right`, `glide-left`, `glide-right`, `glide-up`, `glide-down`) | yes | settings surface |

## 2. Host-embedded features (0/8 headless — the entire gap)

| Feature | Capability today | Access | Headless status | Missing commands |
|---|---|---|---|---|
| plugin store | install / update / remove / list plugins | host IPC + HTTP + dev-links API | no | `plugin-store list\|install\|update\|remove` |
| profile | export / import / backup; sync | host UI; sync via `qol sync` | partial (sync only) | `profile export\|import\|backup` |
| task runner | run / supervise checkout tasks | host `features/task_runner`; guardian entry is spawned-internal | no | `task run\|list\|status` |
| theme | host UI theming | host | no | `theme get\|set` |
| mode toggle | dev-mode switch | host | no | `mode get\|set` |
| auth / github auth | OAuth state for profile sync | host UI | no | `auth status\|login\|logout` |
| launcher apps | app inventory (overlaps launcher plugin discovery) | host | no | `launcher-apps list` or fold into launcher |
| updates | qol-tray update check / apply | host | no | `updates check\|apply` |

## 3. Host surface layers (orchestration, not features)

tray, menu, hotkeys, shortcuts, settings surface, world canvas, native
notifications. `qol-tray` headless entry today: `doctor` (+`--json`), `help`,
`--version`, `exec <target> <action>`, `open <route>`, `--write-mode=`, URL
courier. Missing but mission-relevant: `hotkeys list` / `shortcuts list`
(state queries behind "qol-tray takes the hotkey back").

## 4. Headless CLIs that exist (consumers, 5/5)

| Binary | Commands |
|---|---|
| qol | `setup`, `dev`, `emu`, `env`, `flow`, `cat`, `build`, `check`, `clean`, `install`, `doctor`, `sync`, `trace`, `trace-rs` (+ hidden workers: process-guardian, flow-worker, image-import-worker) |
| qol-guest-runner | headless runner + doctor |
| qol-tray-install | headless install + doctor |
| qol-tray-doctor | aggregate doctor (host + every plugin `doctor --json`) |
| qol-tray-migrate | headless migrations |

## 5. Gap summary

- 21/21 standalone units headless (100%).
- 0/8 host-embedded features have user-facing headless commands.
- 1 partial: profile (sync headless via `qol sync`; export/import/backup not).

## 6. Roadmap

| Phase | Work | Criterion |
|---|---|---|
| 0 | Audit script + this doc (done) | measure exists, gateable exit code |
| 1 | Add 8 host-feature command surfaces on `qol-tray` / `qol` front door, qol-headless contract, order: plugin-store → profile export/import/backup → theme/mode → updates → task → auth → launcher-apps | 8/8 host features headless |
| 2 | UI-adapter hardening: UI layers consume headless APIs only; extend gallery-parity rule to plugin surfaces | no domain logic behind UI boundary |
| 3 | Grow audit.sh into CI gate + `qol doctor` "headless contract" check group (action→command mapping, `doctor --json` parses, `help` exit 0, host features covered) | gate fails on any regression |
| 4 | `qol plugin new` scaffolder; new ecosystem features headless-first | no new feature ships without a command |

## 7. Definition of 100%

- Every feature answers `help` + `doctor --json`; every capability is
  `<binary> <command>` under the qol-headless contract.
- Every host-embedded feature works headlessly without the tray UI.
- No domain logic in UI layers.
- `qol doctor` exercises the whole ecosystem headlessly.
