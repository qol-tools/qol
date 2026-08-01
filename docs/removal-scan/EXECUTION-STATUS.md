# Removal scan - execution status

CONSOLIDATED-FINAL.md is the verbatim merged audit (originally under target/removal-scan/, lost to a cargo clean; restored from session context 2026-08-01).

Known corrections to its arithmetic:
- 131 CONFIRMED reconciles as 117 surviving + 13 downgraded + 1 refuted (root_has_exited); the "SURVIVES + DOWNGRADED" rule in the summary paragraph is wrong for libs-system.
- "117 deletable now" overstates: 14 of the 117 are cross-cutting-duplication consolidation findings, not removal candidates. Delete-ready count was 103.
- The slice-12 claim that removing the qol-cli dep line drops qol-platform from the qol build graph is wrong; qol-platform stays reachable via qol-apps and qol-dev-env (cargo tree verified).

Executed 2026-08-01, direct to main:
- c6120131 five dead dependency edges (execution order step 1)
- 6888542d tray-ui 48 items (runActions chain deferred, see prerequisite 5)
- 64926059 tray-rust-core 5 items (+ stranded clamp_opacity helper)
- 34cef807 libs slices 11 items (activation module, objc2 deps, build-identity 4, system 4, dev 2)
- a3ec44e8 plugins-devices 20 items + state_cache (LIKELY, promoted: became a dead_code warning after list_targets removal)
- b54fb8c8 plugins picker/sessions/input/misc 13 items

Verified: cargo check/clippy/fmt workspace-clean, qol-tray tests with dev feature 0 failed, per-crate agent tests green, ui node --test 581 pass (1 pre-existing failure in views/task-runner/test-runner-subpage.js, reproduces on clean HEAD).

## LIKELY tier, verified and partly executed 2026-08-01

Four independent read-only verifiers re-grepped all 67 LIKELY plus the 8 UNCERTAIN items. Zero refutations. Three corrections to the report:

- `InMemoryWorktreeLister`, `InMemoryBinaryProbe` (tray boot_contract) and `NoServiceProbe` (cli-sessions) are live test doubles injected into tests of production code, not dead code. The report's "test-only" evidence class conflates two different things: a production function whose only tests exist to cover itself, versus a fixture that exists so live code can be tested. Only the first is deletable.
- `GENERIC_TOOL_ID`, `claude_tool()`, `generic_tool()` (qol-terminal-sessions) are called inside their own crate to build the live `CliSessionStrategy` impls. Deleting them per the report's bullet would not compile; the correct action is a visibility trim.
- lights `BackendHealth`, `LightState`, `LightTargetInfo`, `BackendConnectionStatus` became fully orphaned once the CONFIRMED `health()`/`list_targets()` chain was deleted. Deleted as part of the plugins slice.

Executed (adfa6aa5, f200e90c, e5884e1c, f7882926): tray `is_prod`, five profile path helpers, three profile registry fns, the probe re-export, `PluginRow.js`, `_mockup-profile.html`, 8 css tokens; `walk_menu_items`, six `*_dark()` accessors, 13 dead css push lines, `DEV_ACCENT_KEY`/`QOL_DEV_ACCENT`, `BuildIdentityEnvironment::intent()`, `build_linked_plugins`; launcher and controllers re-export aliases, `Selection::anchored()`, the cli-sessions `"show"` alias, `TranscriberRequest::automatic()`, `move_to_hue_sat`, two `DeviceRegistry` lookups, the `DeviceLeft` variant and its match arm, the lights orphan cluster.

Deliberately kept, with reasons:

- `takeover::restore()` (qol-host-fixes) is the canonical host-restore helper behind the "host left as found" mission invariant. Bluetooth hand-rolls the same sequence; the fix is to make bluetooth use it, not to delete it.
- `require_feature` (qol-artifact) is the only populator of `required_features`. Deleting it would strand the verify loop that reads them and silently drop a working capability of the artifact identity contract.
- `RunHandle::wait` drives five tests that assert live orchestrator outcomes; `RunHandle::detach` has a named test pinning detach-versus-drop reap semantics.
- `build_linked_plugins_with_progress` and `_with_core_events` remain because a planning test uses them to assert fingerprint-skip behavior; only the unused top wrapper went.
- `clear_cancellation_request` is the cleanup call in a cancellation test; deleting it leaves temp files behind for a five-line win.
- The dev_console feature-flags subsystem is a live wired ctrl+f panel and the cited reference pattern for the worktrees panel. `service_commands` is config schema. `legacy_cask_json` is an external CLI output contract.

Latent failures found and fixed: `cargo test --workspace` was never run against the earlier CONFIRMED commits, which had left three qol-theme tests failing (two pinning deleted css tokens, one scanning a deleted ui file). Full workspace tests, clippy, and fmt are green as of f7882926.

Remaining: 13 DOWNGRADED (qol-skills prerequisite), 8 UNCERTAIN (human calls), runActions chain, 14 consolidation targets, and the kept-with-reasons items above. macOS edits uncompiled locally; CI matrix is the proof.
