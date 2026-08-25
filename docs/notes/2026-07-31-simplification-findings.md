# qol-monorepo Simplification Findings (deterministically verified)

Investigation: 2026-07-31. Re-checked against the current tree: 2026-08-25. Workspace: 32 libs, 16 plugins, 1 app, 2 tools (51 workspace packages, `cargo metadata`). Every claim below was re-checked against the repo (grep/diff/Cargo.lock/workflow files); false claims are marked CORRECTED with evidence. Statuses: **TRUE** (verified), **FALSE** (corrected), **RESOLVED** (finding's fix shipped), **LOCKED** (deliberate design, 2026-07 audit).

Line numbers are hints; paths and symbol names are durable (same convention as `docs/plugin-contract.md`).

## VERIFIED TRUE - build time

| # | Finding | Evidence |
|---|---|---|
| B3 | CI compiles the world 3x on cold cache | `.github/workflows/ci.yml:134` clippy `--all-targets` (via `CLIPPY_ARGS`, `--workspace --all-targets` from `.github/scripts/affected_crates.py:109`), `:141` `cargo build --release`, `:202/:208` `cargo test`. Fix: drop standalone release build, run `cargo test --release` |
| B4 | `zbus` exists in qol-platform for one 52-line Cinnamon module; only window-actions consumes it | `libs/qol-platform/Cargo.toml:15` (zbus 5.16), `src/cinnamon/platform/linux.rs` (52 lines); consumers: `window-actions/src/platform/linux/system.rs:161`, `glide.rs:11,18`. qol-platform is a dep of qol-apps, qol-dev-env, launcher, qol-shot and window-actions; zbus pulls async-io/async-executor/futures-lite (cargo tree), not smol/async-std, and qol-cli does not depend on qol-platform at all. alt-tab carries its own zbus dep (`alt-tab/Cargo.toml:37`) for its own `preview_plane` cinnamon modules, so it does not consume qol-platform's. Moving dep + module drops the async-io tree from the four non-window-actions qol-platform consumers |
| B6 | Duplicated crate versions, all bump-compatible | Cargo.lock: png `0.17.16`/`0.18.1`, tungstenite `0.26.2`/`0.30.0`, getrandom `0.2.17`/`0.3.4`/`0.4.3`. Note: getrandom 0.2 persists via ring/rand_core/redox_users/nanorand/const-random-macro, not the voice stack |
| B7 | No `rust-toolchain.toml` | `ls rust-toolchain*` = none; 1030 packages in lockfile; `cargo tree --workspace --duplicates` = 2038 duplicate subtree lines (specific pairs in B6) |
| B2 | git2 vendored libgit2 + **vendored-openssl** (openssl compiled from source) | `apps/qol-tray/Cargo.toml:132-133` (linux) - **LOCKED**: comment says "vendored so the host needs no system git"; self-containment is deliberate. Alternative only if gix adoption is ever accepted |
| B5 | image dep in qol-gpui | **FALSE - CORRECTED**: `libs/qol-gpui/src/color_wheel.rs:506-510` builds `RenderImage::new(smallvec![image::Frame::new(...)])` - `image` is gpui's public API type (`impl Into<SmallVec<[Frame; 1]>>`), not type-only removable |

## VERIFIED TRUE - dead code (zero consumers, grep-verified)

| Item | Lines | Evidence |
|---|---|---|
| `qol-runtime::broker` listener half | 1,005 / 17 files | 0 production refs to `BrokerListener`/`bind_broker_listener`/`broker_socket_path` outside `src/broker/`; the only outside refs are 18 in `libs/qol-runtime/tests/broker_socket_path_structural.rs` (6 tests over `bind_broker_listener` + `broker_socket_path_for_uid`) - see action 4 to rework that file. `peer_cred` is used (by `local_ipc/platform/unix.rs`). ADR `libs/qol-runtime/docs/adr/RUNTIME-1-*` documents an unwired subsystem |
| `qol-plugin-api::restore` + `restore-rule` capability + `pane_field` | ~170 | 0 consumers of `RestoreClaim`/`restore-rule`/`pane_field` in plugins/apps/tools |
| Hand-rolled TOML mini-parser | 135 | `libs/qol-conventions/src/build/plugin_manifest.rs`; `toml` is a workspace dep (`Cargo.toml:56`) already used by sibling `qol-build-identity` (`emitter.rs:146`) - swap costs one workspace dep edge on qol-conventions |
| `apps/qol-tray/src/process/mod.rs` shim | 25 | i32-adapting shim over qol-process; inline-able |

## VERIFIED TRUE - merge candidates (consumer counts from Cargo.tomls)

| Merge | Lines | Consumers | Zero new edges? |
|---|---|---|---|
| `qol-frecency` -> launcher | 162 | 1 (launcher) | yes |
| `qol-app-icon` -> `qol-apps` | 527 | 2 (tray, alt-tab) | yes - both already dep on qol-apps |
| `qol-host-fixes` -> `qol-plugin-daemon` | 15,416 | 6 (bluetooth, controllers, monitor, os-themes, tray, qol-cli) | yes - all 6 already dep on qol-plugin-daemon; crate has grown ~37x since the 2026-07-31 investigation |
| `qol-migrations` -> qol-tray | 6,692 | 2 (tray, qol-profile-sync) | no - profile-sync would gain a tray edge |
| `qol-dev-orchestrator` -> qol-cli | 2,833 | 1 (qol-cli) | yes |
| `qol-color` -> `qol-theme` (optional) | 138 | 5 (qol-shot, lights, qol-theme, cli-sessions, qol-gpui) | leaf with zero deps (empty `[dependencies]`); already imported by qol-theme; low value either way |
| inline ide-checkout daemon infra | ~490 | - | `daemon/server.rs` is 485 lines of hand-rolled HTTP-over-TCP; 0 uses of `qol_plugin_daemon::daemon::start_listener`/`run_stateful_listener`; takeover.rs 5 lines |

## VERIFIED TRUE - stale / drift

| Finding | Evidence |
|---|---|
| Committed WASM bundle is stale and has **no regen path - it is now orphaned** | `apps/qol-tray/ui/wasm/qol_wasm_bg.wasm` last commit 2026-03-08 (`7427eb91f`); `libs/qol-search` code last touched 2026-07-10 (`e5bca69b1`); the cdylib glue crate `libs/qol-wasm` (crate-type `["cdylib","rlib"]`, deps wasm-bindgen + qol-search) was **deleted in `03bcfd512` (2026-08-22)** while the bundle remains committed; imported live by `apps/qol-tray/ui/components/CommandPalette.js:5`; 0 `wasm-pack` refs in any sh/yml/Makefile/md/toml under apps/tools/.github. **The stale bundle is now orphaned - no source crate exists to regen it** |
| `docs/plugin-contract.md` stale source paths | `:56-57` `plugin_config_handlers/` (actual: `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/`); `:60-61,:427` `runtime/server/socket/requests.rs` (actual: `apps/qol-tray/src/runtime/server/socket/platform/unix/requests.rs`); `:64` `plugin-cli-sessions/src/notify.rs` (actual: `plugins/cli-sessions/src/ui/notify.rs`) |
| `docs/plugin-contract.md:192` claims `libs/qol-config/docs/v1.md` is stale | **The note itself is stale**: v1.md documents all 11 field types (13 matches for qr_code/action/list/status/color/gamepad) |
| `docs/plugin-layout.md` stale qol-shot tree | `:93` `region_selector/platform/` and `:95` `saved_toast.rs` no longer exist; actual is single `ui/region_selector/mod.rs` |

## RESOLVED - findings whose fix shipped since 2026-07-31

| Claim | Resolution (verified) |
|---|---|
| B1: qol-voice ML stack (candle-core/nn/transformers, tokenizers, hf-hub) is **ungated**: unconditional deps, no `[features]` section | **Shipped**: `plugins/qol-voice/Cargo.toml:23-31` now has `[features] local-stt` gating candle-core/nn/transformers, hf-hub, tokenizers as optional `[target.'cfg(target_os = "linux")'.dependencies]` (`:34-39`); `sherpa-stt` gates sherpa-onnx (`:31,:38`). Landed in `68298d9ea` (2026-07-31). Rust-side gating surface is `transcribe/platform/linux/mod.rs:1-17` + `cli/doctor/platform/linux.rs:3-8`; `plugin.toml:11` enables both features for the plugin release build. The 11 ML `opt-level` overrides (candle*/gemm*/pulp, `Cargo.toml:136-167`) remain but only cost when the feature is enabled. The "getrandom 0.2 dies with the voice gate" parenthetical from the original row was wrong - 0.2.17 persists via other crates (see B6) |
| `qol-plugin-daemon::activation` dead code (65 / 5 files) | **Removed**: `libs/qol-plugin-daemon/src/activation/` deleted in `81da050c2` (2026-08-01, seven-crate removal scan; see sibling `docs/notes/2026-08-01-removal-scan-audit.md`); 0 refs to `plugin_daemon::activation` remain anywhere |

## CORRECTED - claims that were FALSE as stated

| Claim | Correction (verified) |
|---|---|
| `.githooks/commit-msg.test.sh` orphaned from CI (0 refs in .github, runs only manually) | **FALSE**: `.github/workflows/ci.yml:49` runs `bash .githooks/commit-msg.test.sh` ("Test commit-msg hook" step, wired in `388273f04` 2026-08-07) |
| Safe-identifier rule in 3 copies, tray drops null-byte checks | Single impl in `qol-plugin-api` (`src/manifest/validation/command_rules.rs:36`); tray delegates: `paths/mod.rs:83` -> `qol_plugin_api::manifest::is_valid_safe_identifier`, `shortcuts/validation.rs:15` -> `validate_safe_identifier`. Null-byte check present |
| Health DTO hand-duplicated across HTTP boundary | Both sides re-export one source of truth: `daemon_health.rs:4` and `dev_server.rs:10` (tools/qol-cli) -> `qol_conventions::dev_health` |
| `image` type-only in qol-gpui, 5-line struct replaces it | `RenderImage::new(impl Into<SmallVec<[Frame;1]>>)` is gpui's public API; `color_wheel.rs:506-510` constructs it with `image::Frame::new`. Real dep |
| qol-conventions twice in 12 manifests; qol-gpui/qol-build-identity twice in tray | `[dependencies]` vs `[build-dependencies]` and cfg(target_os=...) sections - mandatory Cargo structure, not duplication. Current: 11 manifests carry it twice (alt-tab, bluetooth, controllers, ide-checkout, launcher, lights, monitor, pointz, qol-shot, qol-voice, window-actions); 31 reference it in total; tray has qol-gpui at `:130/:142` and qol-build-identity at `:47/:161` |
| `libs/qol-wasm` zero consumers, delete it | It WAS the cdylib source of the wasm bundle; `CommandPalette.js:5` imports it live. Now moot in the opposite direction: the crate was deleted in `03bcfd512` (2026-08-22) while the bundle stayed committed - the stale bundle is the real issue and it now has no source to rebuild from |
| Ghost-config field blocks byte-identical (launcher:15-28 vs alt-tab:124-137) | Blocks differ: `description` text ("hidden launcher window" vs "hidden picker window") and `section` value ("appearance" vs "layout"); current refs `launcher/qol-config.toml:15-24` vs `alt-tab/qol-config.toml:140-149`. Shared field shape, per-plugin values - likely legitimate |
| plugins/template "no tooling references" | 8 refs exist, all test-fixture ID strings (`apps/qol-tray/src/plugins/resolver.rs:408-410`, `doctor/checks/reserved_plugin_ids.rs:77-80`, `doctor/diagnosis/mod.rs:534`, `.github/scripts/tests/test_plugin_version.py:80-81,365`). Workspace `exclude` is still viable (fixture refs are strings, the crate itself is never built by those tests), but the claim as stated was wrong |

## LOCKED (deliberate, do not propose again)

| Item | Reason |
|---|---|
| git2 vendored-libgit2 + vendored-openssl in qol-tray | Comment: "vendored so the host needs no system git" - self-containment requirement |
| image/rav1e (avif) weight in gpui subtree | gpui-imposed; the ravif stub (`vendor/ravif`) already drops the rav1e encoder stack |

## Host state snapshot (not a repo finding)

| Item | Snapshot |
|---|---|
| target/ disk | 2026-08-25 host measurement: `du -sh target/debug target/release` = 24G debug, no target/release dir (investigation-time 113G/7.3G no longer holds - this tracks host usage, not the repo). Prune with `cargo sweep` if desired |

## Verified clean

Zero build warnings as of the 2026-07-31 investigation (only upstream `proc-macro-error2` future-incompat via gpui). 2026-08-25 re-check: `cargo check --workspace --all-targets` emitted no warnings in the crates it compiled. A pre-existing compile failure in `plugins/cli-sessions/tests/reconcile.rs` (missing `external_id_authoritative` in `CliSessionDescriptor` since 56f0a34ad added the required field on 2026-08-24 without updating the plugin test) was fixed on 2026-08-25 by adding the field to both fake descriptors; the workspace check now passes end-to-end. 0 TODO/FIXME/HACK markers in .rs (grep-verified). No dead workspace members (all 32 libs referenced from other manifests; plugins are standalone bins discovered via plugin.toml, by design). Both vendor patches live and justified (`Cargo.toml:83-89` `[patch.crates-io]` global-hotkey + ravif).

## Remaining actionable list (in impact order)

1. Resolve the orphaned wasm bundle: `qol_wasm_bg.wasm` (committed 2026-03-08) is imported live by `CommandPalette.js:5` but its cdylib source crate was deleted (03bcfd512) and no regen path exists. Recreate the glue crate + add a regen step (Makefile/CI) plus a staleness guard, or drop the bundle and fall back to JS fuzzy match
2. CI: drop standalone `cargo build --release` (:141), use `cargo test --release` (B3) - cuts the cold-cache 3x compile to 2x
3. Move zbus out of qol-platform into window-actions (B4); drops the async-io tree from qol-apps/qol-dev-env/launcher/qol-shot; alt-tab already has its own zbus dep, so no new edge
4. Delete broker listener half + restore/pane_field dead code (~1,175 lines), and delete or rework `libs/qol-runtime/tests/broker_socket_path_structural.rs` (18 refs, 6 tests) in the same change
5. Version bumps: png 0.17->0.18, tungstenite 0.26->0.30 (B6)
6. Merge single-consumer libs: frecency, app-icon (zero new edges); host-fixes is also zero-new-edges (all 6 consumers dep on qol-plugin-daemon) but has grown to 15.4k lines - reassess before merging; migrations, dev-orchestrator (bigger diffs, lower priority; migrations now adds a qol-profile-sync edge)
7. Fix stale doc paths (contract.md, layout.md line refs above), add rust-toolchain.toml
