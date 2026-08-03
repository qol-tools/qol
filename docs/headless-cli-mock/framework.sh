#!/usr/bin/env bash
set -o pipefail
# mock-headless framework — qol-headless in bash.
# V1: help/doctor/--json/aliases/subcommands/fallback.  V2: lifecycle safe-defaults.
# V3: canonical daemon alias, dashed→bare.  V4: config show/get.  V5: doctor --fix.

MOCK_APP_ID=""; MOCK_BIN=""; MOCK_ABOUT=""
MOCK_DEFAULT=(); MOCK_FALLBACK=""; MOCK_DOCTOR_FN=""
declare -A MOCK_META MOCK_HANDLER MOCK_JSON MOCK_ALIAS MOCK_SUBS MOCK_LIFECYCLE
SEP=$'\x1f'

MOCK_EXIT_SUCCESS=0; MOCK_EXIT_RUNTIME=1; MOCK_EXIT_USAGE=64
MOCK_STATE="${MOCK_STATE:-/tmp/qol-mock-state}"; mkdir -p "$MOCK_STATE"

mock_init() { MOCK_APP_ID="$1"; MOCK_BIN="$2"; MOCK_ABOUT="$3"; shift 3; MOCK_DEFAULT=("$@"); }
mock_command() {
  local name="$1"; shift; local about="" usage="" detail="" output="" exit_behavior="" handler="" json=0 alias=""
  while [ $# -gt 0 ]; do case "$1" in
    --about) about="$2"; shift 2 ;; --usage) usage="$2"; shift 2 ;; --detail) detail="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;; --exit) exit_behavior="$2"; shift 2 ;; --handler) handler="$2"; shift 2 ;;
    --json) json=1; shift ;; --alias) alias="$2"; shift 2 ;; *) shift ;;
  esac; done
  MOCK_META["$name"]="${about}${SEP}${usage}${SEP}${detail}${SEP}${output}${SEP}${exit_behavior}"
  MOCK_HANDLER["$name"]="$handler"; [ "$json" = 1 ] && MOCK_JSON["$name"]=1; [ -n "$alias" ] && MOCK_ALIAS["$alias"]="$name"
}
mock_subcommand() { MOCK_SUBS["$1"]="${MOCK_SUBS[$1]:-} $2"; }
mock_fallback() { MOCK_FALLBACK="$1"; }
mock_doctor() { MOCK_DOCTOR_FN="$1"; }
mock_config() {
  MOCK_CONFIG_FN="$1"
  mock_command "config" --about "Inspect configuration." --usage "$MOCK_BIN config <show|get>"
  mock_subcommand "config" "show"; mock_subcommand "config" "get"
  mock_command "config show" --about "Print the effective configuration." --usage "$MOCK_BIN config show" --output "JSON object." --exit "Exits zero when config is readable." --json --handler _mock_config_show
  mock_command "config get" --about "Print one config key." --usage "$MOCK_BIN config get <key>" --output "Key value or null." --exit "Exits non-zero when the key is unknown." --handler _mock_config_get
}
_mock_config_show() { "$MOCK_CONFIG_FN"; }
_mock_config_get() {
  local key="${1:-}"
  [ -z "$key" ] && { echo "$MOCK_BIN: config get requires a key" >&2; return "$MOCK_EXIT_USAGE"; }
  local val; val="$("$MOCK_CONFIG_FN" | grep -o '"'"$key"'":[^,}]*' | head -1)"
  [ -z "$val" ] && { echo "$MOCK_BIN: key '$key' not found in config" >&2; return 1; }
  echo "$val"
}
mock_lifecycle() {
  MOCK_LIFECYCLE[feature]="$1"; MOCK_LIFECYCLE[start_cmd]="$2"
  if [ -n "$4" ]; then mock_command "status" --about "Report service status." --usage "$MOCK_BIN status" --output "Running or stopped." --exit "Exits zero whether or not a service is running." --handler "$4"; fi
  if [ -n "$3" ]; then mock_command "kill" --about "Stop the running service." --usage "$MOCK_BIN kill" --output "No output on success." --exit "Exits zero whether or not a service is currently running." --handler "$3"; fi
  if [ -n "$2" ] && [ "$2" != "daemon" ]; then local sh="${MOCK_HANDLER[$2]:-}"
    [ -n "$sh" ] && mock_command "daemon" --about "Start the service (canonical)." --usage "$MOCK_BIN daemon" --output "Lifecycle diagnostics." --exit "Exits non-zero on failure." --handler "$sh"
  fi
  # V3: bare aliases for dashed commands
  for key in "${!MOCK_META[@]}"; do if [ "${key:0:2}" = "--" ]; then local bare="${key#--}"; [ -z "${MOCK_META[$bare]+x}" ] && MOCK_ALIAS["$bare"]="$key"; fi; done
}
mock_check() { local fix="" id="$1" status="$2" message="$3"; shift 3; while [ $# -gt 0 ]; do case "$1" in --fix) fix=",\"fix\":\"$2\""; shift 2 ;; --fixed) fix="$fix,\"fixed\":true"; shift ;; *) shift ;; esac; done; printf '{"id":"%s","status":"%s","message":"%s"%s}\n' "$id" "$status" "$message" "$fix"; }
mock_start_daemon() { echo running > "$MOCK_STATE/$1.state"; echo "$1: daemon started"; return 0; }
mock_stop_daemon() { rm -f "$MOCK_STATE/$1.state"; echo "$1: daemon stopped"; return 0; }
mock_daemon_status() { if [ -f "$MOCK_STATE/$1.state" ]; then echo "$1: daemon running${2:+ ($2)}"; else echo "$1: daemon not running"; fi; }

mock_help_for() {
  local key="$1"
  if [ -z "$key" ]; then
    echo "$MOCK_BIN — $MOCK_ABOUT"; echo; echo "Usage: $MOCK_BIN <command> [args...]"
    [ -n "${MOCK_DEFAULT[*]}" ] && echo "       (no arguments selects: ${MOCK_DEFAULT[*]})"
    echo; echo "Commands:"
    for k in "${!MOCK_META[@]}"; do case "$k" in *" "*) continue ;; esac; printf '  %-16s %s\n' "$k" "${MOCK_META[$k]%%$SEP*}"
      for s in ${MOCK_SUBS[$k]:-}; do printf '    %-14s %s\n' "$s" "${MOCK_META[$k $s]%%$SEP*}"; done
    done
    echo "  doctor             Run read-only health checks."; echo "  help               Show help."
    echo; echo "Global flags:"; echo "  --json  Request structured JSON output from commands that support it."
    return
  fi
  local meta="${MOCK_META[$key]:-}"
  if [ -z "$meta" ]; then echo "$MOCK_BIN: unknown command '$key'" >&2; return "$MOCK_EXIT_USAGE"; fi
  IFS="$SEP" read -r about usage detail output exit_behavior <<< "$meta"
  echo "$key — $about"; echo; echo "Usage: ${usage:-$MOCK_BIN $key}"
  [ -n "$detail" ] && printf '\n%s\n' "$detail"; echo; echo "Output:"; echo "  ${output:-N/A}"
  echo; echo "Exit:"; echo "  ${exit_behavior:-N/A}"; echo
  if [ -n "${MOCK_JSON[$key]+x}" ]; then echo "Supports --json."; else echo "Does not support --json."; fi
}

mock_doctor_report() {
  local json_mode="$1" fix_mode="$2" out status="ok"
  if [ "$fix_mode" = 1 ]; then MOCK_FIX=1; out="$("$MOCK_DOCTOR_FN" 2>/dev/null)"; MOCK_FIX=0; else out="$("$MOCK_DOCTOR_FN" 2>/dev/null)"; fi
  if printf '%s' "$out" | grep -q '"status":"fail"'; then status="fail"; elif printf '%s' "$out" | grep -q '"status":"warn"'; then status="warn"; fi
  if [ "$json_mode" = 1 ]; then printf '{"plugin_id":"%s","status":"%s","checks":[%s]}\n' "$MOCK_APP_ID" "$status" "$(printf '%s' "$out" | paste -sd, -)"
  else printf '%s\n' "$out" | sed 's/^{"id":"\([^"]*\)","status":"\([^"]*\)","message":"\([^"]*\)".*$/[\2] \1 - \3/'; echo; echo "status: $status"; fi
  case "$status" in ok) return 0 ;; warn) return 1 ;; fail) return 2 ;; esac
}

mock_dispatch() {
  local -a raw; raw=("$@")
  local json=0 fix=0; local -a args=()
  for a in "${raw[@]}"; do case "$a" in
    --json) json=1 ;; --fix) fix=1 ;; -h|--help) args+=("help") ;; *) args+=("$a") ;;
  esac; done
  local n="${#args[@]}"

  for ((i = 1; i < n - 1; i++)); do
    if [ "${args[$i]}" = "help" ]; then echo "$MOCK_BIN: 'help' may appear as the first token or the final token, not in the middle" >&2; return "$MOCK_EXIT_USAGE"; fi
  done

  if [ "$n" -eq 0 ]; then
    if [ -n "${QOL_MOCK_DAEMON:-}" ] && [ -n "${MOCK_LIFECYCLE[start_cmd]:-}" ]; then args=("${MOCK_LIFECYCLE[start_cmd]}")
    elif [ -n "${MOCK_LIFECYCLE[feature]:-}" ]; then args=("status")
    elif [ "${#MOCK_DEFAULT[@]}" -gt 0 ]; then args=("${MOCK_DEFAULT[@]}"); else args=("help"); fi
    n="${#args[@]}"
  fi

  if [ "${args[0]}" = "help" ]; then
    [ "$n" -eq 1 ] && { mock_help_for ""; return $?; }
    mock_help_for "$(mock_resolve $((n - 1)) "${args[@]:1}")"; return $?
  fi
  if [ "${args[$((n - 1))]}" = "help" ]; then
    mock_help_for "$(mock_resolve $((n - 1)) "${args[@]:0:$((n - 1))}")"; return $?
  fi

  local key; key="$(mock_resolve "$n" "${args[@]}")"
  [ -z "${MOCK_META[$key]+x}" ] && [ -n "${MOCK_ALIAS[$key]:-}" ] && key="${MOCK_ALIAS[$key]}"

  if [ "$key" = "doctor" ] && [ -n "$MOCK_DOCTOR_FN" ]; then mock_doctor_report "$json" "$fix"; return $?; fi
  if [ -z "${MOCK_META[$key]+x}" ]; then
    [ -n "$MOCK_FALLBACK" ] && { "$MOCK_FALLBACK" "$@"; return $?; }
    echo "$MOCK_BIN: unknown command '$key'" >&2; echo "Run '$MOCK_BIN help' for usage." >&2; return "$MOCK_EXIT_USAGE"
  fi
  if [ "$json" = 1 ] && [ -z "${MOCK_JSON[$key]+x}" ]; then echo "$MOCK_BIN: '$key' does not support --json" >&2; return "$MOCK_EXIT_USAGE"; fi

  local -a operands=(); case "$key" in *" ") operands=("${args[@]:2}") ;; *) operands=("${args[@]:1}") ;; esac
  local handler="${MOCK_HANDLER[$key]:-}"
  [ -n "$handler" ] && { "$handler" "${operands[@]}"; return $?; }
  echo "$MOCK_BIN: $key executed (mock)"
}

mock_resolve() {
  [ "$1" -eq 1 ] && { echo "${2}"; return; }
  local two="$2 $3"; [ -n "${MOCK_META[$two]+x}" ] && { echo "$two"; return; }
  echo "$2"
}
