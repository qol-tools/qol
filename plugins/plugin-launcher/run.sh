#!/usr/bin/env bash
set -euo pipefail

SOCKET_PATH="/tmp/qol-launcher.sock"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHER="$SCRIPT_DIR/launcher"

if [[ ! -x "$LAUNCHER" ]]; then
    if command -v launcher >/dev/null 2>&1; then
        LAUNCHER="launcher"
    else
        echo "launcher binary not found" >&2
        exit 1
    fi
fi

send_show_socket() {
    if [[ ! -S "$SOCKET_PATH" ]]; then
        return 1
    fi

    if command -v socat >/dev/null 2>&1; then
        printf 'show' | socat - UNIX-CONNECT:"$SOCKET_PATH" >/dev/null 2>&1
        return $?
    fi

    if command -v nc >/dev/null 2>&1; then
        printf 'show' | nc -U "$SOCKET_PATH" >/dev/null 2>&1
        return $?
    fi

    if command -v python3 >/dev/null 2>&1; then
        python3 - "$SOCKET_PATH" <<'PY' >/dev/null 2>&1
import socket
import sys

path = sys.argv[1]
try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    s.sendall(b"show")
    s.close()
except Exception:
    raise SystemExit(1)
PY
        return $?
    fi

    return 1
}

if [[ "${1:-}" == "open" ]]; then
    if send_show_socket; then
        exit 0
    fi

    setsid "$LAUNCHER" --show >/dev/null 2>&1 &
    exit 0
fi

exec "$LAUNCHER" "$@"
