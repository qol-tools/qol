## Problem

Contributors running qol-tray locally (`make dev` / `make install`) need to pick which gh account `qol-tools/*` shells use when `cd`-ing into the workspace. Today the only path is a terminal command. The qol-tray UI should expose this as a setting — the user types their handle, it's persisted to the existing `DevConfig` JSON, and the shell hook reads it.

## Constraints

- **Single binary, runtime-gated.** Same qol-tray binary regardless of install path; the field surfaces only when dev mode is active (existing `/api/dev/enabled` flag).
- **Free-form text input.** User types their handle (e.g. `KMRH47`). No dropdown, no `gh auth status --json` listing, no validation against keychain — keep it dumb. If the handle isn't in the keychain, the shell hook silently no-ops.
- **Reuse existing storage.** `DevConfig` (`src/dev/config.rs`) already exists with one field (`search_paths`) and is JSON-backed at `paths::dev_config_path()`. Add one new field there. Don't invent a parallel storage location.
- **Reuse existing UI surface.** Dev settings already render via `ui/views/dev/components/DevLayout.js`. Add an input section there.
- **Reuse existing dev-mode flag.** `/api/dev/enabled` already exists (`meta_handlers.rs:13,19-21`). Don't invent `/dev/status`.
- **Cross-platform path resolution.** qol-tray's Rust uses `dirs::config_dir()` via `paths::dev_config_path()` (already platform-aware). The shell hook uses `case "$OSTYPE"` to detect macOS Application Support vs Linux XDG vs Windows AppData.

## Proposed solution

### qol-tray (Rust + UI)

1. Add field to `DevConfig`:
   ```rust
   pub struct DevConfig {
       #[serde(default)] pub search_paths: Vec<PathBuf>,
       #[serde(default)] pub tooling_gh_account: Option<String>,  // NEW
   }
   ```
2. Save mutator: load → set field → atomic-write via existing pattern. Mirror however `search_paths` is mutated today.
3. UI: extend `ui/views/dev/components/DevLayout.js` with a labeled text input + Save + Clear. Wire to a tiny POST/GET pair on the existing dev API surface (or extend an existing handler — pick whichever is consistent with how `search_paths` is currently exposed; do not invent a new feature module).
4. Field is hidden / unrendered when dev mode is off.

### qol-cicd (shell hook)

`bin/activate.sh`'s chpwd hook needs to read the new field instead of the deprecated `~/.config/qol-tools/gh-account` file. Updated logic:

```sh
case "$OSTYPE" in
  darwin*) cfg="$HOME/Library/Application Support/qol-tray/dev-config.json" ;;
  *)       cfg="${XDG_CONFIG_HOME:-$HOME/.config}/qol-tray/dev-config.json" ;;
esac
[ -f "$cfg" ] || { unset GH_TOKEN; return; }
user=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("tooling_gh_account") or "")' "$cfg" 2>/dev/null)
[ -n "$user" ] || { unset GH_TOKEN; return; }
GH_TOKEN_NEW=$(command gh auth token --user "$user" 2>/dev/null)
[ -n "$GH_TOKEN_NEW" ] && export GH_TOKEN=$GH_TOKEN_NEW || unset GH_TOKEN
```

Cache by user in `_QOL_GH_TOKEN_CACHE` / `_QOL_GH_TOKEN_USER` (existing pattern), invalidate on user change.

### Cleanup (same PR, qol-cicd side)

- Delete `bin/qol-gh-account` (vestigial CLI from CICD-2).
- Update README to reflect the new flow (qol-tray UI is the source of truth).

## Dev flow (target state)

```
ONCE per machine
├─ make install (or binary install)            # qol-tray-install wires ~/.zshrc
└─ open qol-tray UI → enter handle → save      # writes DevConfig JSON

EVERY new terminal (automatic)
└─ cd qol-tools → GH_TOKEN exported. cd out → unset.
```

## Affected files

**qol-tray:**
- `src/dev/config.rs` — add field + setter
- `src/dev/state.rs` (or wherever DevConfig is exposed to HTTP) — extend POST/GET to handle new field
- `ui/views/dev/components/DevLayout.js` — input section
- `tests/` — unit test for serde round-trip; integration test for HTTP

**qol-cicd:**
- `bin/activate.sh` — replace path + reader logic
- `bin/qol-gh-account` — delete
- `README.md` — update flow

## Out of scope

- Adding accounts to gh keychain (use `gh auth login`).
- Per-directory account scoping (per-machine is sufficient).
- `qol-sdk` extraction (deferred to qol-mission roadmap).

## Verified facts

- ✅ `DevConfig` exists with one field today (`search_paths`) — `src/dev/config.rs:7-14`
- ✅ `paths::dev_config_path()` already platform-aware via `dirs::config_dir()` — `src/paths.rs`
- ✅ `/api/dev/enabled` exists for runtime dev-mode detection — `src/features/plugin_store/server/meta_handlers.rs:13,19-21`
- ✅ Dev settings UI renders at `ui/views/dev/components/DevLayout.js`
- ✅ qol-tray-install (TRAY-14) wires rc-file source line on first `make install`
- ✅ python3 ships with macOS and most Linux distros (no new dep for the shell hook)
- ✅ DevConfig is user-config (JSON, persisted, atomic-write capable)

## Open questions

(none — all prior [DECIDE]s collapsed by simplifications above)

## Research log

- [Ship-readiness 1](https://github.com/qol-tools/qol-tray/issues/17#issuecomment-4378865285) — flagged path mismatch (CICD-2's hardcoded `~/.config/qol-tools/`), wrong UI placement, reinventing `/api/dev/enabled`. Body simplified to one DevConfig field + extend existing UI + activate.sh reads JSON.

