# Repository inventory — 2026-09-30

The baseline is `b28d8825b0a7bf6c45c7fdc613f6c5442a359fa2`: **3,793 tracked files, 35,017,609 bytes of file content, and 61 workspace packages** (one application, 40 libraries, 18 plugins, two tools). This is a file/folder inventory and a targeted ownership audit, not a claim that every function was manually reviewed.

Every tracked file was hashed. The scan found **74 byte-identical groups**, representing **190,224 repeated bytes** before consolidation. Most are deliberate packaging, platform, or fixture repetition. Names alone were not used to declare files redundant.

## Reproduce the inventory

```sh
node verify/repository-inventory.mjs
node --test verify/repository-inventory.test.mjs
```

Requires Git, Node with `util.parseArgs`, and Cargo. Package discovery uses `cargo metadata --no-deps --locked --offline`; it does not build or fetch dependencies. Run from the repository root, or pass `--root <checkout>`. `--out <directory>` selects an ignored directory inside the checkout or an external directory.

The default output is `verify/reports/repository-inventory/`:

| Artifact | Contents |
| --- | --- |
| `files.tsv` | Every tracked and nonignored untracked file: path, Git status, file type, package owner, role hint, bytes, text lines, SHA-256, binary hint |
| `directories.tsv` | Every containing directory, with recursive file and byte totals |
| `packages.tsv` | Every workspace package and its manifest description |
| `report.json` | The same inventory, Cargo targets and internal dependency edges, identical-file groups, repeated-code candidates with line locations, ignored boundaries, detected ELF core dumps, and scan errors |

Reports record the current commit and dirty status. File hashes describe working-tree bytes; the `object` field is the Git index object. Role/kind labels are hints. Repeated-code candidates use consecutive nonblank source lines with indentation and whole-line comments removed, retaining identifiers and literals. They can overlap, include tests, or cross syntactic boundaries; they are leads for review, not a duplication percentage. Vendor/minified code is omitted only from the repeated-block analysis, not the file inventory or exact hashing.

Symlinks are inventoried by link text without following their targets. Git internals and ignored build/report trees are listed as boundaries rather than recursively hashed. Missing tracked files are reported and make the command fail; stage deliberate deletions before taking the final inventory. This avoids presenting a partial scan as complete.

## Root map

Counts below describe the baseline, before this PR's removals and new audit tooling.

| Folder | Tracked files | Responsibility |
| --- | ---: | --- |
| `apps/` | 1,153 | Tray host, installer, doctor, migrations, native/web settings and embedded assets |
| `libs/` | 967 | Shared Rust capabilities and contracts; includes one accidental core dump removed by this PR |
| `plugins/` | 975 | 18 independent plugin release units, contracts, native/web surfaces, assets and tests |
| `tools/` | 264 | Developer CLI and guest-side control runner |
| `docs/` | 205 | Cross-project specs, plans, audit notes, runnable mocks and memory research |
| `flows/` | 19 | Disposable guest environment definitions, provisioning and image identity |
| `vendor/` | 161 | Patched GPUI, global-hotkey and ravif sources |
| `.github/` | 26 | CI/release workflows, shared Rust setup, release/affected-package helpers and tests |
| `.githooks/` | 8 | Commit/push checks, lockfile merge driver, single-source guard and hook tests |
| `.cargo/` | 1 | Cargo aliases, linking and build environment |
| `.claude/` | 4 | Tracked agent integration state/instructions |
| `.config/` | 1 | Repository tool configuration |
| `verify/` | 1 | Existing session-relay guest verification workflow |
| Root files | 8 | Workspace/lockfile, README/license, Git metadata rules and tool configuration |

The root Cargo workspace is the source of truth for membership. Other Cargo manifests belong to the root workspace definition, three vendored packages, and the independent `docs/research/qol-memory/tier1` experiment. They are not additional members of the 61-package workspace.

## Package ownership map

File counts include each package's docs, tests, assets and contracts. All nested folders and individual paths are available in the generated directory/file inventories.

| Folder | Files | Package responsibility |
| --- | ---: | --- |
| `apps/tray/` | 1,153 | Quality of Life Tray - Pluggable system tray daemon for utility scripts |
| `libs/agent-homes/` | 2 | Single source of truth for agent harness homes (Claude Code, codex, kimi, pi) |
| `libs/app-icon/` | 7 | Read OS app icon bytes (RGBA) by bundle id or pid |
| `libs/apps/` | 22 | Shared app inventory and desktop entry helpers for qol-tools |
| `libs/artifact/` | 5 | Inspection and verification of deployable QoL artifact identity |
| `libs/audio/` | 25 | Bundled native PulseAudio control client for qol-tools |
| `libs/bluetooth-control/` | 4 | Shared Bluetooth peripheral identity and reconnect holds for qol-tools plugins |
| `libs/build-identity/` | 6 | Canonical QoL native artifact identity emission and source provenance |
| `libs/cinnamon/` | 5 | Cinnamon shell Eval session over D-Bus for qol-tools |
| `libs/color/` | 5 | Color parsing utilities for qol-tools |
| `libs/config/` | 44 | Versioned plugin configuration contract for qol-tools |
| `libs/conventions/` | 15 | Single source of truth for cross-process QoL Tray constants (port, sockets, env vars, URLs) |
| `libs/dev-build/` | 48 | Shared dev-linked plugin registry, build, and fingerprint tracking for qol-tray and qol-cli |
| `libs/dev-env/` | 20 | Shared development environment registry, inventory, and lifecycle contracts for qol-tools |
| `libs/dev-guest/` | 2 | Typed host-to-guest control protocol for disposable qol development environments |
| `libs/dev-orchestrator/` | 9 | Typed durable development-environment worker orchestration for qol-tools |
| `libs/frecency/` | 5 | Frequency/recency ranking for qol-tools |
| `libs/fs/` | 7 | Atomic file persistence primitives for qol-tools |
| `libs/gpui/` | 216 | GPUI windowing helpers for resident qol-tray plugins: ghost popups, monitor tracking, runtime event bridge |
| `libs/headless/` | 10 | Headless CLI contract helpers for qol-tools applications and plugins |
| `libs/host-fixes/` | 40 | Host policy, mutation journals and platform repair operations |
| `libs/host-session/` | 2 | Resident host session snapshots and mutation ownership |
| `libs/hotkeys/` | 14 | Shared hotkey grammar and keycode adapters for qol-tools |
| `libs/log/` | 9 | Log directory resolution and bounded file retention for qol-tools |
| `libs/mcp/` | 5 | Transport-agnostic JSON-RPC handler for the MCP tools surface |
| `libs/migrations/` | 61 | On-disk format migrations between qol-tray releases |
| `libs/peers/` | 121 | Core peer identity and mutually authenticated transport foundation |
| `libs/platform/` | 19 | Platform detection and capabilities for qol-tools |
| `libs/plugin-api/` | 32 | Plugin manifest and restore-claim wire contract for qol-tray plugins |
| `libs/plugin-daemon/` | 16 | Resident-process OS plumbing for qol-tray plugins: unix-socket IPC, activation policy, foreground probe |
| `libs/process/` | 19 | Cross-platform process lifecycle primitives for qol-tools |
| `libs/profile-sync/` | 11 | Shared profile sync engine for qol-tray and the qol CLI |
| `libs/runtime/` | 57 | Runtime protocol and client for QoL Tray platform state |
| `libs/search/` | 5 | Fuzzy search algorithm for qol-tools |
| `libs/terminal-sessions/` | 52 | Backend-neutral live terminal session discovery and interaction |
| `libs/theme/` | 9 | First-party theme source of truth for qol-tools Rust and CSS surfaces |
| `libs/watch/` | 3 | Filesystem change notification primitives for qol-tools |
| `libs/windowing/` | 14 | Shared window identity, geometry, and window operation contract for qol-tools |
| `libs/workspace-hack/` | 4 | workspace-hack package, managed by hakari |
| `libs/workspace/` | 3 | Shared workspace and plugin source discovery for qol-tools |
| `libs/zigbee/` | 14 | Zigbee Network Processor (ZNP) serial stack for qol-tools |
| `plugins/alt-tab/` | 91 | Alt-Tab window switcher plugin for QoL Tray |
| `plugins/bluetooth/` | 47 | Bluetooth device control, pairing and host integration |
| `plugins/cli-sessions/` | 104 | Always-on-top overview of live CLI sessions (Claude Code, Codex, any command) for QoL Tray |
| `plugins/controllers/` | 27 | Controller discovery, profiles, gamepad input and fixes |
| `plugins/ide-checkout/` | 27 | Task runner and Python daemon supervision |
| `plugins/keyremap/` | 50 | Keyboard and mouse remapping plugin for QoL Tray |
| `plugins/launcher/` | 115 | Application and file launcher plugin for QoL Tray |
| `plugins/lights/` | 59 | Lighting backends, Zigbee and live color control |
| `plugins/memory/` | 84 | Memory capture, retrieval, distillation and answer verification |
| `plugins/monitor/` | 45 | Display inventory, arrangement, modes, brightness and color state |
| `plugins/os-themes/` | 61 | OS appearance, cursor effects and theme settings |
| `plugins/pointz/` | 29 | PointZerver - Headless server for remote PC control from mobile devices |
| `plugins/removeapp/` | 27 | Uninstall an app and its leftovers for QoL Tray |
| `plugins/shot/` | 79 | Screenshot/recording capture, overlays, editing and output |
| `plugins/sound/` | 21 | Audio device, volume, routing and repair actions |
| `plugins/template/` | 17 | Source template and contract baseline for new plugins |
| `plugins/voice/` | 56 | Voice capture/transcription and terminal delivery policy |
| `plugins/window-actions/` | 36 | Window geometry, snapping, movement and glide actions |
| `tools/cli/` | 256 | qol-tray dev orchestrator (replaces make) |
| `tools/guest-runner/` | 8 | Guest-side desktop-session control runner for disposable qol development environments |

## Consolidations in this PR

1. **One atomic file publisher.** `libs/fs/src/lib.rs` already owns unique temporary filenames, replacement, permission preservation and failure cleanup. Seven writers were independently using fixed staging names: fingerprint sidecars in `libs/dev-build/src/sidecar.rs`, and session records in CLI `sessions/{bridge,fork,last_send,spawn,watch}.rs` (`bridge` has two writers). They now call `qol_fs::atomic_write`. Their JSON formats, record paths, caller locks and best-effort/error-return policies stay with their existing owners. Fixed names could collide between writers and left staging files behind when publication failed. The shared helper already has concurrent-writer coverage; new consumer regressions exercise failed-publication cleanup.
2. **One ADR copy.** `apps/tray/docs/adr/TRAY-14-auto-wire-shell-hook-from-installer-v2.md` was byte-identical to the non-`v2` file (2,780 bytes), with no incoming repository references to the duplicate path. Keep the original and remove the duplicate.
3. **Remove a generated artifact from the tracked tree.** `libs/host-fixes/core` was a 2,850,816-byte AArch64 ELF core dump from a test process. It is neither source nor a referenced fixture. Remove it and ignore that exact path, avoiding a broad `core` rule that would hide legitimate `src/core/` directories. This changes the current tree; it does not rewrite Git history.

## Remaining overlaps worth a separate change

These are confirmed repeated implementations or stale descriptions. None is a claim of a newly reproduced runtime failure. Locations below refer to the baseline; current line numbers can shift.

| Area | Evidence | Suggested owner and validation |
| --- | --- | --- |
| Profile-sync persistence | `apps/tray/src/features/profile/sync/service.rs:845,942` and `tools/cli/src/commands/sync/mod.rs:295,310` repeat Git-ignore setup and merged-profile writing. The host version additionally holds a runtime config mutation guard. | Move the shared file/schema operation into `qol-profile-sync`; retain the host's guard and invalidation lifecycle at the host. Exercise CLI and host merge/repair tests. |
| Terminal transcript metadata | `FileSignature` and `file_signature` repeat in `libs/terminal-sessions/src/cli/builtins/{claude,codex,kimi,pi}/metadata.rs`. | Share the file metadata primitive inside `qol-terminal-sessions`; keep harness-specific readiness, transcript parsing and cache policy separate. Exercise append/truncate/partial-transcript cases. |
| Image-import permissions | Identical `set_readonly` implementations in CLI `commands/emu/image_import/{reconcile,report,storage}.rs`. | Reuse the existing image-import storage owner and keep report finalization/cleanup semantics intact. |
| PointZ input translation | An 85-significant-line run of matching mouse/key/modifier behavior in `plugins/pointz/src/input/platform/linux.rs:193` and `macos.rs:249`, plus repeated modifier application. | Extract platform-neutral input policy while leaving native event dispatch and activation in adapters. Requires Linux/macOS input verification, not just text deduplication. |
| Monitor gamma session policy | Matching restore-result mapping and baseline adoption around `plugins/monitor/src/monitor/backends/cg_gamma.rs:410` and `x11_randr_gamma.rs:532`. Native LUT access differs. | Share the session/restore policy behind the existing backend seam; preserve foreign-LUT and resume/wake guarantees. Validate both backend suites. |
| CLI instruction ownership | `apps/tray/skills/qol-cli-commands/SKILL.md` contains command/source-path facts, including paths such as `tools/cli/src/dev_console.rs` that are now directory modules. CLI skills also live outside this repository. | Retire or redirect the repo-local skill after reconciling the skill package owner; derive command facts from current CLI help. External skills were not changed in this PR. |
| Memory research vs plugin | `docs/research/qol-memory/` has Node write/query/evaluation tooling; the Rust plugin has capture/distill implementations. `plugins/memory/README.md:21` still describes the plugin as read-only, while `src/cli.rs` exposes capture/distill. Doctor fixes still reference research `skills.mjs`. | Clarify the current write-path owner and update the README, then retire only scripts proven superseded by parity tests. The research directory is still referenced and is not safe to delete wholesale. |

Large navigation hotspots include `plugins/monitor/src/daemon.rs` (8,633 lines), CLI `sessions/watch.rs` (8,003), `flow.rs` (7,093), `sessions/spawn.rs` (6,986), and `libs/gpui/src/settings_panel/view/mod.rs` (6,755). These counts include tests. Size is a prompt to inspect responsibility boundaries, not evidence that a file should be mechanically split.

## Identical files retained

| Category | Baseline groups | Decision |
| --- | ---: | --- |
| Per-package licenses | 1 | Keep legal notices with independently packaged release units. |
| Platform adapters/stubs | 49 | Keep explicit target boundaries. Identical unsupported implementations do not establish shared runtime ownership. |
| Fixtures | 10 | Keep before/after migration and scenario fixtures. Two Pi terminal captures are byte-identical and are explicitly used for provider-error and spinner-stability expectations in `libs/terminal-sessions/tests/pi_stall_frames.rs`. |
| Build entrypoints | 2 | Most plugins contain only a call to the shared build-identity emitter. The implementation is already centralized. |
| Bundled web libraries | 4 | Tray and Keyremap ship identical Preact, hooks, htm and html binding files. Their import maps currently resolve package-local URLs. Any shared asset route/build change must preserve independent plugin packaging. |
| Documentation | 1 | Remove the redundant ADR copy in this PR. |
| Other | 7 | Includes tray icon copies consumed by native/web packaging, four identical Nordic keymap schemas, CLI workflow entrypoints, per-package ignores, and empty/autostart marker files. Preserve identities and packaging contracts until a consumer-aware change is validated. |

The native GPUI helper library, vendored GPUI engine, theme data, host web UI, and plugin-native surfaces have different responsibilities. Likewise `qol-config` (settings), `qol-plugin-api` (plugin contracts), and `qol-runtime` (runtime transport/state) overlap in vocabulary but are not interchangeable copies. `qol-dev-env`, `qol-dev-orchestrator`, `qol-dev-guest`, CLI emulation and the guest runner are separate control/lifecycle/protocol boundaries. No consolidation is proposed merely because their names are similar.

## Generated and local material

The main checkout also contains large ignored/local trees. At scan time `target/` occupied roughly 204.5 GiB and `reports/` 1.38 GiB. The nested memory research `tier1/target/` occupied 492 MiB, accounting for nearly all of `docs/`' on-disk allocation despite only about 2.6 MiB of tracked documentation/research content. An ignored Bluetooth executable occupied about 25.4 MiB. These are local disk allocations, not tracked source sizes or promised savings.

Other ignored boundaries include browser captures, Python/pytest caches, property-test regression output, local worktrees, `verify/reports/`, and excluded `tools/qol-memory/`. The inventory records these boundaries; this PR does not delete local builds, reports or worktrees.

## Validation

```sh
node --test verify/repository-inventory.test.mjs
node verify/repository-inventory.mjs
qol check --base origin/main --report verify/reports/consolidation-check.json
cargo test -p qol-fs --locked --offline
```

The inventory tests cover exact and partial duplicates, package ownership, ignored paths, unusual filenames, stable reruns, missing tracked files, ELF core recognition, and symlink boundaries. Rust verification covers the affected packages/dependents through the repository check planner. No desktop input, native display, plugin UI, or other runtime surface changes are included in this PR.
