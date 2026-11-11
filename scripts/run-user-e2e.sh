#!/usr/bin/env bash
# User-facing black-box E2E checks for a running aria-router instance.
#
# Prerequisites:
#   - aria-router already serving on both planes (data + management). This script
#     NEVER builds, installs, starts, stops or upgrades a router process.
#   - curl, jq and standard coreutils. python3 is only needed by bench-optional.
#
# Usage:
#   ./scripts/run-user-e2e.sh [options]
#   ROUTER_URL=http://127.0.0.1:8899 MGMT_URL=http://127.0.0.1:8090 \
#     ./scripts/run-user-e2e.sh
#   ./scripts/run-user-e2e.sh --list
#   ./scripts/run-user-e2e.sh --self-test
#   ./scripts/run-user-e2e.sh --read-only --group cli,data,decisioners
#   ./scripts/run-user-e2e.sh --with-local --strict-skip
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
#   --with-local           enable local-optional / bench-optional groups
#   --strict-skip          exit 1 when any case is skipped
#   --list                 list the case catalog and exit
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

DATA_URL="${ROUTER_URL:-http://127.0.0.1:8899}"
MGMT_URL="${MGMT_URL:-http://127.0.0.1:8090}"
ADMIN_USER="${ADMIN_USER:-}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"
ROUTER_API_KEY="${ROUTER_API_KEY:-}"
SESSION_TOKEN="${ROUTER_SESSION_TOKEN:-}"
TIMEOUT="${TIMEOUT:-15}"
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
STRICT_SKIP=0
LIST=0
SELF_TEST=0
GROUP_FILTER=""
SKIP_FILTER=""

ALL_GROUPS=(cli auth keys data decisioners mgmt-read mgmt-write dashboard local-optional bench-optional)

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
      --strict-skip)    STRICT_SKIP=1; shift ;;
      --list)           LIST=1; shift ;;
      --self-test)      SELF_TEST=1; shift ;;
      -h|--help)        usage; exit 0 ;;
      *)                die_usage "unknown argument: $1" ;;
    esac
  done
  [[ "$TIMEOUT" =~ ^[0-9]+$ ]] || die_usage "--timeout must be an integer"
  [[ "$SSE_TIMEOUT" =~ ^[0-9]+$ ]] || die_usage "--sse-timeout must be an integer"
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
  local g="$1" item
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
  return 0
}

groups_need_service() {
  local g
  for g in "${ALL_GROUPS[@]}"; do
    group_enabled "$g" || continue
    case "$g" in
      cli|local-optional) : ;;
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
  local safe
  safe="$(printf '%s' "$detail" \
    | sed -E 's/sk-(aria|bf)_[A-Za-z0-9_-]+/[REDACTED]/g; s/Bearer [A-Za-z0-9._-]+/Bearer [REDACTED]/g; s/"(token|secret|password)"[[:space:]]*:[[:space:]]*"[^"]*"/"\1":"[REDACTED]"/g' \
    | tr '\n' ' ')"
  jq -nc \
    --arg group "$group" --arg id "$id" --arg status "$status" \
    --arg expect "$expect" --arg actual "$actual" --arg detail "$safe" \
    '{group:$group,id:$id,status:$status,expect:$expect,actual:$actual,detail:($detail[0:200])}' \
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
  printf '%-16s %-40s %s\n' GROUP CASE DESCRIPTION
  case_catalog | while IFS=$'\t' read -r g i d; do
    printf '%-16s %-40s %s\n' "$g" "$i" "$d"
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

  bash "$0" --not-a-real-flag >/dev/null 2>&1
  [[ $? -eq 2 ]]; st_expect "invalid argument exit 2" $?

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
  local cases duration ro
  cases="$(jq -s '.' "$RESULTS_FILE" 2>/dev/null)" || cases='[]'
  [[ -n "$cases" ]] || cases='[]'
  duration=$(( (SECONDS - START_EPOCH) * 1000 ))
  ro="false"
  (( READ_ONLY )) && ro="true"
  local json="$OUT_DIR/user-e2e-report.json"
  local text="$OUT_DIR/user-e2e-report.txt"
  if ! jq -n \
      --argjson cases "$cases" \
      --arg data_url "$DATA_URL" --arg mgmt_url "$MGMT_URL" \
      --arg started_at "$STARTED_AT" --argjson duration_ms "$duration" \
      --argjson read_only "$ro" --argjson pass "$PASS" \
      --argjson fail "$FAIL" --argjson skip "$SKIP" \
      '{
        tool: "aria-router-user-e2e",
        data_url: $data_url, mgmt_url: $mgmt_url,
        started_at: $started_at, duration_ms: $duration_ms, read_only: $read_only,
        summary: {pass: $pass, fail: $fail, skip: $skip, total: ($pass + $fail + $skip)},
        cases: $cases
      }' > "$json" 2>/dev/null; then
    printf 'error: cannot write JSON report\n' >&2
    return 1
  fi
  {
    printf 'aria-router user E2E — %s\n' "$STARTED_AT"
    printf 'data=%s mgmt=%s read_only=%s allow_mutating=%s\n' \
      "$DATA_URL" "$MGMT_URL" "$ro" "$([[ $ALLOW_MUTATING -eq 1 ]] && printf true || printf false)"
    printf 'PASS=%d FAIL=%d SKIP=%d TOTAL=%d duration_ms=%d\n\n' \
      "$PASS" "$FAIL" "$SKIP" "$((PASS + FAIL + SKIP))" "$duration"
    jq -r '.cases[] | [.status, .group, .id, .actual, .detail] | @tsv' "$json" 2>/dev/null \
      | while IFS=$'\t' read -r s g i a d; do printf '%-6s %-16s %-38s %-8s %s\n' "$s" "$g" "$i" "$a" "$d"; done
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
    if [[ "$group" == "local-optional" || "$group" == "bench-optional" ]] && (( WITH_LOCAL == 0 )); then
      skip_case "$group" disabled "pass --with-local to enable this group"
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
  [[ -f "$CASES_LIB" ]] || die_usage "missing case library: $CASES_LIB"
  # shellcheck source=/dev/null
  source "$CASES_LIB"
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
