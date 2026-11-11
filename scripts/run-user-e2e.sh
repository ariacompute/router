#!/usr/bin/env bash
# User-facing black-box E2E checks for aria-router, mapped to task.md T-numbers.
#
# Prerequisites (default mode):
#   - aria-router already serving on both planes (data + management). The
#     default mode NEVER builds, installs, starts, stops or upgrades a process.
#   - curl, jq and standard coreutils. python3 only for optional offline groups.
#
# Modes:
#   default          probe an already-running instance through its URLs
#   --with-fixtures  additionally start isolated LOCAL fixture instances from
#                    tiny YAML examples (needs a prebuilt aria-router binary;
#                    never compiles, downloads or touches the default instance)
#   --with-local     additionally run offline contract / bench / UI-audit cmds
#
# Usage:
#   ./scripts/run-user-e2e.sh [options]
#   ROUTER_URL=http://127.0.0.1:8899 MGMT_URL=http://127.0.0.1:8090 \
#     ./scripts/run-user-e2e.sh
#   ./scripts/run-user-e2e.sh --list
#   ./scripts/run-user-e2e.sh --self-test
#   ./scripts/run-user-e2e.sh --read-only --group cli,data,decisioners
#   ./scripts/run-user-e2e.sh --with-local --with-fixtures --strict-skip
#   ./scripts/run-user-e2e.sh --task-focus T90,T91,T92 --with-fixtures
#
# Options:
#   --data-url URL         OpenAI-compatible data plane (default 127.0.0.1:8899)
#   --mgmt-url URL         management plane (default 127.0.0.1:8090)
#   --admin-user USER      local admin username for the login journey
#   --admin-password PASS  local admin password
#   --api-key KEY          pre-existing data-plane key (sk-aria_ / sk-bf-)
#   --session-token TOKEN  pre-existing management session token (not an API key)
#   --semantic-model NAME  semantic entrypoint model id
#   --agent-model NAME     agent entrypoint model id
#   --afm-d-model NAME     afm-d entrypoint model id
#   --bypass-model NAME    concrete provider name for bypass checks
#   --timeout SEC          per-request timeout (default 15)
#   --sse-timeout SEC      stream read timeout (default 10)
#   --out-dir DIR          report directory (default ./out)
#   --group LIST           comma-separated groups to run (default: all)
#   --skip LIST            comma-separated groups to skip
#   --read-only            skip all mutating requests (negative cases still run)
#   --allow-mutating       also run irreversible writes (config reload,
#                          provider upsert, trust publish, serve account sync)
#   --with-local           enable local-optional / bench-optional / task-offline
#   --with-fixtures        enable task-fixtures (isolated local instances)
#   --task-focus LIST      comma-separated T-numbers; only run mapped groups
#   --strict-skip          exit 1 when any case is skipped
#   --list                 list the case catalog (group / case / task / notes)
#   --self-test            verify the assertion helpers offline and exit
#   -h, --help             show this help
#
# Environment (equivalent to the flags above, flags win):
#   ROUTER_URL, MGMT_URL, ADMIN_USER, ADMIN_PASSWORD, ROUTER_API_KEY,
#   ROUTER_SESSION_TOKEN, TIMEOUT, SSE_TIMEOUT, OUT_DIR, ARIA_ROUTER_BIN,
#   ENTRY_SEMANTIC, ENTRY_AGENT, ENTRY_AFMD, BYPASS_MODEL, BENCH_CORPUS
#
# Exit codes:
#   0  all executed cases passed (SKIP is reported but does not fail by default)
#   1  at least one FAIL, or --strict-skip with skipped cases
#   2  usage / environment error (missing tools, unreachable service)
#
# Safety notes:
#   - Mutating cases use a per-run random prefix and are cleaned up on EXIT.
#     Irreversible writes are SKIPped unless --allow-mutating is explicit.
#   - Tokens, key secrets and full response bodies are never written to stdout
#     or to the JSON report.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CASES_LIB="$ROOT/scripts/user-e2e-cases.sh"
FIXTURES_LIB="$ROOT/scripts/user-e2e-fixtures.sh"
TASK_MAP_FILE="$ROOT/scripts/user-e2e-task-map.tsv"

DATA_URL="${ROUTER_URL:-http://127.0.0.1:8899}"
MGMT_URL="${MGMT_URL:-http://127.0.0.1:8090}"
ADMIN_USER="${ADMIN_USER:-}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"
ROUTER_API_KEY="${ROUTER_API_KEY:-}"
SESSION_TOKEN="${ROUTER_SESSION_TOKEN:-}"
TIMEOUT="${TIMEOUT:-15}"
AGENT_TIMEOUT="${AGENT_TIMEOUT:-60}"
SSE_TIMEOUT="${SSE_TIMEOUT:-10}"
OUT_DIR="${OUT_DIR:-$ROOT/out}"
ARIA_ROUTER_BIN="${ARIA_ROUTER_BIN:-}"
BENCH_CORPUS="${BENCH_CORPUS:-}"
ENTRY_SEMANTIC="${ENTRY_SEMANTIC:-ariacompute/semantic-auto}"
ENTRY_AGENT="${ENTRY_AGENT:-ariacompute/agent-auto}"
ENTRY_AFMD="${ENTRY_AFMD:-ariacompute/afm-d-auto}"
BYPASS_MODEL="${BYPASS_MODEL:-local/general}"

READ_ONLY=0
ALLOW_MUTATING=0
WITH_LOCAL=0
WITH_FIXTURES=0
STRICT_SKIP=0
LIST=0
SELF_TEST=0
GROUP_FILTER=""
SKIP_FILTER=""
TASK_FOCUS=""

ALL_GROUPS=(cli auth keys data decisioners mgmt-read mgmt-write dashboard \
            task-static task-http local-optional bench-optional \
            task-fixtures task-offline)

PASS=0
FAIL=0
SKIP=0
START_EPOCH="$SECONDS"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
WORK_DIR=""
RESULTS_FILE=""
LAST_BODY=""
LAST_HEADERS=""
CURL_ERR=""
LAST_STATUS="000"
LAST_CONTENT_TYPE=""
LAST_DETAIL=""
E2E_CT="application/json"
TEST_PREFIX="e2e-$$-$(date +%s)"
SESSION_ARGS=(-H "X-Aria-E2E: 1")
DATA_KEY_ARGS=(-H "X-Aria-E2E: 1")
CLEANUP_KIND=()
CLEANUP_ARG=()
E2E_TASK_ID=""
E2E_TASK_KIND=""
AUDIT_TMP_DIR=""
T9X_PATTERN='^T9[0-9]$'
TASK_IDS=()
TASK_STAGE=()
TASK_KIND=()
TASK_GROUP=()
TASK_PATTERN=()
TASK_TARGET=()
TASK_KEYWORD=()

usage() {
  awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
}

die_usage() {
  printf 'error: %s\n\n' "$*" >&2
  usage >&2
  exit 2
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die_usage "missing required command: $1"
}

load_task_map() {
  [[ -f "$TASK_MAP_FILE" ]] || die_usage "missing task map: $TASK_MAP_FILE"
  local task stage summary kind groups pattern target keyword
  while IFS=$'\t' read -r task stage summary kind groups pattern target keyword; do
    [[ -z "$task" || "$task" == \#* ]] && continue
    TASK_IDS+=("$task")
    TASK_STAGE+=("$stage")
    TASK_KIND+=("$kind")
    TASK_GROUP+=("$groups")
    TASK_PATTERN+=("$pattern")
    TASK_TARGET+=("$target")
    TASK_KEYWORD+=("$keyword")
  done < "$TASK_MAP_FILE"
  (( ${#TASK_IDS[@]} > 0 )) || die_usage "task map is empty: $TASK_MAP_FILE"
}

validate_task_focus() {
  [[ -z "$TASK_FOCUS" ]] && return 0
  local -a ids
  local item i known
  IFS=',' read -ra ids <<< "$TASK_FOCUS"
  for item in "${ids[@]}"; do
    known=0
    for i in "${!TASK_IDS[@]}"; do
      [[ "${TASK_IDS[i]}" == "$item" ]] && { known=1; break; }
    done
    (( known )) || die_usage "unknown task number '$item' (see $TASK_MAP_FILE; gaps such as T30/T60-T80 are intentionally unmapped)"
  done
}

group_matches_task() {
  local group="$1" list="$2"
  [[ "$list" == "*" || ",$list," == *",$group,"* ]]
}

task_for_case() {
  local group="$1" id="$2" i glist pat
  (( ${#TASK_IDS[@]} > 0 )) || { printf '%s' "-"; return 0; }
  for i in "${!TASK_IDS[@]}"; do
    pat="${TASK_PATTERN[i]}"
    [[ "$pat" == "*" ]] && continue
    glist="${TASK_GROUP[i]}"
    group_matches_task "$group" "$glist" || continue
    [[ "$id" == $pat ]] && { printf '%s' "${TASK_IDS[i]}"; return 0; }
  done
  for i in "${!TASK_IDS[@]}"; do
    if group_matches_task "$group" "${TASK_GROUP[i]}"; then
      printf '%s' "${TASK_IDS[i]}"
      return 0
    fi
  done
  printf '%s' "-"
}

task_kind_for() {
  local task="$1" i
  for i in "${!TASK_IDS[@]}"; do
    if [[ "${TASK_IDS[i]}" == "$task" ]]; then printf '%s' "${TASK_KIND[i]}"; return 0; fi
  done
  printf '%s' "static"
}

set_task() {
  E2E_TASK_ID="$1"
  E2E_TASK_KIND="$2"
}

clear_task() {
  E2E_TASK_ID=""
  E2E_TASK_KIND=""
}

task_scope() {
  local id="$1" kind="$2" prev_id="$E2E_TASK_ID" prev_kind="$E2E_TASK_KIND" rc=0
  shift 2
  E2E_TASK_ID="$id"
  E2E_TASK_KIND="$kind"
  "$@" || rc=$?
  E2E_TASK_ID="$prev_id"
  E2E_TASK_KIND="$prev_kind"
  return "$rc"
}

build_coverage() {
  local out="$WORK_DIR/coverage.jsonl" all i task p f s n status kinds
  : > "$out"
  all="$(jq -s '.' "$RESULTS_FILE" 2>/dev/null)" || all='[]'
  [[ -n "$all" ]] || all='[]'
  for i in "${!TASK_IDS[@]}"; do
    task="${TASK_IDS[i]}"
    p="$(printf '%s' "$all" | jq --arg t "$task" '[.[]|select(.feature==$t and .status=="PASS")]|length')"
    f="$(printf '%s' "$all" | jq --arg t "$task" '[.[]|select(.feature==$t and .status=="FAIL")]|length')"
    s="$(printf '%s' "$all" | jq --arg t "$task" '[.[]|select(.feature==$t and .status=="SKIP")]|length')"
    kinds="$(printf '%s' "$all" | jq -r --arg t "$task" '[.[]|select(.feature==$t)|.kind]|unique|join(",")')"
    n=$((p + f + s))
    status="UNCOVERED"
    (( n > 0 )) && status="SKIP"
    (( p > 0 )) && status="PASS"
    (( f > 0 )) && status="FAIL"
    jq -nc --arg task "$task" --arg stage "${TASK_STAGE[i]}" \
      --arg kind "${TASK_KIND[i]}" --arg kinds "$kinds" --arg status "$status" \
      --argjson pass "$p" --argjson fail "$f" --argjson skip "$s" \
      '{task:$task,stage:$stage,status:$status,kind:$kind,kinds:$kinds,pass:$pass,fail:$fail,skip:$skip,cases:($pass+$fail+$skip)}' >> "$out"
  done
  jq -s '.' "$out"
}

parse_args() {
  while (( $# > 0 )); do
    case "$1" in
      --data-url)       (( $# >= 2 )) || die_usage "--data-url needs a value"; DATA_URL="$2"; shift 2 ;;
      --mgmt-url)       (( $# >= 2 )) || die_usage "--mgmt-url needs a value"; MGMT_URL="$2"; shift 2 ;;
      --admin-user)     (( $# >= 2 )) || die_usage "--admin-user needs a value"; ADMIN_USER="$2"; shift 2 ;;
      --admin-password) (( $# >= 2 )) || die_usage "--admin-password needs a value"; ADMIN_PASSWORD="$2"; shift 2 ;;
      --api-key)        (( $# >= 2 )) || die_usage "--api-key needs a value"; ROUTER_API_KEY="$2"; shift 2 ;;
      --session-token)  (( $# >= 2 )) || die_usage "--session-token needs a value"; SESSION_TOKEN="$2"; shift 2 ;;
      --semantic-model) (( $# >= 2 )) || die_usage "--semantic-model needs a value"; ENTRY_SEMANTIC="$2"; shift 2 ;;
      --agent-model)    (( $# >= 2 )) || die_usage "--agent-model needs a value"; ENTRY_AGENT="$2"; shift 2 ;;
      --afm-d-model)    (( $# >= 2 )) || die_usage "--afm-d-model needs a value"; ENTRY_AFMD="$2"; shift 2 ;;
      --bypass-model)   (( $# >= 2 )) || die_usage "--bypass-model needs a value"; BYPASS_MODEL="$2"; shift 2 ;;
      --timeout)        (( $# >= 2 )) || die_usage "--timeout needs a value"; TIMEOUT="$2"; shift 2 ;;
      --sse-timeout)    (( $# >= 2 )) || die_usage "--sse-timeout needs a value"; SSE_TIMEOUT="$2"; shift 2 ;;
      --out-dir)        (( $# >= 2 )) || die_usage "--out-dir needs a value"; OUT_DIR="$2"; shift 2 ;;
      --group)          (( $# >= 2 )) || die_usage "--group needs a value"; GROUP_FILTER="$2"; shift 2 ;;
      --skip)           (( $# >= 2 )) || die_usage "--skip needs a value"; SKIP_FILTER="$2"; shift 2 ;;
      --read-only)      READ_ONLY=1; shift ;;
      --allow-mutating) ALLOW_MUTATING=1; shift ;;
      --with-local)     WITH_LOCAL=1; shift ;;
      --with-fixtures)  WITH_FIXTURES=1; shift ;;
      --task-focus)     (( $# >= 2 )) || die_usage "--task-focus needs a value"; TASK_FOCUS="$2"; shift 2 ;;
      --strict-skip)    STRICT_SKIP=1; shift ;;
      --list)           LIST=1; shift ;;
      --self-test)      SELF_TEST=1; shift ;;
      -h|--help)        usage; exit 0 ;;
      *)                die_usage "unknown argument: $1" ;;
    esac
  done
  [[ "$TIMEOUT" =~ ^[0-9]+$ ]] || die_usage "--timeout must be an integer"
  [[ "$SSE_TIMEOUT" =~ ^[0-9]+$ ]] || die_usage "--sse-timeout must be an integer"
  [[ -z "$TASK_FOCUS" || "$TASK_FOCUS" =~ ^T[0-9]+(,T[0-9]+)*$ ]] \
    || die_usage "--task-focus expects comma-separated T-numbers, e.g. T90,T91"
  validate_group_filter "$GROUP_FILTER"
  validate_group_filter "$SKIP_FILTER"
  if (( READ_ONLY )) && (( ALLOW_MUTATING )); then
    die_usage "--read-only and --allow-mutating are mutually exclusive"
  fi
}

validate_group_filter() {
  local list="$1" item known
  [[ -z "$list" ]] && return 0
  local -a parts
  IFS=',' read -ra parts <<< "$list"
  for item in "${parts[@]}"; do
    known=0
    local g
    for g in "${ALL_GROUPS[@]}"; do
      [[ "$item" == "$g" ]] && known=1
    done
    (( known )) || die_usage "unknown group '$item' (valid: ${ALL_GROUPS[*]})"
  done
}

group_enabled() {
  local g="$1" item i
  local -a parts
  if [[ -n "$GROUP_FILTER" ]]; then
    local found=0
    IFS=',' read -ra parts <<< "$GROUP_FILTER"
    for item in "${parts[@]}"; do [[ "$item" == "$g" ]] && found=1; done
    (( found )) || return 1
  fi
  if [[ -n "$SKIP_FILTER" ]]; then
    IFS=',' read -ra parts <<< "$SKIP_FILTER"
    for item in "${parts[@]}"; do [[ "$item" == "$g" ]] && return 1; done
  fi
  if [[ -n "$TASK_FOCUS" ]]; then
    local focus_hit=0
    for i in "${!TASK_IDS[@]}"; do
      [[ ",$TASK_FOCUS," == *",${TASK_IDS[i]},"* ]] || continue
      if group_matches_task "$g" "${TASK_GROUP[i]}"; then focus_hit=1; break; fi
    done
    (( focus_hit )) || return 1
  fi
  return 0
}

groups_need_service() {
  local g
  for g in "${ALL_GROUPS[@]}"; do
    group_enabled "$g" || continue
    case "$g" in
      cli|local-optional|task-static|task-fixtures|task-offline) : ;;
      *) return 0 ;;
    esac
  done
  return 1
}

setup_workspace() {
  WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/aria-router-e2e.XXXXXX" 2>/dev/null)" || {
    printf 'error: cannot create temporary directory\n' >&2
    exit 2
  }
  RESULTS_FILE="$WORK_DIR/cases.jsonl"
  LAST_BODY="$WORK_DIR/last.body"
  LAST_HEADERS="$WORK_DIR/last.headers"
  CURL_ERR="$WORK_DIR/curl.err"
  : > "$RESULTS_FILE"
  : > "$LAST_BODY"
  : > "$LAST_HEADERS"
  : > "$CURL_ERR"
}

http_req() {
  local method="$1" url="$2" body="$3"
  shift 3
  local ct="$E2E_CT"
  E2E_CT="application/json"
  LAST_STATUS="000"
  LAST_CONTENT_TYPE=""
  LAST_DETAIL=""
  : > "$LAST_BODY"
  : > "$LAST_HEADERS"
  : > "$CURL_ERR"
  local -a args
  args=(-sS -m "$TIMEOUT" -o "$LAST_BODY" -D "$LAST_HEADERS" -w '%{http_code}' -X "$method")
  if [[ "$body" != "-" ]]; then
    printf '%s' "$body" > "$WORK_DIR/request.body"
    args+=(--data-binary "@$WORK_DIR/request.body" -H "content-type: $ct")
  fi
  if (( $# > 0 )); then args+=("$@"); fi
  local out rc=0
  out="$(curl "${args[@]}" "$url" 2>"$CURL_ERR")" || rc=$?
  out="${out//$'\r'/}"
  out="${out//$'\n'/}"
  if [[ "$out" =~ ^[0-9]{3}$ ]]; then LAST_STATUS="$out"; else LAST_STATUS="000"; fi
  if (( rc != 0 )) && [[ "$LAST_STATUS" == "000" ]]; then
    LAST_DETAIL="curl failed (rc=$rc): $(head -n1 "$CURL_ERR" 2>/dev/null | cut -c1-120)"
  fi
  return 0
}

http_stream() {
  local method="$1" url="$2" body="$3"
  shift 3
  local ct="$E2E_CT"
  E2E_CT="application/json"
  LAST_STATUS="000"
  LAST_CONTENT_TYPE=""
  LAST_DETAIL=""
  : > "$LAST_BODY"
  : > "$LAST_HEADERS"
  : > "$CURL_ERR"
  local -a args
  args=(-sN -m "$SSE_TIMEOUT" -o "$LAST_BODY" -D "$LAST_HEADERS" -w '%{http_code} %{content_type}' -X "$method")
  if [[ "$body" != "-" ]]; then
    printf '%s' "$body" > "$WORK_DIR/request.body"
    args+=(--data-binary "@$WORK_DIR/request.body" -H "content-type: $ct")
  fi
  if (( $# > 0 )); then args+=("$@"); fi
  local out rc=0
  out="$(curl "${args[@]}" "$url" 2>"$CURL_ERR")" || rc=$?
  LAST_STATUS="$(printf '%s\n' "$out" | awk 'NF {print $1}' | tail -n1)"
  LAST_CONTENT_TYPE="$(printf '%s\n' "$out" | awk 'NF {print $2}' | tail -n1)"
  [[ "$LAST_STATUS" =~ ^[0-9]{3}$ ]] || LAST_STATUS="000"
  if (( rc != 0 )) && [[ "$LAST_STATUS" == "000" ]] && ! grep -q 'data:' "$LAST_BODY" 2>/dev/null; then
    LAST_DETAIL="stream failed (rc=$rc): $(head -n1 "$CURL_ERR" 2>/dev/null | cut -c1-120)"
  fi
  return 0
}

status_matches() {
  local expected="$1" actual="$2" e
  local -a parts
  IFS='|' read -ra parts <<< "$expected"
  for e in "${parts[@]}"; do
    case "$e" in
      2xx) [[ "$actual" =~ ^2[0-9]{2}$ ]] && return 0 ;;
      3xx) [[ "$actual" =~ ^3[0-9]{2}$ ]] && return 0 ;;
      4xx) [[ "$actual" =~ ^4[0-9]{2}$ ]] && return 0 ;;
      5xx) [[ "$actual" =~ ^5[0-9]{2}$ ]] && return 0 ;;
      *)   [[ "$actual" == "$e" ]] && return 0 ;;
    esac
  done
  return 1
}

assert_json() {
  local filter="$1"
  if ! jq -e . "$LAST_BODY" >/dev/null 2>&1; then
    LAST_DETAIL="response body is not valid JSON (filter: $filter)"
    return 1
  fi
  if ! jq -e "$filter" "$LAST_BODY" >/dev/null 2>&1; then
    LAST_DETAIL="jq assertion failed: $filter"
    return 1
  fi
  return 0
}

assert_header() {
  local name="$1" pattern="$2" value
  value="$(awk -v want="$(printf '%s' "$name" | tr 'A-Z' 'a-z')" '
    {
      line = $0
      sub(/\r$/, "", line)
      pos = index(line, ":")
      if (pos == 0) next
      key = tolower(substr(line, 1, pos - 1))
      if (key != want) next
      val = substr(line, pos + 1)
      sub(/^[ \t]+/, "", val)
      found = val
    }
    END { print found }
  ' "$LAST_HEADERS")"
  if [[ -z "$value" ]]; then
    LAST_DETAIL="missing response header: $name"
    return 1
  fi
  if [[ "$pattern" == "any" ]]; then return 0; fi
  if [[ "$pattern" == re:* ]]; then
    if printf '%s\n' "$value" | grep -Eqi -- "${pattern#re:}"; then return 0; fi
    LAST_DETAIL="header $name=value does not match ${pattern#re:}"
    return 1
  fi
  if [[ "$value" == "$pattern" ]]; then return 0; fi
  LAST_DETAIL="header $name has unexpected value (expected $pattern)"
  return 1
}

assert_body_contains() {
  local text="$1"
  if grep -Fq -- "$text" "$LAST_BODY" 2>/dev/null; then return 0; fi
  LAST_DETAIL="body does not contain expected text"
  return 1
}

assert_sse() {
  if ! grep -qi 'content-type: *text/event-stream' "$LAST_HEADERS" 2>/dev/null; then
    LAST_DETAIL="stream response is not text/event-stream"
    return 1
  fi
  if ! grep -q 'data:' "$LAST_BODY" 2>/dev/null; then
    LAST_DETAIL="stream response contains no data: frame"
    return 1
  fi
  return 0
}

record() {
  local group="$1" id="$2" status="$3" expect="$4" actual="$5" detail="$6"
  local safe feature kind
  safe="$(printf '%s' "$detail" \
    | sed -E 's/sk-(aria|bf)_[A-Za-z0-9_-]+/[REDACTED]/g; s/Bearer [A-Za-z0-9._-]+/Bearer [REDACTED]/g; s/"(token|secret|password)"[[:space:]]*:[[:space:]]*"[^"]*"/"\1":"[REDACTED]"/g' \
    | tr '\n' ' ')"
  feature="$E2E_TASK_ID"
  if [[ -z "$feature" ]]; then
    case "$id" in
      disabled|group_missing|group_crashed) feature="-" ;;
      *) feature="$(task_for_case "$group" "$id")" ;;
    esac
  fi
  kind="$E2E_TASK_KIND"
  if [[ -z "$kind" ]]; then
    if [[ "$feature" == "-" ]]; then kind="-"; else kind="$(task_kind_for "$feature")"; fi
  fi
  jq -nc \
    --arg group "$group" --arg id "$id" --arg status "$status" \
    --arg expect "$expect" --arg actual "$actual" --arg detail "$safe" \
    --arg feature "$feature" --arg kind "$kind" \
    '{group:$group,id:$id,status:$status,expect:$expect,actual:$actual,detail:($detail[0:200]),feature:$feature,kind:$kind}' \
    >> "$RESULTS_FILE" 2>/dev/null
  case "$status" in
    PASS)
      PASS=$((PASS + 1))
      printf 'OK:   %-16s %-36s %s\n' "$group" "$id" "$actual"
      ;;
    FAIL)
      FAIL=$((FAIL + 1))
      printf 'FAIL: %-16s %-36s expect=%s actual=%s | %s\n' \
        "$group" "$id" "$expect" "$actual" "$safe" >&2
      ;;
    SKIP)
      SKIP=$((SKIP + 1))
      printf 'SKIP: %-16s %-36s %s\n' "$group" "$id" "$safe"
      ;;
  esac
  return 0
}

skip_case() {
  record "$1" "$2" SKIP "-" "-" "$3"
}

check() {
  local group="$1" id="$2" expect="$3" jqf="$4"
  shift 4
  if ! status_matches "$expect" "$LAST_STATUS"; then
    record "$group" "$id" FAIL "status $expect" "$LAST_STATUS" "${LAST_DETAIL:-status mismatch}"
    return 1
  fi
  if [[ "$jqf" != "-" ]] && ! assert_json "$jqf"; then
    record "$group" "$id" FAIL "status $expect + json" "$LAST_STATUS" "$LAST_DETAIL"
    return 1
  fi
  local spec hp name pattern
  for spec in "$@"; do
    case "$spec" in
      jq:*)
        if ! assert_json "${spec#jq:}"; then
          record "$group" "$id" FAIL "status $expect + json" "$LAST_STATUS" "$LAST_DETAIL"
          return 1
        fi
        ;;
      hdr:*)
        hp="${spec#hdr:}"
        [[ "$hp" == *=* ]] || {
          record "$group" "$id" FAIL "valid hdr spec" "$LAST_STATUS" "malformed assertion: $spec"
          return 1
        }
        name="${hp%%=*}"
        pattern="${hp#*=}"
        if ! assert_header "$name" "$pattern"; then
          record "$group" "$id" FAIL "status $expect + header" "$LAST_STATUS" "$LAST_DETAIL"
          return 1
        fi
        ;;
      body:*)
        if ! assert_body_contains "${spec#body:}"; then
          record "$group" "$id" FAIL "status $expect + body" "$LAST_STATUS" "$LAST_DETAIL"
          return 1
        fi
        ;;
      sse)
        if ! assert_sse; then
          record "$group" "$id" FAIL "status $expect + sse" "$LAST_STATUS" "$LAST_DETAIL"
          return 1
        fi
        ;;
      *)
        record "$group" "$id" FAIL "valid assertion spec" "$LAST_STATUS" "unknown assertion: $spec"
        return 1
        ;;
    esac
  done
  record "$group" "$id" PASS "status $expect" "$LAST_STATUS" "assertions ok"
  return 0
}

check_local() {
  local group="$1" id="$2" expect_rc="$3" pattern="$4"
  shift 4
  local out rc=0
  out="$("$@" 2>&1)" || rc=$?
  if { [[ "$expect_rc" == "any" ]] || [[ "$rc" == "$expect_rc" ]]; } \
     && printf '%s' "$out" | grep -Eq -- "$pattern"; then
    record "$group" "$id" PASS "rc=$expect_rc" "rc=$rc" "output matched"
  else
    record "$group" "$id" FAIL "rc=$expect_rc pattern=$pattern" "rc=$rc" "output did not match expected pattern"
  fi
}

check_file_exists() {
  if [[ -f "$3" ]]; then
    record "$1" "$2" PASS "file exists" "found" "$3"
  else
    record "$1" "$2" FAIL "file exists" "missing" "$3"
  fi
}

check_file_jq() {
  local group="$1" id="$2" file="$3" filter="$4"
  if [[ ! -f "$file" ]]; then skip_case "$group" "$id" "missing file: $file"; return; fi
  if jq -e "$filter" "$file" >/dev/null 2>&1; then
    record "$group" "$id" PASS "json contract" "ok" "$filter"
  else
    record "$group" "$id" FAIL "json contract" "mismatch" "$filter"
  fi
}

check_file_grep() {
  local group="$1" id="$2" file="$3" pattern="$4"
  if [[ ! -f "$file" ]]; then skip_case "$group" "$id" "missing file: $file"; return; fi
  if grep -Fq -- "$pattern" "$file"; then
    record "$group" "$id" PASS "text contract" "ok" "$pattern"
  else
    record "$group" "$id" FAIL "text contract" "mismatch" "$pattern"
  fi
}

register_cleanup() {
  CLEANUP_KIND+=("$1")
  CLEANUP_ARG+=("$2")
}

cleanup_resources() {
  (( ${#CLEANUP_KIND[@]} > 0 )) || return 0
  local i kind arg body rc
  for (( i = ${#CLEANUP_KIND[@]} - 1; i >= 0; i-- )); do
    kind="${CLEANUP_KIND[i]}"
    arg="${CLEANUP_ARG[i]}"
    case "$kind" in
      delete_key)
        http_req DELETE "$MGMT_URL/v1/router/keys/$arg" - "${SESSION_ARGS[@]}"
        if [[ "$LAST_STATUS" == 200 ]]; then
          record cleanup "revoke_$arg" PASS "status 200" "$LAST_STATUS" "test key revoked"
        else
          record cleanup "revoke_$arg" FAIL "status 200" "$LAST_STATUS" "could not revoke test key"
        fi
        ;;
      restore_allow_register)
        body="{\"allow_register\":$arg}"
        http_req PUT "$MGMT_URL/v1/router/settings/allow_register" "$body" "${SESSION_ARGS[@]}"
        if [[ "$LAST_STATUS" == 200 ]]; then
          record cleanup restore_allow_register PASS "status 200" "$LAST_STATUS" "setting restored"
        else
          record cleanup restore_allow_register FAIL "status 200" "$LAST_STATUS" "setting NOT restored"
        fi
        ;;
      restore_email)
        body="$(jq -nc --arg email "$arg" '{email:$email}')"
        http_req PUT "$MGMT_URL/v1/router/auth/email" "$body" "${SESSION_ARGS[@]}"
        if [[ "$LAST_STATUS" == 200 ]]; then
          record cleanup restore_email PASS "status 200" "$LAST_STATUS" "email restored"
        else
          record cleanup restore_email FAIL "status 200" "$LAST_STATUS" "email NOT restored"
        fi
        ;;
      restore_config)
        if [[ -f "$arg" ]]; then
          E2E_CT="application/yaml"
          http_req PUT "$MGMT_URL/v1/router/config" "$(cat "$arg")"
          if [[ "$LAST_STATUS" == 200 ]]; then
            record cleanup restore_config PASS "status 200" "$LAST_STATUS" "config restored"
          else
            record cleanup restore_config FAIL "status 200" "$LAST_STATUS" "config NOT restored"
          fi
        fi
        ;;
      *)
        printf 'WARN: unknown cleanup kind %s\n' "$kind" >&2
        ;;
    esac
  done
  CLEANUP_KIND=()
  CLEANUP_ARG=()
  return 0
}

cleanup_on_exit() {
  local rc=$?
  if declare -F fixture_cleanup_all >/dev/null 2>&1; then
    fixture_cleanup_all
  fi
  if [[ -n "$AUDIT_TMP_DIR" && -d "$ROOT/$AUDIT_TMP_DIR" ]]; then
    rm -rf "$ROOT/$AUDIT_TMP_DIR"
  fi
  if [[ -n "$WORK_DIR" && -d "$WORK_DIR" ]]; then
    cleanup_resources
    rm -rf "$WORK_DIR"
  fi
  exit "$rc"
}

probe_liveness() {
  http_req GET "$MGMT_URL/health" -
  if [[ "$LAST_STATUS" != 200 ]] || ! jq -e '.status == "ok"' "$LAST_BODY" >/dev/null 2>&1; then
    printf 'error: management plane not healthy at %s/health (HTTP %s)\n' "$MGMT_URL" "$LAST_STATUS" >&2
    printf '  start the router yourself first, e.g.:\n' >&2
    printf '  aria-router serve --config config/examples/triad-gateway.yaml --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090\n' >&2
    return 1
  fi
  http_req GET "$DATA_URL/v1/models" -
  if [[ "$LAST_STATUS" == "000" ]]; then
    printf 'error: data plane not reachable at %s/v1/models\n' "$DATA_URL" >&2
    return 1
  fi
  return 0
}

prepare_session_args() {
  if [[ -n "$SESSION_TOKEN" ]]; then
    SESSION_ARGS=(-H "X-Aria-E2E: 1" -H "Authorization: Bearer $SESSION_TOKEN")
  fi
  if [[ -n "$ROUTER_API_KEY" ]]; then
    DATA_KEY_ARGS=(-H "X-Aria-E2E: 1" -H "Authorization: Bearer $ROUTER_API_KEY")
  fi
}

list_cases() {
  printf '%-16s %-38s %-7s %s\n' GROUP CASE TASK DESCRIPTION
  case_catalog | while IFS=$'\t' read -r g i t d; do
    printf '%-16s %-38s %-7s %s\n' "$g" "$i" "$t" "$d"
  done
}

run_self_test() {
  local failures=0
  local tmp
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/aria-router-selftest.XXXXXX" 2>/dev/null)" || {
    printf 'error: cannot create temporary directory\n' >&2
    exit 2
  }
  RESULTS_FILE="$tmp/cases.jsonl"
  LAST_BODY="$tmp/body"
  LAST_HEADERS="$tmp/headers"
  CURL_ERR="$tmp/err"
  : > "$RESULTS_FILE"

  st_expect() {
    local label="$1" rc="$2"
    if (( rc == 0 )); then
      printf 'OK:   self-test %s\n' "$label"
    else
      printf 'FAIL: self-test %s\n' "$label" >&2
      failures=$((failures + 1))
    fi
  }

  status_matches 200 200; st_expect "status exact" $?
  status_matches '200|404' 404; st_expect "status alternation" $?
  status_matches 2xx 201; st_expect "status class" $?
  status_matches 200 401; [[ $? -ne 0 ]]; st_expect "status negative" $?

  printf '{"a":1}' > "$LAST_BODY"
  assert_json '.a == 1'; st_expect "json positive" $?
  assert_json '.a == 2'; [[ $? -ne 0 ]]; st_expect "json negative" $?
  printf 'not-json' > "$LAST_BODY"
  assert_json '.a == 1'; [[ $? -ne 0 ]]; st_expect "json invalid body" $?

  printf 'HTTP/1.1 200 OK\r\nx-aria-router-layer: semantic\r\ncontent-type: application/json\r\n\r\n' > "$LAST_HEADERS"
  assert_header x-aria-router-layer semantic; st_expect "header exact" $?
  assert_header x-aria-router-layer 're:^semantic$'; st_expect "header regex" $?
  assert_header x-aria-router-layer any; st_expect "header presence" $?
  assert_header x-aria-router-layer bypass; [[ $? -ne 0 ]]; st_expect "header mismatch" $?
  assert_header x-missing any; [[ $? -ne 0 ]]; st_expect "header missing" $?

  printf 'HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n' > "$LAST_HEADERS"
  printf 'data: {"ok":true}\n\n' > "$LAST_BODY"
  assert_body_contains 'data:'; st_expect "body contains" $?
  assert_sse; st_expect "sse headers+data" $?

  printf 'HTTP/1.1 200 OK\r\nx-aria-router-layer: semantic\r\ncontent-type: application/json\r\n\r\n' > "$LAST_HEADERS"
  PASS=0; FAIL=0; SKIP=0
  printf '{"ok":true}' > "$LAST_BODY"; LAST_STATUS=200; LAST_DETAIL=""
  check selftest check_pass 200 '.ok == true' 'hdr:x-aria-router-layer=any' >/dev/null 2>&1
  [[ $PASS -eq 1 && $FAIL -eq 0 ]]; st_expect "check positive" $?
  printf '{"ok":false}' > "$LAST_BODY"; LAST_STATUS=401; LAST_DETAIL=""
  check selftest check_fail 200 '.ok == true' >/dev/null 2>&1
  [[ $FAIL -eq 1 ]]; st_expect "check negative" $?
  skip_case selftest skip_case 'reason'
  [[ $SKIP -eq 1 ]]; st_expect "skip accounting" $?

  record selftest redaction PASS "-" "-" 'leak sk-aria_ABC123 Bearer abc.def'
  if grep -q 'sk-aria_ABC123' "$RESULTS_FILE" || grep -q 'Bearer abc.def' "$RESULTS_FILE"; then
    st_expect "secret redaction" 1
  else
    st_expect "secret redaction" 0
  fi

  (( ${#TASK_IDS[@]} > 0 )); st_expect "task map loaded" $?
  [[ "$(task_for_case mgmt-read trust)" != "-" ]]; st_expect "task_for_case lookup" $?
  set_task T90 http
  record selftest tagged PASS "-" "-" "tagged case"
  grep -q '"feature":"T90"' "$RESULTS_FILE"; st_expect "record feature tag" $?
  clear_task
  record task-fixtures disabled SKIP "-" "-" "pass --with-fixtures to enable this group" >/dev/null
  jq -se 'any(.[]; .id=="disabled" and .feature=="-" and .kind=="-")' "$RESULTS_FILE" >/dev/null 2>&1
  st_expect "framework skip is untagged" $?
  WORK_DIR="$tmp"
  build_coverage >/dev/null; st_expect "coverage matrix build" $?
  jq -se 'any(.[]; .task=="T90" and .pass >= 1)' "$tmp/coverage.jsonl" >/dev/null 2>&1
  st_expect "coverage T90 counted" $?
  jq -se 'any(.[]; .task=="T90" and has("kinds"))' "$tmp/coverage.jsonl" >/dev/null 2>&1
  st_expect "coverage kinds recorded" $?
  jq -ne --arg pat "$T9X_PATTERN" --arg t "T90" '($t|test($pat))' >/dev/null 2>&1
  st_expect "coverage pattern keeps T90" $?
  jq -ne --arg pat "$T9X_PATTERN" --arg t "T9" '($t|test($pat))|not' >/dev/null 2>&1
  st_expect "coverage pattern excludes T9" $?
  WORK_DIR=""

  bash "$0" --not-a-real-flag >/dev/null 2>&1
  [[ $? -eq 2 ]]; st_expect "invalid argument exit 2" $?
  bash "$0" --task-focus T30 --list >/dev/null 2>&1
  [[ $? -eq 2 ]]; st_expect "unmapped task number exit 2" $?

  RESULTS_FILE=""
  rm -rf "$tmp"
  if (( failures > 0 )); then
    printf 'FAIL: self-test had %d failing check(s)\n' "$failures" >&2
    exit 1
  fi
  printf 'OK: self-test all helpers passed\n'
  exit 0
}

write_report() {
  mkdir -p "$OUT_DIR" 2>/dev/null || {
    printf 'error: cannot create report directory %s\n' "$OUT_DIR" >&2
    return 1
  }
  local cases duration ro coverage modes wf wl
  cases="$(jq -s '.' "$RESULTS_FILE" 2>/dev/null)" || cases='[]'
  [[ -n "$cases" ]] || cases='[]'
  duration=$(( (SECONDS - START_EPOCH) * 1000 ))
  ro="false"; (( READ_ONLY )) && ro="true"
  wf="false"; (( WITH_FIXTURES )) && wf="true"
  wl="false"; (( WITH_LOCAL )) && wl="true"
  coverage="$(build_coverage)"
  [[ -n "$coverage" ]] || coverage='[]'
  modes="$(jq -nc --argjson f "$wf" --argjson l "$wl" '{with_fixtures:$f,with_local:$l}')"
  local json="$OUT_DIR/user-e2e-report.json"
  local text="$OUT_DIR/user-e2e-report.txt"
  if ! jq -n \
      --argjson cases "$cases" --argjson coverage "$coverage" --argjson modes "$modes" \
      --arg data_url "$DATA_URL" --arg mgmt_url "$MGMT_URL" \
      --arg started_at "$STARTED_AT" --argjson duration_ms "$duration" \
      --argjson read_only "$ro" --argjson pass "$PASS" \
      --argjson fail "$FAIL" --argjson skip "$SKIP" \
      '{
        tool: "aria-router-user-e2e",
        data_url: $data_url, mgmt_url: $mgmt_url,
        started_at: $started_at, duration_ms: $duration_ms, read_only: $read_only,
        modes: $modes,
        summary: {pass: $pass, fail: $fail, skip: $skip, total: ($pass + $fail + $skip)},
        coverage: $coverage,
        cases: $cases
      }' > "$json" 2>/dev/null; then
    printf 'error: cannot write JSON report\n' >&2
    return 1
  fi
  {
    printf 'aria-router user E2E — %s\n' "$STARTED_AT"
    printf 'data=%s mgmt=%s read_only=%s allow_mutating=%s with_fixtures=%s with_local=%s\n' \
      "$DATA_URL" "$MGMT_URL" "$ro" \
      "$([[ $ALLOW_MUTATING -eq 1 ]] && printf true || printf false)" "$wf" "$wl"
    printf 'PASS=%d FAIL=%d SKIP=%d TOTAL=%d duration_ms=%d\n\n' \
      "$PASS" "$FAIL" "$SKIP" "$((PASS + FAIL + SKIP))" "$duration"
    printf '=== task coverage (T90-T99 first; KINDS = verification types actually run) ===\n'
    jq -r --arg pat "$T9X_PATTERN" '.coverage[] | select(.task|test($pat)) | [.task,.status,(.kinds // .kind),(.pass|tostring),(.fail|tostring),(.skip|tostring),(.cases|tostring),.stage] | @tsv' "$json" 2>/dev/null \
      | while IFS=$'\t' read -r t s k p f sk n st; do
          [[ -z "$t" ]] && continue
          printf '%-6s %-10s %-18s P=%s F=%s S=%s N=%s %s\n' "$t" "$s" "$k" "$p" "$f" "$sk" "$n" "$st"
        done
    jq -r --arg pat "$T9X_PATTERN" '.coverage[] | select((.task|test($pat))|not) | [.task,.status,(.kinds // .kind),(.pass|tostring),(.fail|tostring),(.skip|tostring),(.cases|tostring),.stage] | @tsv' "$json" 2>/dev/null \
      | while IFS=$'\t' read -r t s k p f sk n st; do
          [[ -z "$t" ]] && continue
          printf '%-6s %-10s %-18s P=%s F=%s S=%s N=%s %s\n' "$t" "$s" "$k" "$p" "$f" "$sk" "$n" "$st"
        done
    printf '\n=== cases ===\n'
    jq -r '.cases[] | [.status, .group, .id, .feature, .kind, .actual, .detail] | @tsv' "$json" 2>/dev/null \
      | while IFS=$'\t' read -r s g i t k a d; do
          printf '%-6s %-16s %-38s %-6s %-8s %-8s %s\n' "$s" "$g" "$i" "$t" "$k" "$a" "$d"
        done
  } > "$text"
  printf '\n=== summary: %d PASS / %d FAIL / %d SKIP (total %d) ===\n' \
    "$PASS" "$FAIL" "$SKIP" "$((PASS + FAIL + SKIP))"
  printf 'OK: report %s\n' "$json"
  printf 'OK: report %s\n' "$text"
  return 0
}

run_groups() {
  local group fn
  for group in "${ALL_GROUPS[@]}"; do
    group_enabled "$group" || continue
    clear_task
    if [[ "$group" == "local-optional" || "$group" == "bench-optional" || "$group" == "task-offline" ]] \
       && (( WITH_LOCAL == 0 )); then
      skip_case "$group" disabled "pass --with-local to enable this group"
      continue
    fi
    if [[ "$group" == "task-fixtures" ]] && (( WITH_FIXTURES == 0 )); then
      skip_case "$group" disabled "pass --with-fixtures to enable this group"
      continue
    fi
    printf '\n=== %s ===\n' "$group"
    fn="run_group_${group//-/_}"
    if ! declare -F "$fn" >/dev/null 2>&1; then
      record "$group" group_missing FAIL "case group implemented" "missing" "$fn"
      continue
    fi
    "$fn" || record "$group" group_crashed FAIL "case group completes" "error" "group returned non-zero"
  done
}

final_exit() {
  if (( FAIL > 0 )); then
    printf 'FAIL: %d case(s) failed\n' "$FAIL" >&2
    exit 1
  fi
  if (( STRICT_SKIP )) && (( SKIP > 0 )); then
    printf 'FAIL: %d case(s) skipped with --strict-skip\n' "$SKIP" >&2
    exit 1
  fi
  printf 'OK: %d case(s) passed, %d skipped\n' "$PASS" "$SKIP"
  exit 0
}

main() {
  parse_args "$@"
  need_cmd curl
  need_cmd jq
  need_cmd mktemp
  load_task_map
  validate_task_focus
  [[ -f "$CASES_LIB" ]] || die_usage "missing case library: $CASES_LIB"
  # shellcheck source=/dev/null
  source "$CASES_LIB"
  if (( WITH_FIXTURES )); then
    [[ -f "$FIXTURES_LIB" ]] || die_usage "missing fixture library: $FIXTURES_LIB"
    # shellcheck source=/dev/null
    source "$FIXTURES_LIB"
  fi
  if (( LIST )); then list_cases; exit 0; fi
  if (( SELF_TEST )); then run_self_test; fi
  setup_workspace
  trap cleanup_on_exit EXIT
  if groups_need_service; then
    probe_liveness || exit 2
  fi
  prepare_session_args
  run_groups
  cleanup_resources
  write_report || exit 2
  final_exit
}

main "$@"
