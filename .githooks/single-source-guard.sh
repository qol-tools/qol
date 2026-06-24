#!/usr/bin/env bash
# Single-source-of-truth guard for cross-process constants.
#
# The dev-server port, the platform state-socket path, and the env-var names the
# host injects into plugins live in ONE place: libs/qol-conventions. This guard
# blocks the raw literals from reappearing in any other crate, so the value a
# plugin/CLI/tray process uses can never drift from the value qol-conventions
# defines. Run by the pre-commit hook and by the CI "Single source guard" step.
set -u

root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[ -n "$root" ] && cd "$root" || exit 0

pattern='42700|qol-tray-state\.sock|QOL_TRAY_STATE_SOCKET|QOL_TRAY_PLUGIN_ID'

hits="$(git grep -nE "$pattern" -- '*.rs' ':!libs/qol-conventions/' 2>/dev/null || true)"

if [ -n "$hits" ]; then
  {
    printf '\n  single-source guard rejected: cross-process constants must come from qol-conventions\n'
    printf '  offending occurrences:\n'
    printf '%s\n' "$hits" | sed 's/^/    /'
    printf '\n  fix: use qol_conventions::{DEFAULT_PORT, STATE_SOCKET_PATH, ENV_STATE_SOCKET, ENV_PLUGIN_ID, settings_url}\n'
    printf '       (and derive a plugin id from plugin.toml via qol_conventions::build::emit_plugin_id)\n\n'
  } >&2
  exit 1
fi

exit 0
