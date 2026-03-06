# Architecture Branch Handoff

Status:
- Repo: `qol-tray`
- Branch: `architecture`
- Remote: `origin/architecture`
- Baseline commit for this refactor phase: `c6e87d4`
- Handoff doc commit base: `f41f267`

Purpose:
- This document is the current handoff for the large architecture refactor on the `architecture` branch.
- It is intended to let another engineer or agent continue work without replaying the full chat history.
- It supersedes the old parallel-plan context and is more current than the file-separation inventory for the late UI work.

## What This Branch Was Doing

The branch continued a large separation-of-concerns refactor across `src/` and `ui/` with these rules:
- thin facades
- explicit domain owners
- platform-specific behavior only in domain-owned `platform/` modules
- no shallow buckets when a file does not earn its own seam
- stable route shapes, DTOs, and runtime behavior
- UI split by page coordinator vs data/effects/rendering helpers

This branch was intentionally checkpointed with many `wip:` commits so the user could later squash or reshuffle them.

## Current Source Of Truth

Use these docs in this order:

1. `docs/architecture/ARCHITECTURE_BRANCH_HANDOFF.md`
- Most current branch-level handoff.
- Covers work completed after the original parallel plan.

2. `docs/architecture/file-separation-of-concerns-inventory.md`
- Good for overall file-by-file SoC inventory.
- Partially stale for the latest UI splits on this branch.
- In particular, the late dev-page UI and style state described there is older than the current branch state.

3. `workspace: docs/architecture/qol-tray-parallel-refactor-plan.md`
- Historical orchestration doc for the multi-agent phase.
- Useful for process history, not for current branch truth.

## Branch Summary

High-level outcome from `c6e87d4..f41f267`:
- 142 files changed
- 11,956 insertions
- 8,644 deletions
- most large mixed files were turned into facades plus focused sub-owners
- the refactor touched both backend architecture and web UI architecture

The user verified runtime manually during the branch work. No automated build/test commands were run by the agent because the workspace rules forbid that.

## Completed Refactor Waves

### 1. Runtime, daemon, and desktop-state boundaries

Completed:
- `src/runtime/server.rs` was split into socket, poll, and shared-state owners
- `src/runtime/server/socket.rs`
- `src/runtime/server/socket/io.rs`
- `src/runtime/server/socket/requests.rs`
- `src/runtime/server/poll.rs`
- `src/runtime/server/poll/events.rs`
- `src/runtime/server/poll/sample.rs`
- `src/runtime/server/shared.rs`
- `src/runtime/server/shared/snapshot.rs`
- `src/runtime/server/shared/subscribers.rs`

Completed:
- `src/desktop_state/mod.rs` was reduced to a facade
- ignore-pid logic was split out into `src/desktop_state/ignore_pids.rs`
- platform ownership remains in:
  - `src/desktop_state/platform/mod.rs`
  - `src/desktop_state/platform/linux.rs`
  - `src/desktop_state/platform/macos.rs`

Completed:
- `src/daemon/init.rs` was deleted
- daemon construction/lifecycle concerns were consolidated into `src/daemon/mod.rs`
- daemon event ownership stayed in `src/daemon/events.rs`

Important invariant:
- runtime remains the single authority for monitor, cursor, and focus state
- plugins must not own monitor/focus polling

### 2. Dev module backend split

Completed:
- `src/dev/build.rs` is now a thin composition facade
- planning split into:
  - `src/dev/build/planning.rs`
  - `src/dev/build/planning/queue.rs`
  - `src/dev/build/planning/rebuild_reason.rs`
  - `src/dev/build/planning/selection.rs`
- cargo build split into:
  - `src/dev/build/cargo_build.rs`
  - `src/dev/build/cargo_build/spawn.rs`
  - `src/dev/build/cargo_build/plugin_build.rs`
  - `src/dev/build/cargo_build/plugin_build/progress.rs`
  - `src/dev/build/cargo_build/plugin_build/streams.rs`
  - `src/dev/build/cargo_build/self_build.rs`
  - `src/dev/build/cargo_build/self_build/artifacts.rs`
  - `src/dev/build/cargo_build/codesign.rs`
- build service split into:
  - `src/dev/build/service.rs`
  - `src/dev/build/service/events.rs`
  - `src/dev/build/service/persistence.rs`
  - `src/dev/build/service/runner.rs`
- config split into:
  - `src/dev/config.rs`
  - `src/dev/config/loading.rs`
  - `src/dev/config/search_paths.rs`
- discovery split into:
  - `src/dev/discovery.rs`
  - `src/dev/discovery/manifest.rs`
  - `src/dev/discovery/output.rs`
  - `src/dev/discovery/search.rs`
  - `src/dev/discovery/source.rs`
- linking split into:
  - `src/dev/linking.rs`
  - `src/dev/linking/listing.rs`
  - `src/dev/linking/store.rs`

Completed earlier in the same broader refactor:
- dev core consolidated around:
  - `src/dev/core/mod.rs`
  - `src/dev/core/model.rs`
  - `src/dev/core/reducer.rs`
  - `src/dev/core/progress_estimator.rs`
  - `src/dev/core/progress_parser.rs`

### 3. Doctor checks

Completed:
- `src/doctor/checks.rs` is no longer the whole check system
- extracted:
  - `src/doctor/checks/autostart_target.rs`
  - `src/doctor/checks/install_identity.rs`
  - `src/doctor/checks/runtime_prereqs.rs`

### 4. Hotkeys

Completed backend split:
- `src/hotkeys/mod.rs` became a facade
- extracted:
  - `src/hotkeys/manager.rs`
  - `src/hotkeys/planning.rs`
  - `src/hotkeys/catalog.rs`
  - `src/hotkeys/listener.rs`

Completed UI split:
- `ui/views/hotkeys-view.js`
- `ui/views/hotkeys/data.js`
- `ui/views/hotkeys/modal.js`

### 5. Plugin manifest validation

Completed:
- `src/plugins/manifest/validation.rs` was split into rule-focused modules
- extracted:
  - `src/plugins/manifest/validation/command_rules.rs`
  - `src/plugins/manifest/validation/dependency_rules.rs`
  - `src/plugins/manifest/validation/manifest_rules.rs`
  - `src/plugins/manifest/validation/menu_rules.rs`
  - `src/plugins/manifest/validation/runtime_rules.rs`

### 6. Plugin-store dev server and settings/services

Completed:
- old monoliths deleted:
  - `src/features/plugin_store/server/dev_runtime.rs`
  - `src/features/plugin_store/server/dev_services.rs`
  - `src/features/plugin_store/server/plugin_services.rs`
  - `src/features/plugin_store/server/settings/media_cover_handlers.rs`
  - `src/features/plugin_store/server/settings/plugin_config_handlers.rs`

Replaced with:
- `src/features/plugin_store/server/dev_runtime/mod.rs`
- `src/features/plugin_store/server/dev_runtime/core_events.rs`
- `src/features/plugin_store/server/dev_runtime/mock.rs`
- `src/features/plugin_store/server/dev_runtime/snapshot.rs`
- `src/features/plugin_store/server/dev_services/mod.rs`
- `src/features/plugin_store/server/dev_services/reload.rs`
- `src/features/plugin_store/server/dev_services/recompile.rs`
- `src/features/plugin_store/server/dev_services/mock.rs`
- `src/features/plugin_store/server/plugin_services/mod.rs`
- `src/features/plugin_store/server/plugin_services/catalog.rs`
- `src/features/plugin_store/server/plugin_services/installed.rs`
- `src/features/plugin_store/server/plugin_services/operations.rs`
- `src/features/plugin_store/server/settings/media_cover_handlers/mod.rs`
- `src/features/plugin_store/server/settings/media_cover_handlers/cover_file.rs`
- `src/features/plugin_store/server/settings/plugin_config_handlers/mod.rs`
- `src/features/plugin_store/server/settings/plugin_config_handlers/io.rs`
- `src/features/plugin_store/server/settings/plugin_config_handlers/notify.rs`

Per-plugin CPU monitoring split completed:
- `src/features/plugin_store/server/dev_plugin_cpu/mod.rs`
- `src/features/plugin_store/server/dev_plugin_cpu/sampling.rs`
- `src/features/plugin_store/server/dev_plugin_cpu/snapshot.rs`
- `src/features/plugin_store/server/dev_plugin_cpu/state.rs`

### 7. UI style split

Completed:
- old style buckets were split into concept owners
- new style files include:
  - `ui/styles/theme-tokens.css`
  - `ui/styles/app-shell.css`
  - `ui/styles/common-controls.css`
  - `ui/styles/common-dialogs.css`
  - `ui/styles/common-plugin-cards.css`
  - `ui/styles/common-settings.css`
  - `ui/styles/dev-layout.css`
  - `ui/styles/dev-plugin-list.css`
  - `ui/styles/plugin-grid.css`
  - `ui/styles/auto-config-page.css`

Current note:
- `ui/styles/common-components.css`, `ui/styles/dev-page.css`, and `ui/styles/styles.css` are now thin import hubs, not the old giant mixed owners

### 8. App shell and page-level UI splits

Completed:
- app shell split:
  - `ui/components/App.js`
  - `ui/components/app/dev-flows.js`
  - `ui/components/app/views.js`

Completed:
- plugins page split:
  - `ui/views/plugins-view.js`
  - `ui/views/plugins/data.js`
  - `ui/views/plugins/grid.js`
  - `ui/views/plugins/confirm-modal.js`

Completed:
- store page split:
  - `ui/views/store-view.js`
  - `ui/views/store/data.js`
  - `ui/views/store/grid.js`
  - `ui/views/store/token-panel.js`

Completed:
- task runner page split:
  - `ui/views/task-runner-view.js`
  - `ui/views/task-runner/data.js`
  - `ui/views/task-runner/panels.js`

### 9. Dev page UI split

This was the largest UI cleanup on this branch.

Completed:
- `ui/views/dev/index.js` became a coordinator instead of a behavior bucket
- extracted:
  - `ui/views/dev/action-router.js`
  - `ui/views/dev/key-router.js`
  - `ui/views/dev/view-dom.js`
  - `ui/views/dev/cpu-controller.js`
  - `ui/views/dev/plugin-actions-controller.js`
  - `ui/views/dev/plugin-actions/linking.js`
  - `ui/views/dev/plugin-actions/log-controls.js`
  - `ui/views/dev/plugin-actions/reload.js`
  - `ui/views/dev/plugin-row-template.js`
  - `ui/views/dev/plugin-row/cpu.js`
  - `ui/views/dev/plugin-row/menu.js`
  - `ui/views/dev/plugin-row/status.js`
  - `ui/views/dev/build-overlay.js`
  - `ui/views/dev/build-overlay/dom.js`
  - `ui/views/dev/build-overlay/fill.js`
  - `ui/views/dev/build-overlay/sync.js`
  - `ui/views/dev/build-overlay/completion.js`
  - `ui/views/dev/build-overlay/completion/phases.js`
  - `ui/views/dev/build-overlay/completion/store.js`

Important practical result:
- the dev page render/update code is now split by coordinator, routing, row rendering, CPU state, action flows, and build overlay animation concerns

### 10. Auto-config page and style cleanup

Completed partially:
- `ui/auto-config.html` was reduced significantly
- `ui/styles/auto-config-page.css` now owns page styling

Not complete:
- `ui/auto-config.html` is still one of the largest remaining mixed UI files

## Important Integration Fixes That Already Happened On This Branch

These were not just structural changes. Some real regressions were fixed during integration:
- refactor merge regressions were repaired in `wip: fix merged refactor integration regressions`
- stale root plugin binaries no longer shadow dev build outputs for dev-linked plugins
- workspace IntelliJ run configurations for `qol-tray` were corrected earlier in the session history
- various visibility/export issues from Rust module extraction were fixed after user build verification
- several test-only and warning-only import regressions were fixed during the split
- user-verified `make dev` was clean after the integration repair pass

## What Is Still Not Fully Done

These are the highest-value remaining hotspots on the branch.

Backend:
- `src/runtime/server.rs`
  - still large even after the socket/poll/shared split
  - likely still worth one more pass to reduce orchestration density
- `src/dev/build/planning.rs`
  - still one of the densest backend seams
- `src/dev/build/service/runner.rs`
  - still carries a large execution pipeline
- `src/dev/build/cargo_build/plugin_build.rs`
- `src/dev/build/cargo_build/self_build.rs`
- `src/dev/discovery.rs`
- `src/doctor/checks.rs`
- `src/features/plugin_store/server/dev_plugin_cpu/sampling.rs`
- `src/features/plugin_store/server/dev_runtime/mock.rs`
- `src/features/plugin_store/server/dev_services/recompile.rs`
- `src/features/plugin_store/server/plugin_services/operations.rs`
- `src/hotkeys/mod.rs`
- `src/plugins/manifest/validation.rs`

UI:
- `ui/auto-config.html`
  - still the biggest mixed UI file
- `ui/components/App.js`
  - much better than before, but still one of the larger coordinators
- `ui/views/dev/plugin-actions/linking.js`
  - focused now, but still relatively dense
- `ui/views/dev/build-overlay/fill.js`
  - isolated correctly, but still fairly dense animation logic

## What Is Probably Good Enough For Now

These were previous hotspots but are now in acceptable shape and should not be split again unless behavior changes demand it:
- `ui/views/store-view.js`
- `ui/views/plugins-view.js`
- `ui/views/task-runner-view.js`
- `ui/views/hotkeys-view.js`
- `ui/views/dev/index.js`
- `ui/views/dev/build-overlay/completion.js`
- `src/features/plugin_store/server.rs`
- `src/features/plugin_store/github/mod.rs`
- `src/plugins/action_executor.rs`
- `src/plugins/manager/mod.rs`

## Agent Pickup Guidance

If more agents continue from this branch, keep the original architecture rules:
- no `cargo build`, `cargo test`, `cargo check`, or `make`
- thin facades only
- platform code stays in domain-owned `platform/` modules
- preserve routes, payload shapes, and behavior
- keep `main.rs`, `lib.rs`, and server DTO surfaces stable unless there is a compelling integration reason

Recommended next work ordering:
1. `src/dev/build/planning.rs`
2. `src/dev/build/service/runner.rs`
3. `src/dev/build/cargo_build/plugin_build.rs`
4. `src/dev/build/cargo_build/self_build.rs`
5. `src/runtime/server.rs`
6. `ui/auto-config.html`
7. `ui/components/App.js`

Reasoning:
- the remaining highest-value debt is now mostly in backend planning/execution and the auto-config page
- many UI page coordinators have already crossed the line from mixed to acceptable

## Branch Sync Commands

To continue on another machine:

```bash
cd /path/to/qol-tray
git fetch origin
git checkout architecture
git pull --ff-only origin architecture
```

If the branch does not exist locally yet:

```bash
cd /path/to/qol-tray
git fetch origin
git checkout -b architecture origin/architecture
```

## Checkpoint Commit History For This Phase

From `c6e87d4..f41f267`:

- `6b4d711` `wip: split runtime server boundaries`
- `738aeac` `wip: split dev module owners`
- `d9e1f04` `wip: split dev build planning and execution`
- `f0c9fac` `wip: split manifest validation rules`
- `25b49f5` `wip: split hotkey listener runtime`
- `5f30035` `wip: split doctor checks owners`
- `78d82fa` `wip: split ui style boundaries`
- `9fcdb9a` `wip: fix merged refactor integration regressions`
- `41d452c` `wip: split dev page cpu actions and row rendering`
- `6a1ca07` `wip: split dev build overlay and sync architecture inventory`
- `948dfe8` `wip: split hotkeys and runtime server seams`
- `b194f8e` `wip: split hotkeys view state and modal helpers`
- `a7980f3` `wip: split task runner view data and panels`
- `a768649` `wip: split app shell dev flows and mounted views`
- `0f39f05` `wip: split plugins view data and grid helpers`
- `706a4ab` `wip: split store view data token and grid helpers`
- `ee5a560` `wip: split dev page routing and dom state`
- `eac3f91` `wip: split dev plugin actions by concern`
- `61fa20c` `wip: split dev completion playback state and phases`
- `8e3c60d` `wip: split dev overlay fill and row sync`
- `f41f267` `wip: split dev plugin row cpu menu and status`

## Final Notes

- The branch is structurally much closer to the `plugin-alt-tab` architecture bar than the baseline was.
- The biggest remaining risk is not hidden coupling anymore. It is a smaller number of still-dense files.
- If this branch is continued, prefer another sequence of small `wip:` checkpoints rather than one large mixed pass.
