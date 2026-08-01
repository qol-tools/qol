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

Remaining: 67 LIKELY, 13 DOWNGRADED (qol-skills prerequisite), 8 UNCERTAIN (human calls), runActions chain, 14 consolidation targets. macOS edits uncompiled locally; CI matrix is the proof.
