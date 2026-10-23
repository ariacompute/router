#!/usr/bin/env bash
# Track E: routing-accuracy across semantic / agent / afm-d on :8899.
# Prerequisites:
#   - aria-router serving config/examples/triad-gateway.yaml (or --start-router)
#   - GATEWAY_API_KEY set if backends require it
#   - For afm-d: aria-engine at DECISION_MODEL_URL (default http://127.0.0.1:8010)
#
# Usage:
#   ./scripts/run-routing-accuracy.sh
#   ./scripts/run-routing-accuracy.sh --start-router
#   CORPUS=bench/corpus/routing_gateway_hard.json ./scripts/run-routing-accuracy.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ROUTER_URL="${ROUTER_URL:-http://127.0.0.1:8899}"
MGMT_URL="${MGMT_URL:-http://127.0.0.1:8090}"
DECISION_MODEL_URL="${DECISION_MODEL_URL:-http://127.0.0.1:8010}"
CORPUS="${CORPUS:-bench/corpus/routing_gateway_eval.json}"
OUT_DIR="${OUT_DIR:-./out}"
CONFIG="${CONFIG:-config/examples/triad-gateway.yaml}"
PYTHON="${PYTHON:-python3}"
TIMEOUT="${TIMEOUT:-120}"
START_ROUTER=0
SKIP_AFMD=0
ROUTER_PID=""

usage() {
  cat <<EOF
Usage: $0 [--start-router] [--skip-afmd] [--help]

Env:
  ROUTER_URL            default http://127.0.0.1:8899
  MGMT_URL              default http://127.0.0.1:8090 (liveness: GET /health)
  DECISION_MODEL_URL    default http://127.0.0.1:8010 (afm-d)
  CORPUS                default bench/corpus/routing_gateway_eval.json
  OUT_DIR               default ./out
  CONFIG                triad gateway YAML
  GATEWAY_API_KEY       TokenHub key for aria-router *process* (api_key_env);
                        do NOT send this as client Authorization to :8899
  ROUTER_API_KEY        optional client key for bench→router when require_api_key
  PYTHON                default python3
  TIMEOUT               per-request timeout seconds
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --start-router) START_ROUTER=1; shift ;;
    --skip-afmd) SKIP_AFMD=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

cleanup() {
  if [[ -n "$ROUTER_PID" ]] && kill -0 "$ROUTER_PID" 2>/dev/null; then
    kill "$ROUTER_PID" 2>/dev/null || true
    wait "$ROUTER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# Liveness only — do not require a successful chat (TokenHub/upstream may 5xx).
router_up() {
  if curl -sf -o /dev/null --max-time 2 "${MGMT_URL}/health" 2>/dev/null; then
    return 0
  fi
  # Fallback: any HTTP response from the data plane means the process is up.
  local code
  code="$(
    curl -s -o /dev/null -w '%{http_code}' --max-time 3 \
      -X POST "${ROUTER_URL}/v1/chat/completions" \
      -H 'content-type: application/json' \
      -d '{"model":"ariacompute/semantic-auto","messages":[{"role":"user","content":"ping"}],"max_tokens":1}' \
      2>/dev/null || true
  )"
  [[ -n "$code" && "$code" != "000" ]]
}

decision_up() {
  curl -sf -o /dev/null --max-time 2 "${DECISION_MODEL_URL}/health" 2>/dev/null \
    || curl -sf -o /dev/null --max-time 2 "${DECISION_MODEL_URL}/v1/health" 2>/dev/null \
    || curl -sf -o /dev/null --max-time 2 "${DECISION_MODEL_URL}/" 2>/dev/null \
    || return 1
}

if [[ "$START_ROUTER" -eq 1 ]]; then
  ARIA_ROUTER_BIN="${ARIA_ROUTER_BIN:-}"
  if [[ -z "$ARIA_ROUTER_BIN" ]]; then
    if command -v aria-router >/dev/null 2>&1; then
      ARIA_ROUTER_BIN="$(command -v aria-router)"
    elif [[ -x "$ROOT/target/release/aria-router" ]]; then
      ARIA_ROUTER_BIN="$ROOT/target/release/aria-router"
    else
      echo "error: aria-router not found (PATH or ./target/release/aria-router)" >&2
      exit 1
    fi
  fi
  export DECISION_MODEL_URL
  "$ARIA_ROUTER_BIN" serve --config "$CONFIG" --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090 &
  ROUTER_PID=$!
  echo "started aria-router pid=$ROUTER_PID bin=$ARIA_ROUTER_BIN config=$CONFIG"
  for _ in $(seq 1 60); do
    if router_up; then
      break
    fi
    sleep 0.5
  done
fi

if ! router_up; then
  echo "error: router not reachable (mgmt $MGMT_URL/health or data $ROUTER_URL)" >&2
  echo "  start with: ./target/release/aria-router serve --config $CONFIG --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090" >&2
  echo "  or re-run with --start-router" >&2
  echo "  tip: probe with: curl -sS $MGMT_URL/health" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

PICK_MAP=(
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash
  --pick-map ariacompute/ariamodel-mid=glm-5.3
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro
)

# triad-gateway has require_api_key: false. TokenHub auth is GATEWAY_API_KEY in the
# *serve* process only. Passing that same key as client Bearer makes aria-router
# return 401 invalid api key (it is not a router keystore key).
API_KEY_ARGS=()
if [[ -n "${ROUTER_API_KEY:-}" ]]; then
  API_KEY_ARGS=(--api-key "router=${ROUTER_API_KEY}")
elif [[ -n "${GATEWAY_API_KEY:-}" ]]; then
  echo "note: GATEWAY_API_KEY is set in this shell but will NOT be sent to $ROUTER_URL" >&2
  echo "      (TokenHub key belongs on the aria-router serve process; use ROUTER_API_KEY for client auth)" >&2
fi

run_one() {
  local name="$1"
  local entrypoint="$2"
  local report="$OUT_DIR/acc_${name}.json"
  echo "=== routing-accuracy: $name ($entrypoint) ==="
  "$PYTHON" -m bench routing-accuracy \
    --router "$ROUTER_URL" \
    --entrypoint "$entrypoint" \
    --pick-header x-aria-router-model \
    "${PICK_MAP[@]}" \
    "${API_KEY_ARGS[@]}" \
    --corpus "$CORPUS" \
    --timeout "$TIMEOUT" \
    --max-tokens 16 \
    --report "$report"
}

run_one semantic ariacompute/semantic-auto
run_one agent ariacompute/agent-auto

if [[ "$SKIP_AFMD" -eq 1 ]]; then
  echo "skip afm-d (--skip-afmd)"
else
  if ! decision_up; then
    echo "warn: decision model not reachable at $DECISION_MODEL_URL; afm-d may error" >&2
    echo "  start: aria-engine serve --track encoder --bind 127.0.0.1:8010" >&2
  fi
  run_one afmd ariacompute/afm-d-auto
fi

# Summary table
"$PYTHON" - <<'PY' "$OUT_DIR" "$SKIP_AFMD"
import json, sys
from pathlib import Path

out = Path(sys.argv[1])
skip_afmd = sys.argv[2] == "1"
names = ["semantic", "agent"] + ([] if skip_afmd else ["afmd"])
print()
print("=== Track E summary ===")
print(f"{'decisioner':<12} {'accuracy':>10} {'correct':>8} {'scored':>8} {'errors':>8}")
rows = []
for name in names:
    path = out / f"acc_{name}.json"
    if not path.exists():
        print(f"{name:<12} {'MISSING':>10}")
        continue
    rep = json.loads(path.read_text(encoding="utf-8"))
    s = rep.get("summary") or {}
    acc = float(s.get("overall_accuracy") or 0.0)
    print(
        f"{name:<12} {acc:>10.3f} {s.get('correct', 0):>8} "
        f"{s.get('scored', 0):>8} {s.get('errors', 0):>8}"
    )
    rows.append({"decisioner": name, **s})
summary_path = out / "acc_summary.json"
summary_path.write_text(
    json.dumps({"decisioners": rows}, indent=2) + "\n", encoding="utf-8"
)
print(f"\nwrote {summary_path}")
PY
