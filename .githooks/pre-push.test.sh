#!/usr/bin/env bash
set -u

hook="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/pre-push"
zero=0000000000000000000000000000000000000000

passed=0
failed=0
ok()   { passed=$((passed + 1)); printf '  ok   %s\n' "$1"; }
fail() { failed=$((failed + 1)); printf '  FAIL %s -- %s\n' "$1" "$2"; }

push_status() {
  local remote_ref=$1 remote_sha=$2 local_sha=$3
  shift 3
  local stderr status
  stderr="$(printf 'refs/heads/main %s %s %s\n' "$local_sha" "$remote_ref" "$remote_sha" \
    | env "$@" python3 "$hook" 2>&1 >/dev/null)"
  status=$?
  case "$status:$stderr" in
    0:*) echo passed ;;
    1:*"pre-push rejected"*) echo rejected ;;
    *) echo "error $status" ;;
  esac
}

expect() {
  local name=$1 expected=$2 actual=$3
  if [ "$expected" = "$actual" ]; then ok "$name"; else fail "$name" "expected $expected got $actual"; fi
}

expect "a release profile change that re-releases every unit is rejected" rejected \
  "$(push_status refs/heads/main a0294d3dd 9625d7046)"
expect "a shared lib constant that re-releases every unit is rejected" rejected \
  "$(push_status refs/heads/main a0294d3dd ae2a2ed28)"
expect "the override lets a wide release through" passed \
  "$(push_status refs/heads/main a0294d3dd 9625d7046 QOL_ALLOW_WIDE_RELEASE=1)"
expect "a gpui fix that releases six units passes" passed \
  "$(push_status refs/heads/main ae2a2ed28 15cf139af)"
expect "a single plugin fix passes" passed \
  "$(push_status refs/heads/main c799e17ec d559eda15)"
expect "a push to another branch is never checked" passed \
  "$(push_status refs/heads/feature a0294d3dd 9625d7046)"
expect "a new remote branch is never checked" passed \
  "$(push_status refs/heads/main "$zero" 9625d7046)"

printf '\nSummary: %d passed, %d failed\n' "$passed" "$failed"
[ "$failed" -eq 0 ]
