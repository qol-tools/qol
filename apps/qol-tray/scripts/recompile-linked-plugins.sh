#!/usr/bin/env bash
set -uo pipefail

# Recompile every dev-linked plugin via `cargo build` so daemons launched
# by qol-tray pick up local source changes. Reads the source-of-truth
# registry at $XDG_CONFIG_HOME/qol-tray/plugin-registry.json.
#
# Behaviour:
# - Missing registry: print one notice, exit 0 (fresh install / CI).
# - Missing plugin path or Cargo.toml: skip with a one-line warning, continue.
# - cargo build failure: report once at the end, but exit 0 — the user still
#   needs make dev to launch qol-tray's GUI to recover via the Recompile pane.
# - cargo's own incremental build is the no-op fast path.

REGISTRY="${XDG_CONFIG_HOME:-$HOME/.config}/qol-tray/plugin-registry.json"

if [ ! -r "$REGISTRY" ]; then
    printf 'recompile-linked: no registry at %s (skipping)\n' "$REGISTRY"
    exit 0
fi

if ! command -v python3 >/dev/null 2>&1; then
    printf 'recompile-linked: python3 not on PATH; cannot parse registry (skipping)\n' >&2
    exit 0
fi

paths=$(python3 - "$REGISTRY" <<'PY'
import json, sys
with open(sys.argv[1]) as f:
    reg = json.load(f)
for entry in reg.get("entries", []):
    active = entry.get("active") or {}
    src = active.get("source") or {}
    if src.get("type") != "dev-link":
        continue
    path = active.get("path") or src.get("origin_path")
    if path:
        print(f"{entry['id']}\t{path}")
PY
) || { printf 'recompile-linked: failed to parse %s (skipping)\n' "$REGISTRY" >&2; exit 0; }

if [ -z "$paths" ]; then
    printf 'recompile-linked: no dev-linked plugins\n'
    exit 0
fi

failed=()
while IFS=$'\t' read -r id pluginpath; do
    [ -n "$id" ] || continue
    if [ ! -d "$pluginpath" ]; then
        printf 'recompile-linked: skip %s — path missing: %s\n' "$id" "$pluginpath" >&2
        continue
    fi
    if [ ! -f "$pluginpath/Cargo.toml" ]; then
        printf 'recompile-linked: skip %s — no Cargo.toml in %s\n' "$id" "$pluginpath" >&2
        continue
    fi
    printf '==> recompile %s\n' "$id"
    if ! ( cd "$pluginpath" && cargo build ); then
        failed+=("$id")
    fi
done <<< "$paths"

if [ "${#failed[@]}" -gt 0 ]; then
    printf '\nrecompile-linked: %d plugin(s) failed to build: %s\n' \
        "${#failed[@]}" "${failed[*]}" >&2
    printf 'recompile-linked: continuing — recover via qol-tray GUI Recompile pane.\n' >&2
fi

exit 0
