## Problem

`qol-tray-install` already bootstraps per-machine state (autostart entry, install-id marker). After CICD-2 landed, qol-tools tooling expects a shell hook (`qol-cicd/bin/activate.sh`) to be sourced from the user's shell rc so `cd` into the qol-tools tree auto-exports `GH_TOKEN`. Today the user has to hand-edit `~/.zshrc` to source it. That's a manual step `qol-tray-install` should be doing.

## Proposed solution

Extend `qol-tray-install` to write a guarded hook block to the user's shell rc files. Idempotent, reversible, follows the fzf / direnv pattern.

### Block format (written to `~/.zshrc` AND `~/.bashrc` if they exist)

```sh
# >>> qol-tools shell hook >>>
[ -f "$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh" ] && \
  source "$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh"
# <<< qol-tools shell hook <<<
```

### Behavior

- **Install** (`qol-tray-install`): if the marker block isn't already present, append it. If it is, leave alone (idempotent). Touch only `~/.zshrc` and `~/.bashrc` that already exist; don't create new rc files.
- **Uninstall** (`qol-tray-install --uninstall`): strip any existing marker block.
- **Re-install / upgrade**: detect existing marker block, replace contents between markers if path/content changed; preserve user content above and below.
- **Path discovery**: the activate.sh path is hard-coded to `$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh` for now; the guard `[ -f ... ]` makes it a no-op if missing (so contributors who clone elsewhere don't get broken shells). Future: read from a per-machine config if needed — out of scope.

### What does NOT change

- The user's interactive shell already running before re-install. They open a new terminal to pick up the change. Documented in installer output.
- `qol-cicd/bin/activate.sh` itself — already shipped via CICD-2.
- `~/.config/qol-tools/gh-account` — already managed by `qol-cicd/bin/qol-gh-account`.

## Affected files

- `qol-tray/src/installer/` — add a `shell_hook.rs` module (new) that owns the rc-file-mutating logic.
- `qol-tray/src/installer/main.rs` — wire `shell_hook::install()` into the install flow and `shell_hook::uninstall()` into the uninstall flow.
- `qol-tray/src/doctor/checks/` — new check `shell_hook_present` that warns if the marker block is missing on a machine where qol-tray is active. `Diagnosis::Fix(InstallShellHook)` so `qol-tray-doctor --fix` can repair without a full re-install.

## Scope

- Pure Rust, all in qol-tray. No qol-cicd changes (CICD-2 already shipped the activate.sh).
- Tests for: idempotent append, marker-block replacement, uninstall removes only the block, missing rc file is skipped, doctor check + auto-fix.
- Atomic write via existing `crate::file_io::atomic_write`.

