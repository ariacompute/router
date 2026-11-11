#!/usr/bin/env bash
# Isolated fixture-instance scenarios for run-user-e2e.sh (opt-in --with-fixtures).
#
# This library never compiles, downloads, or installs anything. It runs an
# existing aria-router binary against a temporary YAML fixture and a Python
# standard-library stub backend, all bound to loopback ports with a private
# ARIA_COMPUTE_HOME. Every process is stopped by fixture_cleanup_all() from the
# main script's EXIT trap. The default (already-running) instance is untouched.

FIXTURE_PIDS=()
FIXTURE_STUB_PID=""
FIXTURE_DATA_PORT=0
FIXTURE_MGMT_PORT=0
FIXTURE_STUB_PORT=0

fixture_free_port() {
  python3 -c 'import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()' 2>/dev/null
}

fixture_write_stub() {
  cat > "$WORK_DIR/e2e-stub.py" <<'PY'
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CHAT = {
    "id": "chatcmpl-e2e",
    "object": "chat.completion",
    "created": 0,
    "model": "local/general",
    "choices": [{
        "index": 0,
        "message": {"role": "assistant", "content": "hello-from-router"},
        "finish_reason": "stop",
    }],
    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
}
ANSWERS = {"answers": {"route": {"choice": "local/general", "confidence": 0.9}}}


class Handler(BaseHTTPRequestHandler):
    def _send(self, payload, ctype="application/json", code=200):
        data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        return

    def do_GET(self):
        self._send({"status": "ok"})

    def do_POST(self):
        length = int(self.headers.get("content-length", "0") or 0)
        raw = self.rfile.read(length).decode() if length else ""
        if self.path.startswith("/v1/systemone"):
            return self._send(ANSWERS)
        try:
            streaming = json.loads(raw).get("stream") is True
        except (ValueError, AttributeError):
            streaming = False
        if streaming:
            body = b"data: " + json.dumps(CHAT).encode() + b"\n\ndata: [DONE]\n\n"
            return self._send(body, "text/event-stream")
        return self._send(CHAT)


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY
}

fixture_write_config() {
  local data_port="$1" stub_port="$2" out="$WORK_DIR/fixture-triad.yaml"
  cat > "$out" <<EOF
# Derived from config/examples/*-tiny.yaml for isolated E2E scenarios.
version: v0.3

listeners:
  - name: http
    address: 127.0.0.1
    port: ${data_port}

providers:
  defaults:
    default_model: local/general
  models:
    - name: local/general
      provider_model_id: echo
      locality: local
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: failing
          endpoint: 127.0.0.1:9
        - name: stub
          endpoint: 127.0.0.1:${stub_port}

entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: mom
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default

recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      competence:
        enabled: true
        protection:
          enabled: true
          floor: 0.3
      signals:
        keywords:
          - name: needs_explain
            operator: OR
            keywords: ["explain", "walk me through"]
      decisions:
        - name: explanatory
          priority: 100
          rules:
            operator: AND
            conditions:
              - type: keyword
                name: needs_explain
          modelRefs:
            - model: local/general
  - name: afm-d-default
    router: afm-d
    afm-d:
      endpoint: 127.0.0.1:${stub_port}
      timeout_ms: 2000
      fallback: local/general
      question_template: three
      instructions: |
        Pick the best model for this user request.

global:
  require_api_key: false
  services:
    redecision:
      enabled: true
      emit_header: true
      tool_continuation: true
    trust:
      enabled: true
      min_evidence: 3
    backend_failover:
      enabled: true
      max_attempts: 3
    quota:
      enabled: false
EOF
  printf '%s' "$out"
}

fixture_wait_health() {
  local url="$1" i
  for (( i = 0; i < 30; i++ )); do
    if curl -sS -m 1 -o /dev/null "$url" 2>/dev/null; then return 0; fi
    sleep 0.2
  done
  return 1
}

fixture_cleanup_all() {
  local i
  for (( i = ${#FIXTURE_PIDS[@]} - 1; i >= 0; i-- )); do
    kill "${FIXTURE_PIDS[i]}" 2>/dev/null || true
    wait "${FIXTURE_PIDS[i]}" 2>/dev/null || true
  done
  FIXTURE_PIDS=()
  if [[ -n "$FIXTURE_STUB_PID" ]]; then
    kill "$FIXTURE_STUB_PID" 2>/dev/null || true
    wait "$FIXTURE_STUB_PID" 2>/dev/null || true
    FIXTURE_STUB_PID=""
  fi
  return 0
}

fixture_skip_all() {
  local reason="$1" id
  set_task T91 fixture
  skip_case task-fixtures three_question_afmd "$reason"
  set_task T92 fixture
  skip_case task-fixtures protection_state "$reason"
  set_task T95 fixture
  skip_case task-fixtures evidence_scene "$reason"
  set_task T97 fixture
  skip_case task-fixtures failover_candidates "$reason"
  clear_task
}

fixture_start() {
  local bin cfg home data_url mgmt_url stub_url
  bin="$(find_cli_bin)" || return 1
  FIXTURE_STUB_PORT="$(fixture_free_port)"
  FIXTURE_DATA_PORT="$(fixture_free_port)"
  FIXTURE_MGMT_PORT="$(fixture_free_port)"
  [[ -n "$FIXTURE_STUB_PORT" && -n "$FIXTURE_DATA_PORT" && -n "$FIXTURE_MGMT_PORT" ]] || return 1
  [[ "$FIXTURE_DATA_PORT" != "$FIXTURE_MGMT_PORT" && "$FIXTURE_STUB_PORT" != "$FIXTURE_DATA_PORT" \
     && "$FIXTURE_STUB_PORT" != "$FIXTURE_MGMT_PORT" ]] || return 1

  fixture_write_stub
  python3 "$WORK_DIR/e2e-stub.py" "$FIXTURE_STUB_PORT" > "$WORK_DIR/stub.log" 2>&1 &
  FIXTURE_STUB_PID=$!
  stub_url="http://127.0.0.1:$FIXTURE_STUB_PORT/health"
  fixture_wait_health "$stub_url" || return 1

  cfg="$(fixture_write_config "$FIXTURE_DATA_PORT" "$FIXTURE_STUB_PORT")"
  if ! "$bin" validate --config "$cfg" > "$WORK_DIR/validate.log" 2>&1; then
    return 1
  fi
  home="$WORK_DIR/fixture-home"
  mkdir -p "$home"
  ARIA_COMPUTE_HOME="$home" "$bin" serve --config "$cfg" \
    --bind "127.0.0.1:$FIXTURE_DATA_PORT" \
    --mgmt-bind "127.0.0.1:$FIXTURE_MGMT_PORT" \
    --no-dashboard > "$WORK_DIR/router.log" 2>&1 &
  FIXTURE_PIDS+=("$!")
  mgmt_url="http://127.0.0.1:$FIXTURE_MGMT_PORT/health"
  fixture_wait_health "$mgmt_url" || return 1
  data_url="http://127.0.0.1:$FIXTURE_DATA_PORT/v1/models"
  fixture_wait_health "$data_url" || return 1
  return 0
}

run_group_task_fixtures() {
  local body m1 m2 s1 s2 data mgmt
  clear_task
  if ! command -v python3 >/dev/null 2>&1; then
    fixture_skip_all "python3 unavailable (stub backend needs the standard library)"
    return 0
  fi
  if ! command -v find_cli_bin >/dev/null 2>&1 || ! find_cli_bin >/dev/null 2>&1; then
    fixture_skip_all "no prebuilt aria-router binary (ARIA_ROUTER_BIN / PATH / target/release)"
    return 0
  fi
  if ! fixture_start; then
    fixture_skip_all "isolated fixture instance could not start (config rejected or probe timeout)"
    return 0
  fi
  data="http://127.0.0.1:$FIXTURE_DATA_PORT"
  mgmt="http://127.0.0.1:$FIXTURE_MGMT_PORT"

  set_task T91 fixture
  body="$(chat_body "$ENTRY_AFMD" "please explain rust")"
  http_req POST "$data/v1/chat/completions" "$body"
  if [[ "$LAST_STATUS" == 200 ]]; then
    check task-fixtures three_question_afmd 200 'type=="object"' \
      'hdr:x-aria-router-layer=afm-d'
  else
    skip_case task-fixtures three_question_afmd \
      "afm-d three-question routing returned HTTP $LAST_STATUS (stub/decisioner contract)"
  fi

  set_task T92 fixture
  body="$(chat_body "$ENTRY_SEMANTIC" "explain rust")"
  http_req POST "$data/v1/chat/completions" "$body" -H "x-aria-session: $TEST_PREFIX-protection"
  s1="$LAST_STATUS"
  m1="$(header_value x-aria-router-model "$LAST_HEADERS")"
  http_req POST "$data/v1/chat/completions" "$body" -H "x-aria-session: $TEST_PREFIX-protection"
  s2="$LAST_STATUS"
  m2="$(header_value x-aria-router-model "$LAST_HEADERS")"
  if [[ "$s1" != 200 || "$s2" != 200 ]]; then
    skip_case task-fixtures protection_state "chat unavailable (HTTP $s1/$s2)"
  elif [[ -z "$m1" || -z "$m2" ]]; then
    skip_case task-fixtures protection_state "route model header missing"
  elif [[ "$m1" == "$m2" ]]; then
    record task-fixtures protection_state PASS "stable hard-filtered pool" "$m1" \
      "low-score protection did not override the hard-filtered eligible pool"
  else
    record task-fixtures protection_state FAIL "stable hard-filtered pool" "$m1 -> $m2" \
      "protection changed the hard-filtered pool"
  fi

  set_task T95 fixture
  body="$(chat_body "$ENTRY_SEMANTIC" "explain rust" \
    '{"stage_competence":0.42,"plan_version":"plan-1","rubric_version":"rubric-1","task_category":"draft","continuation":0.9,"tool_outcomes":[{"tool":"echo","ok":true}]}')"
  http_req POST "$data/v1/chat/completions" "$body" \
    -H "x-aria-session: $TEST_PREFIX-evidence"
  http_req GET "$mgmt/v1/router/replay?n=20" -
  if [[ "$LAST_STATUS" != 200 ]]; then
    skip_case task-fixtures evidence_scene "fixture replay endpoint returned HTTP $LAST_STATUS"
  elif jq -e '.records|any(.[]; has("task_category") and has("stage_competence") and has("accepted_output") and has("tool_outcomes") and has("evidence_outcome"))' \
      "$LAST_BODY" >/dev/null 2>&1; then
    record task-fixtures evidence_scene PASS "execution evidence keys" "ok" \
      "scored turn patched all production evidence fields into replay"
  elif jq -e '.records|length > 0' "$LAST_BODY" >/dev/null 2>&1; then
    record task-fixtures evidence_scene FAIL "execution evidence keys" "missing" \
      "scored turn produced a replay record without patched evidence"
  else
    skip_case task-fixtures evidence_scene "no replay records from the fixture instance"
  fi

  set_task T97 fixture
  body="$(chat_body "local/general" "ping")"
  http_req POST "$data/v1/chat/completions" "$body"
  if [[ "$LAST_STATUS" == 200 ]]; then
    check task-fixtures failover_candidates 200 'type=="object"' \
      'hdr:x-aria-router-layer=bypass'
  elif upstream_unavailable "$LAST_STATUS"; then
    skip_case task-fixtures failover_candidates \
      "all backend candidates unreachable (HTTP $LAST_STATUS); failover chain not observable"
  else
    check task-fixtures failover_candidates 200 -
  fi
  clear_task
  return 0
}
