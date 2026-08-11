#!/usr/bin/env bash
# Unit tests for commit-msg-lib.sh. Run: bash .githooks/commit-msg.test.sh
set -u

self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
. "$self_dir/commit-msg-lib.sh"

passed=0
failed=0
failed_names=()
ok()   { passed=$((passed + 1)); printf '  ok   %s\n' "$1"; }
fail() { failed=$((failed + 1)); failed_names+=("$1"); printf '  FAIL %s -- %s\n' "$1" "$2"; }

eq() { # name expected actual
  if [ "$2" = "$3" ]; then ok "$1"; else fail "$1" "expected [$2] got [$3]"; fi
}

# ---------------------------------------------------------------------------
test_singular() {
  local cases=(plugins:plugin libs:lib apps:app tools:tool services:service daemons:daemon ui:ui bins:bin)
  for c in "${cases[@]}"; do
    IFS=: read -r in exp <<<"$c"
    eq "singular($in)" "$exp" "$(qol_singular "$in")"
  done
}

# The smart strip: redundant family prefix dies, namespace prefix survives.
test_member_scope() {
  local cases=(
    "plugins:plugin-alt-tab:alt-tab"
    "plugins:plugin-os-themes:os-themes"
    "plugins:plugin-window-actions:window-actions"
    "libs:qol-color:qol-color"
    "libs:qol-plugin-api:qol-plugin-api"
    "libs:qol-plugin-daemon:qol-plugin-daemon"
    "apps:qol-tray:qol-tray"
    "tools:qol-cli:qol-cli"
    "services:service-sync:sync"
    "daemons:daemon-bar:bar"
    "daemons:qol-x-daemon:qol-x-daemon"
    "ui:ui-widget:widget"
    "plugins:plugin:plugin"
  )
  for c in "${cases[@]}"; do
    IFS=: read -r fam mem exp <<<"$c"
    eq "member_scope($fam,$mem)" "$exp" "$(qol_member_scope "$fam" "$mem")"
  done
}

# Build a fixture workspace from "family/member ..." args.
mk_fixture() {
  local root="$1"; shift
  mkdir -p "$root"
  local fams=() seen=" "
  for fm in "$@"; do
    local fam="${fm%%/*}" mem="${fm#*/}"
    mkdir -p "$root/$fam/$mem"
    : >"$root/$fam/$mem/Cargo.toml"
    case "$seen" in *" $fam "*) ;; *) fams+=("$fam"); seen="$seen$fam " ;; esac
  done
  {
    printf '[workspace]\nmembers = ['
    local first=1
    for f in "${fams[@]}"; do
      [ "$first" = 1 ] && first=0 || printf ', '
      printf '"%s/*"' "$f"
    done
    printf ']\n'
  } >"$root/Cargo.toml"
}

test_derive_today() {
  local root; root="$(mktemp -d)"
  mk_fixture "$root" \
    plugins/alt-tab plugins/template \
    libs/qol-color libs/qol-plugin-api apps/qol-tray tools/qol-cli
  local got exp
  got="$(qol_derive_scopes "$root" | tr '\n' ' ')"
  exp="alt-tab apps build ci cli color deps dev emu libs plugin-api plugins qol-cli qol-color qol-plugin-api qol-tray settings template tools tray workspace "
  eq "derive(today's layout)" "$exp" "$got"
  rm -rf "$root"
}

# Proves scaling: a brand-new services/ family derives correctly with NO code change.
test_derive_scales_to_new_family() {
  local root; root="$(mktemp -d)"
  mk_fixture "$root" plugins/alt-tab services/service-sync services/qol-special
  local got exp
  got="$(qol_derive_scopes "$root" | tr '\n' ' ')"
  exp="alt-tab build ci deps dev emu plugins qol-special services settings special sync workspace "
  eq "derive(new services/ family)" "$exp" "$got"
  rm -rf "$root"
}

test_derive_skips_dirs_without_a_manifest() {
  local root; root="$(mktemp -d)"
  mk_fixture "$root" tools/qol-cli
  mkdir -p "$root/tools/__pycache__"
  local got exp
  got="$(qol_derive_scopes "$root" | tr '\n' ' ')"
  exp="build ci cli deps dev emu qol-cli settings tools workspace "
  eq "derive(stray non-crate dir)" "$exp" "$got"
  rm -rf "$root"
}

test_derive_accepts_the_repo_scopes_in_daily_use() {
  local root; root="$(cd "$self_dir/.." && pwd -P)"
  if [ ! -f "$root/Cargo.toml" ]; then
    ok "derive(repo): no workspace manifest, skipped"
    return 0
  fi
  local allowed; allowed="$(qol_derive_scopes "$root")"
  local s
  for s in qol-tray tray terminal-sessions build-identity config runtime alt-tab \
           workspace build ci deps dev emu settings; do
    if printf '%s\n' "$allowed" | grep -qxF "$s"; then
      ok "repo scope: $s"
    else
      fail "repo scope: $s" "missing from the derived vocabulary"
    fi
  done
}

test_derive_no_workspace_is_empty() {
  local root; root="$(mktemp -d)"
  eq "derive(no Cargo.toml)" "" "$(qol_derive_scopes "$root")"
  rm -rf "$root"
}

test_check_subject() {
  local allowed
  allowed="$(printf '%s\n' alt-tab launcher template qol-color qol-plugin-api qol-tray qol-cli plugins libs workspace)"
  # subject @@ ok|no @@ reason-substring-when-no
  local cases=(
    "fix(alt-tab): correct picker ordering@@ok@@"
    "fix(qol-color): clamp channel overflow@@ok@@"
    "fix(qol-tray, hotkeys): regrab on resume@@ok@@"
    "refactor(qol-tray, qol-cli): extract cli crate@@ok@@"
    "chore(workspace): bump shared deps@@ok@@"
    "refactor(plugins): inject id across family@@ok@@"
    "feat(qol-plugin-api)!: break the action trait@@ok@@"
    "docs: refresh the readme@@ok@@"
    "wip(launcher): half-built picker@@ok@@"
    "fix(qol-tray, garbage): tail token is not validated@@ok@@"
    "fix(tray): regrab on resume@@no@@unknown scope 'tray'"
    "chore(plugin-template): bump version@@no@@unknown scope 'plugin-template'"
    "fix(plugin-alt-tab): redundant prefix form@@no@@unknown scope 'plugin-alt-tab'"
    "fix(nonsense): nope@@no@@unknown scope 'nonsense'"
    "fix(hotkeys, qol-tray): member must be first@@no@@unknown scope 'hotkeys'"
    "Fix(alt-tab): capitalised type@@no@@conventional"
    "feature(alt-tab): unknown type@@no@@conventional"
    "fix(alt-tab):no space after colon@@no@@conventional"
    "fix(alt-tab): trailing period.@@no@@trailing period"
    "fix(alt-tab): added the thing@@no@@imperative"
    "chore: tidy up loose ends@@ok@@"
    "refactor(qol-tray): trait-based policy backends and journal split@@no@@imperative"
    "feat(qol-cli): count-based line copy in the disk view@@no@@imperative"
    "fix(qol-tray): top-anchored scroll for the disk view@@no@@imperative"
    "fix(qol-cli): sessions wait skips the send echo@@no@@imperative"
    "fix(qol-tray): unsupported rows save the contract default@@no@@imperative"
    "feat(workspace): shared libs and headless profile sync@@no@@imperative"
    "fix(qol-tray): removing the stale entry@@no@@imperative"
    "fix(qol-tray): read the contract default@@ok@@"
    "perf(qol-tray): speed up the first paint@@ok@@"
    "fix(qol-tray): ping the daemon before dispatch@@ok@@"
    "fix(qol-tray): embed the manifest in the binary@@ok@@"
    "fix(qol-tray): pass the wid through@@ok@@"
    "fix(qol-tray): address the review findings@@ok@@"
    "fix(qol-tray): focus the picker on show@@ok@@"
    "refactor(qol-tray): auto-prune orphaned links@@ok@@"
    "fix(qol-tray): self-heal the stale fingerprint@@ok@@"
  )
  for c in "${cases[@]}"; do
    local subj="${c%%@@*}" rest="${c#*@@}" exp sub reason rc
    exp="${rest%%@@*}"; sub="${rest#*@@}"
    reason="$(qol_check_subject "$subj" "$allowed")"; rc=$?
    if [ "$exp" = ok ]; then
      [ "$rc" -eq 0 ] && ok "accept: $subj" || fail "accept: $subj" "rejected: $reason"
    else
      if [ "$rc" -ne 0 ]; then
        case "$reason" in
          *"$sub"*) ok "reject: $subj" ;;
          *) fail "reject: $subj" "wrong reason: [$reason] want [$sub]" ;;
        esac
      else
        fail "reject: $subj" "accepted but should reject"
      fi
    fi
  done

  # length boundary (>72)
  local long reason rc
  long="fix(alt-tab): $(printf 'x%.0s' {1..70})"
  reason="$(qol_check_subject "$long" "$allowed")"; rc=$?
  { [ "$rc" -ne 0 ] && [[ "$reason" == *"too long"* ]]; } \
    && ok "reject: over 72 chars" || fail "reject: over 72 chars" "rc=$rc reason=$reason"

  # empty allowed set -> format checked, membership skipped
  reason="$(qol_check_subject "fix(anything-goes): x" "")"; rc=$?
  [ "$rc" -eq 0 ] && ok "accept: any scope when no vocabulary" \
    || fail "accept: any scope when no vocabulary" "rejected: $reason"
}

test_check_body() {
  local b reason rc i=0
  local bad=(
    "feat(qol-tray): x"$'\n\n'"Co-authored-by: Claude <noreply@anthropic.com>"
    "feat(qol-tray): x"$'\n\n'"🤖 Generated with a tool"
    "feat(qol-tray): x"$'\n\n'"co-authored-by: some AI helper"
  )
  for b in "${bad[@]}"; do
    i=$((i + 1))
    reason="$(qol_check_body "$b")"; rc=$?
    [ "$rc" -ne 0 ] && ok "body reject #$i" || fail "body reject #$i" "accepted"
  done
  local good="feat(qol-tray): add deeplink routing"$'\n\n'"This explains the why."
  reason="$(qol_check_body "$good")"; rc=$?
  [ "$rc" -eq 0 ] && ok "body accept: clean" || fail "body accept: clean" "rejected: $reason"
}

main() {
  printf '[commit-msg-lib] unit tests\n'
  test_singular
  test_member_scope
  test_derive_today
  test_derive_scales_to_new_family
  test_derive_skips_dirs_without_a_manifest
  test_derive_accepts_the_repo_scopes_in_daily_use
  test_derive_no_workspace_is_empty
  test_check_subject
  test_check_body
  printf '\nSummary: %d passed, %d failed\n' "$passed" "$failed"
  if [ "$failed" -gt 0 ]; then
    printf 'Failed:\n'
    for n in "${failed_names[@]}"; do printf '  - %s\n' "$n"; done
    exit 1
  fi
}

main "$@"