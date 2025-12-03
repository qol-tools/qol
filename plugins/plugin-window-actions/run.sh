#!/usr/bin/env bash
set -euo pipefail

ACTION="${1:-}"
SCRIPT_DIR="$(dirname "$(readlink -f "$0")")/scripts"

case "$ACTION" in
    snap-left|snap-right|snap-bottom|maximize|minimize|restore|center|move-monitor-left|move-monitor-right)
        exec "$SCRIPT_DIR/$ACTION.sh"
        ;;
    *)
        echo "Unknown action: $ACTION" >&2
        exit 1
        ;;
esac

