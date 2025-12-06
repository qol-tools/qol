#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ACTION="${1:-run}"

case "$ACTION" in
    run)
        echo "Hello from My Plugin"
        ;;
    *)
        echo "Unknown action: $ACTION" >&2
        exit 1
        ;;
esac
