
## 2026-06-04
- In qol-tray runtime, focus stamp can outrace cursor stamp because keyboard-only Alt+Tab leaves `cursor_moved=false` (CursorChannel needs >20px), so `update_cursor` short-circuits and `pick_active_monitor` lets focus win.
- `is_own_window` filter in `desktop_state/platform/linux.rs` only matches qol-tray's own pid; plugin daemons (e.g. plugin-alt-tab picker) are separate processes, so their windows DO show up as `focused_window_bounds()`.
- `refresh_focus_synchronously` only restamps `input.focus.at` when the focus monitor actually changes, not on every GET_STATE — so a stale focus monitor keeps its old `Instant` until OS focus moves.

## 2026-06-04
- In qol-tray dev Recompile, `spawn_build` runs bare `cargo build` at the stored dev-link path; if `create_link` stores workspace root, every plugin's build pulls the whole workspace.
- `find_git_worktree_base` in `dev/linking/store.rs::create_link` resolves monorepo plugin sources to the workspace root, corrupting plugin-registry.json with identical paths.
- `check_plugin_platform` used to fail-open on missing/unparseable plugin.toml; a corrupt dev-link entry then delegated to a workspace-wide cargo build instead of being Skipped.

## 2026-06-04
- When adding a new `FixAction` variant gated behind `#[cfg(feature = "dev")]`, also gate the matching arm in `doctor::mod::log_applied` — exhaustiveness check fires under `--features dev`.
- `make test` in `apps/qol-tray` runs without `--features dev`, so dev-gated tests are silently filtered; verify them separately via `cargo test -p qol-tray --features dev`.
- `cargo test -p qol-tray <filter>` (without `--lib`) runs integration bins first and reports "0 passed" with the lib filtered out; use `--lib` to actually see lib-tests results.

## 2026-06-05
- In qol-tray registry, `WorktreeLink` variant exists but is never created in production code; only dev code paths produce `DevLink`, with worktree overrides mutating `entry.active.path` while leaving variant as `DevLink`.
- `auto_fix_startup` runs pre-tokio before plugin load, so registry-mutating fixes (like `RelocateDevLink`) take effect on first plugin resolution with no daemon restart needed.

## 2026-06-07
- `make build` in apps/qol-tray invokes `lint` (clippy `--all-targets -D warnings`) - test-only code (e.g. inside `#[test]` fns) blocks the build, not just CI.
- HashMap-backed JSON saves (e.g. build-fingerprints.json) re-emit keys in nondeterministic order; verify "no content change" with `jq -S` semantic diff, not raw `diff`.
- CI clippy args come from `.github/scripts/affected_crates.py` (`--workspace --exclude keyremap --all-targets` on Ubuntu); local repro needs `--all-targets`, plain `cargo clippy -p X` misses it.
