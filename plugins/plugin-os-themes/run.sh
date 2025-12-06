#!/bin/bash
set -euo pipefail

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
