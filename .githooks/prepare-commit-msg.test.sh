#!/usr/bin/env bash
set -u

hooks="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
hook="$hooks/prepare-commit-msg"
owner='Co-authored-by: KMRH47 <45654430+KMRH47@users.noreply.github.com>'
agent='357478593+kmrh47-agents@users.noreply.github.com'
agent_plus='kmrh47+agents@proton.me'
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/hooks"
cp "$hook" "$scratch/hooks/prepare-commit-msg"

passed=0
failed=0
ok()   { passed=$((passed + 1)); printf '  ok   %s\n' "$1"; }
fail() { failed=$((failed + 1)); printf '  FAIL %s -- %s\n' "$1" "$2"; }

eq() {
  if [ "$2" = "$3" ]; then ok "$1"; else fail "$1" "expected [$2] got [$3]"; fi
}

run_hook() {
  local email=$1 message=$2
  printf '%s\n' "$message" > "$scratch/msg"
  ( cd "$scratch" && GIT_AUTHOR_NAME=someone GIT_AUTHOR_EMAIL="$email" "$hook" "$scratch/msg" message )
  cat "$scratch/msg"
}

owner_lines() {
  printf '%s\n' "$1" | grep -cF "$owner"
}

test_agent_commits_get_the_owner_line() {
  local out email
  for email in "$agent" "$agent_plus" "KMRH47+Agents@Proton.me"; do
    out="$(run_hook "$email" "fix(tray): x")"
    eq "subject only, $email" "fix(tray): x"$'\n\n'"$owner" "$out"
  done
  out="$(run_hook "$agent" "fix(tray): x"$'\n\n'"Why it broke.")"
  eq "body keeps its text" "fix(tray): x"$'\n\n'"Why it broke."$'\n\n'"$owner" "$out"
  out="$(run_hook "$agent" "fix(tray): x"$'\n\n'"Refs: #12")"
  eq "joins an existing trailer block" "fix(tray): x"$'\n\n'"Refs: #12"$'\n'"$owner" "$out"
}

test_the_owner_line_is_added_once() {
  local out
  out="$(run_hook "$agent" "fix(tray): x"$'\n\n'"$owner")"
  eq "already credited" 1 "$(owner_lines "$out")"
  printf 'fix(tray): x\n' > "$scratch/msg"
  ( cd "$scratch" && for _ in 1 2 3; do
      GIT_AUTHOR_NAME=someone GIT_AUTHOR_EMAIL="$agent" "$hook" "$scratch/msg" commit HEAD
    done )
  eq "amended three times" 1 "$(owner_lines "$(cat "$scratch/msg")")"
  out="$(run_hook "$agent" "fix(tray): x"$'\n\n'"Co-authored-by: Someone <someone@example.invalid>")"
  eq "beside another co-author" "fix(tray): x"$'\n\n'"Co-authored-by: Someone <someone@example.invalid>"$'\n'"$owner" "$out"
}

test_other_authors_are_left_alone() {
  local out email
  for email in "kmrh47@proton.me" "45654430+KMRH47@users.noreply.github.com" "someone@example.invalid" \
    "kmrh47+agents@proton.me.example.invalid" "xkmrh47+agents@proton.me"; do
    out="$(run_hook "$email" "fix(tray): x"$'\n\n'"Why it broke.")"
    eq "untouched, $email" "fix(tray): x"$'\n\n'"Why it broke." "$out"
  done
}

commit_message() {
  local repo="$scratch/repo-$1" email=$2
  shift 2
  git init --quiet "$repo"
  git -C "$repo" config core.hooksPath "$scratch/hooks"
  git -C "$repo" config user.name local
  git -C "$repo" config user.email "$email"
  git -C "$repo" commit --quiet --allow-empty "$@" >/dev/null 2>&1 || { echo "commit failed"; return; }
  git -C "$repo" log -1 --format=%B
}

test_git_hands_the_hook_the_author() {
  eq "agent identity from the config" 1 "$(owner_lines "$(commit_message config "$agent" -m "fix(tray): x")")"
  eq "owner identity from the config" 0 "$(owner_lines "$(commit_message plain "kmrh47@proton.me" -m "fix(tray): x")")"
  eq "agent named with --author" 1 \
    "$(owner_lines "$(commit_message flag "kmrh47@proton.me" --author="a <$agent>" -m "fix(tray): x")")"
  eq "owner named with --author" 0 \
    "$(owner_lines "$(commit_message back "$agent" --author="o <kmrh47@proton.me>" -m "fix(tray): x")")"
  eq "message read from a file" 1 \
    "$(owner_lines "$(printf 'fix(tray): x\n\nWhy.\n' | commit_message file "$agent" -F -)")"
}

main() {
  printf '[prepare-commit-msg] tests\n'
  test_agent_commits_get_the_owner_line
  test_the_owner_line_is_added_once
  test_other_authors_are_left_alone
  test_git_hands_the_hook_the_author
  printf '\nSummary: %d passed, %d failed\n' "$passed" "$failed"
  [ "$failed" -eq 0 ]
}

main "$@"
