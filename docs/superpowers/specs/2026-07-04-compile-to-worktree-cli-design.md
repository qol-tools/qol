# Compile-to-worktree in qol dev (CLI)

## Context

The web UI already compiles qol-tray into a selected worktree:
picker -> `POST /dev/recompile-self {worktree_branch}` -> branch-to-path scan -> build in the worktree -> exec-restart into the worktree binary.

The CLI lags behind on both paths:

- `qol dev <branch>` builds the base tray at the monorepo root, boots it, then posts `recompile-self` so the tray rebuilds in the worktree and exec-restarts.
  That is a double build and a double boot.
- Armed reload (ctrl+r) prebuilds at the root and hardcodes `root/target/debug/qol-tray` as the successor binary, so it never compiles to a worktree.
- There is no way to select a worktree from the dev console.

Worktree resolution logic exists in four places today: `qol-cli/src/commands/dev.rs` (`parse_worktree_branches`), `qol-dev-build/src/planning/worktree.rs`, `qol-tray .../dev_services/worktrees.rs` (the scan the web UI uses), and `qol-tray/src/dev/boot_contract.rs` (`GitWorktreeLister`).

## Goals

- `qol dev <branch>` compiles the worktree tray directly and launches that binary: single build, single boot.
- Armed reload builds and hands off to the selected worktree's binary.
- A worktree picker panel in the dev console, arm-only semantics.
- An orange divergence accent when the selected worktree differs from the running build.
- One shared implementation of worktree listing, tray-root resolution, tray self-build, and marker IO, consumed by both qol-tray and qol-cli.

## Non-goals

- Web UI changes.
- `boot_contract` autostart/heal behavior changes (it keeps its own `WorktreeLister` trait; only the marker file IO is shared).
- Consolidating `planning/worktree.rs` (plugin dev-link remapping stays as is).
- Plugin build strategy changes; plugin worktree resolution keeps following the active-worktree marker through existing services.
- The keys legend not turning red while RELOADING (known cosmetic issue, out of scope).

## Design

### 1. Shared library: `libs/qol-dev-build`, new `tray` module

Moved out of qol-tray, with the tray delegating so behavior is unchanged:

- `list_worktrees(anchor: &Path) -> Vec<WorktreeInfo>`: the `worktrees/<branch-path>/<repo>/` ancestor scan from `dev_services/worktrees.rs`, including `resolve_git_branch`.
  `WorktreeInfo { branch, path }` moves into the lib.
  The tray endpoint passes its manifest dir; the CLI passes its repo root.
- `resolve_tray_root(selected: Option<&Path>, fallback: &Path) -> PathBuf`: `resolve_qol_tray_self_root` plus `manifest_is_qol_tray` from `dev/self_build.rs`, with the tray-manifest fallback passed in instead of `env!("CARGO_MANIFEST_DIR")`.
- `build_tray(root: &Path, bins: &[&str], on_progress) -> BuildResult`: the cargo invocation from `dev/self_build.rs` (`--features dev`, JSON message parsing, artifact-count progress), parameterized on bin names so the CLI can also build `qol-tray-doctor`.
- Marker IO: `active_worktree_marker` read/write/clear for `<config>/dev/active-worktree.txt`.
  Replaces the CLI's `active_worktree_marker_path`/`clear_active_worktree_marker` and backs `dev/linking`'s get/set in the tray.

`dev_services/worktrees.rs`, `dev_services/mod.rs` (`list_worktrees`, `list_branches`), and `dev/self_build.rs` become thin delegations.

### 2. CLI startup: `qol dev [worktree|--base]`

- `ensure_worktree_branch` and its `git worktree list` parsing are deleted.
  The branch is resolved against the shared `list_worktrees(repo_root)`; unknown branch still bails with the current message shape.
- The single persisted source of truth for the selection is the active-worktree marker (`<config>/dev/active-worktree.txt`), shared with the web UI and the tray boot contract.
  Argv is parsed into a three-way directive:
  - `qol dev <branch>`: pin - resolve strictly, write the marker.
  - `qol dev --base`: explicit base - clear the marker.
  - `qol dev`: follow - read the marker and boot whatever it names; no marker write.
    A marker branch whose worktree has vanished falls back to base with a printed note and keeps the marker, mirroring the tray's heal semantics.
- With a resolved branch: `build_tray(worktree_root, ["qol-tray", "qol-tray-doctor"], ...)`, launch `worktree_root/target/debug/qol-tray` with `current_dir = worktree_root`.
- `post_recompile(branch)` at boot is removed; `finish_boot` requests a plugin reload in both the branch and branchless case (the tray resolves plugin worktree paths from the marker).
- The base plugin batch build is skipped when a branch is set; the reload request covers plugin builds against worktree paths.

### 3. Armed reload

- The console passes the armed target to the prebuild command explicitly: `__dev-prebuild <branch>` for a worktree, `__dev-prebuild --base` for an explicit base selection, and no target arg when the panel was never used (`Follow`), which makes the prebuild follow the marker.
  `--base` exists so selecting base can override a persisted selection.
- The prebuild step applies the same three-way directive as startup: pin writes the marker, base clears it, follow reads it.
- `restart_child_from_prebuilt` resolves the successor from the marker the prebuild just persisted, so the launched binary can never diverge from what was built or from what the web UI recorded.
- The shadow/promote handoff protocol itself is untouched.

### 4. Worktree panel in the dev console

- New `dev_console/worktrees_panel.rs` modeled on `feature_flags.rs`, reusing the `picker.rs` brick primitives and `SignBox`.
- Opened with global key `ctrl+w` (toggles, like `ctrl+f`); listed in the keys HUD.
- Items: the base clone first, labeled with its actual git branch (e.g. `main`, falling back to `base` when unresolvable), then branches from `list_worktrees(repo_root)`, scanned when the panel opens.
- Arrows move selection, enter arms the highlighted target and closes the panel, esc closes without change.
- Selection state is an explicit enum, `WorktreeSelection::Follow | Pin(Option<String>)` (`Pin(None)` = base), so "nothing selected" and "base selected" are distinct states.
  `Follow` tracks the running branch and can never diverge; only an explicit `Pin` produces the orange state.
  The prebuild target for `Follow` falls back to the startup argv branch.
- Enter never triggers a build; the next armed ctrl+r consumes the target.
- Disarming (space toggle-off, esc, or any non-arm-preserving action while armed) cancels the pending selection, resetting the target to the running branch; only the armed ctrl+r reload itself keeps the target while consuming it.

### 5. Divergence accent

- The poller fetches `/dev/active-worktree` (existing endpoint) alongside health, storing `Dash.running_branch: Option<String>`.
- Divergent state: `worktree_target != running_branch`.
- The three states are exclusive in the UI; exactly one flag renders at a time.
  Precedence: red RELOADING > orange `WORKTREE <branch>` (divergent) > yellow ARMED > default.
- Breadcrumb shows the single active flag (` · WORKTREE <branch>`, or ` · WORKTREE base`).
- Orange is `Color::Rgb(255, 153, 0)` (ratatui has no named orange).
- A persistent branch sign straddles the bottom border, centered like the `qol dev` sign on top, showing the running branch and extending to `running → target` in orange while divergent.

## Error handling

- Unknown branch at startup: bail before any build, as today.
- Worktree build failure at startup: bail with the cargo output, same as a base build failure today.
- Worktree build failure during armed reload: existing "reload aborted: prebuild" path; the running tray stays up.
- Missing worktree binary after a successful build is treated as a build failure (guards against a bin-name mismatch).
- Panel scan finding no worktrees renders a dim "no worktrees" row, mirroring the empty feature-flags panel.

## Testing

- Shared `tray` module: existing scan and `resolve_qol_tray_self_root` tests move with the code; new table-driven tests for `build_tray` command construction and marker IO round-trip.
- CLI: tests for effective-branch resolution (panel selection overrides argv), successor binary path for base vs worktree, prebuild arg construction, accent precedence, and panel arm/close key handling.
- Tray: existing dev_services and self-recompile tests keep passing through the delegations, proving behavior parity.
