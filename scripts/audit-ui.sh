#!/usr/bin/env bash
# impeccable-style UI audit for the aria-router dashboard (npm / vite, no bun).
# Modes: --static | --dom | --url | --browser | --list-rules
#
# A mode that cannot actually run is recorded as "skipped" — never as a pass.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DASH="$ROOT/dashboard"
CONFIG="$DASH/.impeccable/config.json"

IMPECCABLE_VERSION="${IMPECCABLE_VERSION:-4.8.0}"
IMPECCABLE_BIN="${IMPECCABLE_BIN:-}"
IMPECCABLE_BROWSER="${IMPECCABLE_BROWSER:-}"
PREVIEW_PORT="${PREVIEW_PORT:-4173}"
AUDIT_BASE_URL="${AUDIT_BASE_URL:-}"
AUDIT_ROUTES="${AUDIT_ROUTES:-}"
AUDIT_THEMES="${AUDIT_THEMES:-light,dark}"
AUDIT_OUT_DIR="${AUDIT_OUT_DIR:-detect-out}"
AUDIT_PAGE_TIMEOUT="${AUDIT_PAGE_TIMEOUT:-15000}"

OUT="$ROOT/$AUDIT_OUT_DIR"
MODE=""
RULES_PRIMARY=(color/aa-contrast type/min-font-size shape/no-gradient shape/square-corners motion/transform-opacity-only assets/no-web-font-cdn a11y/lang-attribute)
RULES_SECONDARY=(type/no-uppercase type/no-letter-spacing type/local-font-stack)

log() { printf '[audit-ui] %s\n' "$*"; }
fail() { printf '[audit-ui] ERROR: %s\n' "$*" >&2; exit 2; }

usage() {
  cat <<'EOF'
usage: ./scripts/audit-ui.sh [--static|--dom|--url|--browser|--list-rules]

env: IMPECCABLE_VERSION (4.8.0), IMPECCABLE_BIN, IMPECCABLE_BROWSER,
     PREVIEW_PORT (4173), AUDIT_BASE_URL, AUDIT_ROUTES, AUDIT_THEMES,
     AUDIT_OUT_DIR (detect-out), AUDIT_PAGE_TIMEOUT (15000)
EOF
}

for arg in "$@"; do
  case "$arg" in
    --static|--dom|--url|--browser|--list-rules)
      [ -z "$MODE" ] || fail "only one mode may be selected"
      MODE="${arg#--}"
      ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; fail "unknown argument: $arg" ;;
  esac
done
[ -n "$MODE" ] || { usage >&2; fail "no mode selected"; }

mkdir -p "$OUT"
export AUDIT_ROUTES AUDIT_THEMES AUDIT_PAGE_TIMEOUT AUDIT_OUT_DIR AUDIT_BASE_URL
export NODE_OPTIONS="${NODE_OPTIONS:-}"

node_bin() {
  if [ -x "$DASH/node_modules/.bin/$1" ]; then
    printf '%s\n' "$DASH/node_modules/.bin/$1"
  elif command -v "$1" >/dev/null 2>&1; then
    command -v "$1"
  else
    return 1
  fi
}

list_rules() {
  log "impeccable ${IMPECCABLE_VERSION} rule set (from $CONFIG)"
  for r in "${RULES_PRIMARY[@]}"; do printf 'primary   %s\n' "$r"; done
  for r in "${RULES_SECONDARY[@]}"; do printf 'secondary %s\n' "$r"; done
  if [ -f "$CONFIG" ] && command -v python3 >/dev/null 2>&1; then
    python3 - "$CONFIG" <<'PY' || true
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    cfg = json.load(f)
print("routes: " + ", ".join(cfg.get("dom", {}).get("routes", [])))
print("themes: " + ", ".join(cfg.get("dom", {}).get("themes", [])))
PY
  fi
}

impeccable_cmd() {
  if [ -n "$IMPECCABLE_BIN" ] && [ -x "$IMPECCABLE_BIN" ]; then
    printf '%s\n' "$IMPECCABLE_BIN"
  elif [ -x "$DASH/node_modules/.bin/impeccable" ]; then
    printf '%s\n' "$DASH/node_modules/.bin/impeccable"
  else
    return 1
  fi
}

find_browser() {
  if [ -n "$IMPECCABLE_BROWSER" ]; then
    if command -v "$IMPECCABLE_BROWSER" >/dev/null 2>&1 || [ -x "$IMPECCABLE_BROWSER" ]; then
      printf '%s\n' "$IMPECCABLE_BROWSER"
      return 0
    fi
    return 1
  fi
  for b in google-chrome google-chrome-stable chromium chromium-browser chrome; do
    if command -v "$b" >/dev/null 2>&1; then command -v "$b"; return 0; fi
  done
  return 1
}

write_status() {
  local mode="$1" status="$2" reason="$3"
  if command -v python3 >/dev/null 2>&1; then
    MODE_NAME="$mode" STATUS="$status" REASON="$reason" OUT_FILE="$OUT/${mode}-status.json" \
      python3 - <<'PY'
import json, os
with open(os.environ["OUT_FILE"], "w", encoding="utf-8") as f:
    json.dump({"mode": os.environ["MODE_NAME"], "status": os.environ["STATUS"],
               "reason": os.environ["REASON"]}, f, indent=2)
    f.write("\n")
PY
  fi
  log "$mode: $status ($reason)"
}

port_free() {
  python3 - "$1" <<'PY'
import socket, sys
s = socket.socket()
try:
    s.bind(("127.0.0.1", int(sys.argv[1])))
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
}

run_vitest() {
  local spec="$1"
  local vitest
  vitest="$(node_bin vitest)" || fail "vitest not installed; run npm --prefix dashboard ci"
  log "running $spec"
  (
    cd "$DASH"
    if command -v timeout >/dev/null 2>&1; then
      timeout "$((AUDIT_PAGE_TIMEOUT / 1000 + 120))s" "$vitest" run "$spec" --reporter=dot
    else
      "$vitest" run "$spec" --reporter=dot
    fi
  )
}

case "$MODE" in
  list-rules)
    list_rules
    exit 0
    ;;
  static)
    log "static primary rules: ${RULES_PRIMARY[*]}"
    if run_vitest "tests/style.spec.ts"; then
      write_status static passed "0 primary findings"
      exit 0
    fi
    write_status static failed "primary findings present"
    exit 2
    ;;
  dom)
    log "dom render: all routes x ${AUDIT_THEMES} (jsdom, browser not required)"
    if run_vitest "tests/render-snapshot.spec.tsx"; then
      write_status dom passed "jsdom render snapshots generated"
      exit 0
    fi
    write_status dom failed "render snapshot guard failed"
    exit 2
    ;;
  url|browser)
    browser="$(find_browser || true)"
    if [ "$MODE" = "browser" ] && [ -z "$browser" ]; then
      printf '[audit-ui] ERROR: no Chrome/Chromium found. Install one or set IMPECCABLE_BROWSER=/path/to/chrome\n' >&2
      exit 1
    fi
    if [ -z "$browser" ]; then
      write_status url skipped "no Chrome/Chromium; not a pass"
      exit 0
    fi
    if [ -z "$AUDIT_BASE_URL" ]; then
      if ! port_free "$PREVIEW_PORT"; then
        fail "PREVIEW_PORT $PREVIEW_PORT is occupied; refusing to reuse a server we did not start"
      fi
    fi
    cmd="$(impeccable_cmd || true)"
    if [ -z "$cmd" ]; then
      write_status url skipped "impeccable ${IMPECCABLE_VERSION} binary unavailable (set IMPECCABLE_BIN); browser found but not a pass"
      exit 0
    fi
    base="${AUDIT_BASE_URL:-http://127.0.0.1:$PREVIEW_PORT}"
    log "browser=$browser base=$base routes=${AUDIT_ROUTES:-all} timeout=${AUDIT_PAGE_TIMEOUT}ms"
    if "$cmd" audit --base-url "$base" --browser "$browser" --out "$OUT" --timeout "$AUDIT_PAGE_TIMEOUT"; then
      write_status url passed "browser rendering audit completed"
      exit 0
    fi
    write_status url failed "browser rendering audit reported findings"
    exit 2
    ;;
esac
