#!/usr/bin/env bash
# Pure, sourceable helpers for the qol monorepo commit-msg gate.
# Sourcing has no side effects; every function is unit-tested in commit-msg.test.sh.
#
# Scope model (decided 2026-06-03): the first scope token is a workspace MEMBER
# or an UMBRELLA. A member's canonical scope is its directory basename with the
# redundant family prefix dropped: a crate that repeats its parent folder's name
# as a prefix (plugin-alt-tab inside plugins/) loses it; a crate carrying a real
# namespace (qol-color in libs/) keeps it. This is derived from the folder layout
# at runtime, so a new family (services/, daemons/, ...) needs no edit here.

QOL_TYPES='feat|fix|refactor|chore|docs|test|perf|wip|style|ci|build|revert'
QOL_UMBRELLA_EXTRA='workspace'   # root-level scope, owned by no member

# Singularize a family directory name by dropping one trailing 's'.
#   plugins->plugin  libs->lib  apps->app  tools->tool  services->service
#   ui->ui (no trailing 's' -> unchanged)
qol_singular() {
  case "$1" in
    *s) printf '%s' "${1%s}" ;;
    *)  printf '%s' "$1" ;;
  esac
}

# Canonical scope for a member, given its family dir name and its basename.
# Drops a leading "<family-singular>-" prefix; otherwise returns the basename.
#   plugins plugin-alt-tab    -> alt-tab
#   libs    qol-color         -> qol-color
#   libs    qol-plugin-api    -> qol-plugin-api   (leading token is qol-, not lib-)
#   services service-foo      -> foo
qol_member_scope() {
  sing="$(qol_singular "$1")"
  case "$2" in
    "$sing"-?*) printf '%s' "${2#"$sing"-}" ;;
    *)          printf '%s' "$2" ;;
  esac
}

# Echo the workspace member directory patterns from [workspace].members, one per
# line (e.g. "apps/*"). Reads only the members array, ignoring default-members.
qol_member_patterns() {
  [ -f "$1" ] || return 0
  awk '
    /^[[:space:]]*members[[:space:]]*=/ { grab=1 }
    grab {
      s=$0
      while (match(s, /"[^"]+"/)) {
        print substr(s, RSTART+1, RLENGTH-2)
        s=substr(s, RSTART+RLENGTH)
      }
      if (index($0, "]")) grab=0
    }
  ' "$1"
}

# Derive the full allowed first-scope vocabulary for a workspace root:
# every member's canonical scope, plus every family-dir name (umbrella), plus
# the workspace umbrella. Sorted, unique. Empty when root has no workspace manifest.
qol_derive_scopes() {
  root="$1"; cargo="$1/Cargo.toml"
  [ -f "$cargo" ] || return 0
  {
    qol_member_patterns "$cargo" | while IFS= read -r pat; do
      [ -n "$pat" ] || continue
      fam="${pat%%/*}"
      printf '%s\n' "$fam"
      for dir in "$root/$fam"/*/; do
        [ -d "$dir" ] || continue
        qol_member_scope "$fam" "$(basename "$dir")"; printf '\n'
      done
    done
    printf '%s\n' "$QOL_UMBRELLA_EXTRA"
  } | sed '/^$/d' | sort -u
}

# Echo the scope string inside the first (...) of a conventional subject; empty if none.
qol_extract_scope() {
  printf '%s' "$1" | sed -nE "s/^($QOL_TYPES)\(([^)]*)\)!?:.*/\2/p"
}

# Echo the first comma-separated token of a scope, trimmed.
qol_first_member() {
  printf '%s' "$1" | sed -E 's/,.*//; s/^[[:space:]]+//; s/[[:space:]]+$//'
}

# Validate a subject against an allowed-scope set (newline-separated; may be empty
# to skip the membership check). Echoes a reason and returns 1 on the first
# violation; returns 0 when the subject is acceptable.
qol_check_subject() {
  subject="$1"; allowed="$2"

  printf '%s\n' "$subject" | grep -qE "^($QOL_TYPES)(\([a-z0-9 ,._/-]+\))?!?: .+" \
    || { printf '%s' "not a conventional commit (<type>(scope)?: summary)"; return 1; }

  [ "${#subject}" -le 72 ] || { printf 'subject too long (%s > 72 chars)' "${#subject}"; return 1; }

  summary="${subject#*: }"
  case "$summary" in
    *.) printf '%s' "drop the trailing period in the subject"; return 1 ;;
  esac

  word="$(printf '%s' "$summary" | awk '{print tolower($1)}')"
  case "$word" in
    added|fixed|changed|removed|updated|refactored|implemented|\
    adds|fixes|changes|removes|updates|adding|fixing|changing|removing|updating)
      printf "use imperative mood, not '%s' (e.g. add / fix / remove)" "$word"; return 1 ;;
  esac

  scope="$(qol_extract_scope "$subject")"
  [ -n "$scope" ] || return 0          # scopeless commits are allowed
  [ -n "$allowed" ] || return 0        # no workspace vocabulary -> format-only

  member="$(qol_first_member "$scope")"
  printf '%s\n' "$allowed" | grep -qxF "$member" \
    || { printf "unknown scope '%s' (first token must be a workspace member or umbrella)" "$member"; return 1; }
  return 0
}

# Reject AI-attribution / co-author trailers anywhere in the body.
qol_check_body() {
  printf '%s\n' "$1" | grep -qiE 'co-authored-by:.*(claude|anthropic|\bai\b)|generated with|🤖|noreply@anthropic\.com' \
    && { printf '%s' "AI attribution / co-author trailers are not allowed"; return 1; }
  return 0
}