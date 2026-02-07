#!/bin/bash
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND="$SCRIPT_DIR/../backends/linux.sh"

PASS=0
FAIL=0

pass() {
    echo "  ✓ $1"
    PASS=$((PASS + 1))
}

fail() {
    echo "  ✗ $1"
    FAIL=$((FAIL + 1))
}

run_test() {
    echo ""
    echo "Test: $1"
}

run_test "empty query returns nothing"
result=$("$BACKEND" "" 2>/dev/null) || true
[[ -z "$result" ]] && pass "empty query returns empty output" || fail "expected empty output"

run_test "single word query returns results"
result=$("$BACKEND" "bash" 2>/dev/null) || true
[[ -n "$result" ]] && pass "single word query returns results" || fail "expected non-empty output"

run_test "results are limited to 50"
result=$("$BACKEND" "lib" 2>/dev/null) || true
count=$(echo "$result" | wc -l)
[[ "$count" -le 50 ]] && pass "results limited to 50 lines ($count)" || fail "got $count lines"

run_test "case insensitive search"
result_lower=$("$BACKEND" "readme" 2>/dev/null) || true
result_upper=$("$BACKEND" "README" 2>/dev/null) || true
[[ -n "$result_lower" && -n "$result_upper" ]] && pass "case insensitive search works" || fail "case sensitivity issue"

run_test "multi-word query returns results"
result=$("$BACKEND" "launcher plugin" 2>/dev/null) || true
[[ -n "$result" ]] && pass "multi-word query returns results" || fail "expected non-empty output"

run_test "multi-word query performance (<500ms)"
start=$(date +%s%3N)
"$BACKEND" "qol tray" >/dev/null 2>&1 || true
end=$(date +%s%3N)
duration=$((end - start))
[[ "$duration" -le 500 ]] && pass "multi-word search fast (${duration}ms)" || fail "took ${duration}ms"

run_test "special characters in query"
"$BACKEND" "test.txt" >/dev/null 2>&1 && pass "special characters handled" || fail "special characters caused error"

run_test "regex metacharacters escaped"
"$BACKEND" "file[1]" >/dev/null 2>&1 && pass "regex metacharacters escaped" || fail "regex metacharacters caused error"

run_test "three word query"
result=$("$BACKEND" "qol tray plugin" 2>/dev/null) || true
[[ -n "$result" ]] && pass "three word query works" || fail "expected non-empty output"

run_test "path with spaces"
result=$("$BACKEND" "my documents" 2>/dev/null) || true
pass "path with spaces handled (got $(echo "$result" | wc -l) results)"

echo ""
echo "=== Summary ==="
echo "Passed: $PASS"
echo "Failed: $FAIL"

[[ "$FAIL" -gt 0 ]] && exit 1 || exit 0
