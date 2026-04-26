#!/usr/bin/env bash
# Interactive (or auto, when stdin isn't a TTY) setup for qol-tray dev workflow.
# Writes .qol-tray-dev-hooks at the repo root — an executable shell script that
# runs before 'make dev' starts cargo. Add whatever commands you want there;
# qol-tray itself has no knowledge of any other repo or tool.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
hooks_file="$repo_root/.qol-tray-dev-hooks"

hooks=()

if [ -t 0 ]; then
  echo "qol-tray dev setup" >&2
  echo "------------------" >&2
  echo "Add pre-dev hook commands. Each runs in order before 'make dev'" >&2
  echo "starts cargo. Empty input finishes. You can edit $hooks_file" >&2
  echo "later or re-run 'make setup' to redo." >&2
  echo "" >&2

  while true; do
    printf 'Pre-dev hook command (empty to finish): ' >&2
    IFS= read -r cmd || break
    [ -z "$cmd" ] && break
    hooks+=("$cmd")
  done
fi

{
  echo "#!/usr/bin/env bash"
  echo "# qol-tray pre-dev hooks. Runs before 'make dev'. Edit freely."
  echo "# Gitignored — unique to your machine."
  echo ""
  echo "set -euo pipefail"
  echo ""
  if [ ${#hooks[@]} -eq 0 ]; then
    echo "# No hooks configured. Add one command per line, for example:"
    echo "#   /absolute/path/to/some-script --some-flag"
    echo "#   /absolute/path/to/another-step"
  else
    for cmd in "${hooks[@]}"; do
      echo "$cmd"
    done
  fi
} > "$hooks_file"

chmod +x "$hooks_file"

echo "" >&2
echo "wrote $hooks_file" >&2
if [ ${#hooks[@]} -eq 0 ]; then
  echo "(no hooks — 'make dev' will run cargo with no pre-steps; edit $hooks_file to add)" >&2
else
  echo "(${#hooks[@]} hook(s) configured)" >&2
fi
