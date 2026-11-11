#!/usr/bin/env bash
# Case library sourced by run-user-e2e.sh. Defines the case catalog and one
# run_group_<name>() function per case group.
#
# Contract facts that the assertions below rely on (verified against the Rust
# implementation, not just the README):
#   - JSON write endpoints need content-type: application/json; missing/blank
#     Content-Type is 415, malformed JSON 400, missing fields 422 (plain text).
#   - Business errors are {"error":{"message","type"}}; the register 503 is a
#     flat {"error":"..."}; OAuth callback failures are 302 redirects.
#   - Unknown model -> 403 fail closed; unsupported capability -> 501; upstream
#     failure -> 502/504; no admin -> 401 (not 403).
#   - GET /v1/router/keys is {"keys":[...]}, not an array; POST keys returns a
#     flat object whose "secret" appears only once.
#   - Management session is a cookie or "Authorization: Bearer <session token>".
#     An sk-aria_/sk-bf- API key is NOT a management session.
#   - SSE is passed through from the upstream: a "data:" frame is expected,
#     but "[DONE]" is NOT guaranteed by the router.
#   - With the disk dashboard, unknown extensionless paths fall back to index.html.

E2E_HAS_SESSION=0
E2E_CREATED_KEY_ID=""
E2E_MODELS_BODY=""
E2E_REQUIRE_KEY=0
E2E_REQUIRE_KEY_PROBED=0
if [[ -n "$SESSION_TOKEN" ]]; then E2E_HAS_SESSION=1; fi

case_catalog() {
  local g i d
  while IFS='|' read -r g i d; do
    [[ -z "$g" ]] && continue
    printf '%s\t%s\t%s\t%s\n' "$g" "$i" "$(task_for_case "$g" "$i")" "$d"
  done <<'EOF'
cli|version_flag|--version prints the aria-router version
cli|version_subcommand|version subcommand prints the version
cli|no_args_help|no arguments prints help and exits 2
cli|serve_help|serve --help documents --mgmt-bind
cli|setup_status|setup --status is flat key: value and read-only
cli|validate_ok|validate --config accepts semantic-tiny.yaml
cli|validate_missing|validate --config rejects a missing file
cli|upgrade_help|upgrade --help is read-only
cli|upgrade_not_executed|real upgrade is intentionally never executed
auth|register_status|register-status exposes allow_register/needs_setup
auth|login_bad_password|login rejects invalid credentials with 401
auth|login_success|admin login returns user + token
auth|me_authenticated|/auth/me works with a session
auth|me_unauthenticated|/auth/me without a session is 401
auth|logout|logout revokes a dedicated session
auth|register_rejects_empty|register rejects an empty username
auth|oauth_start|oauth/start builds an authorize URL
auth|oauth_start_invalid_site|oauth/start rejects an unknown site
auth|oauth_callback_missing_code|oauth callback without code redirects with error
auth|serve_account|serve/account exposes the linked account
auth|password_short_negative|short password is rejected
auth|email_invalid_negative|email without @ is rejected
keys|list|GET keys returns a keys array
keys|create|POST keys returns id/prefix/secret
keys|list_contains_created|the created key appears in the list
keys|spend_view|GET spend returns key_id/used_usd
keys|spend_invalid_period|invalid spend period is rejected
keys|spend_reset_empty|empty spend payload is accepted
keys|revoke_unknown|revoking an unknown key is rejected
cleanup|revoke_<id>|the created test key is revoked
data|models|GET /v1/models lists entrypoints and providers
data|models_mgmt|management /models matches the data plane
data|chat_json|data-plane chat returns a JSON object with route headers
data|chat_mgmt_playground|playground chat works without an API key
data|chat_sse|streamed chat returns text/event-stream with data frames
data|chat_missing_model|chat without model is rejected
data|chat_unknown_model|unknown model fails closed with 403
data|auth_required_when_enabled|require_api_key enforces 401 without a key
data|auth_key_accepted|a valid key is accepted by the data plane
data|chat_unsupported|unsupported capability case requires a special config
decisioners|models_include_*|each entrypoint is advertised by /v1/models
decisioners|chat_*|each entrypoint produces its layer/route headers
decisioners|redecision_header|redecision header uses a documented value
decisioners|retention_headers|retention headers are present when enabled
decisioners|session_stability|repeated same-session requests keep the model
mgmt-read|health|/health reports status ok
mgmt-read|version|version exposes version/commit
mgmt-read|validate|POST validate reports ok
mgmt-read|config|GET config exposes document + yaml
mgmt-read|providers|GET providers exposes models
mgmt-read|overview|overview exposes status and counts
mgmt-read|topology|topology exposes nodes and edges
mgmt-read|models|management models list
mgmt-read|cost|cost report exposes totals and recent events
mgmt-read|cost_limited|cost?n=1 limits recent events
mgmt-read|cost_invalid_n|non-numeric cost?n is rejected
mgmt-read|cost_by_competence|cost/by-competence exposes by_tier
mgmt-read|cost_model_performance|model-performance exposes by_model
mgmt-read|quota|quota exposes enabled
mgmt-read|trust|trust status exposes thresholds
mgmt-read|replay|replay exposes items and records
mgmt-read|replay_limited|replay?n=1 limits items
mgmt-read|users|admin can list users
mgmt-read|users_unauthenticated|listing users without a session is 401
mgmt-write|config_rejects_invalid|invalid YAML is rejected without rewriting config
mgmt-write|config_write|PUT config is skipped (it rewrites router.yml verbatim)
mgmt-write|providers_missing_fields|provider upsert without fields is rejected
mgmt-write|providers_upsert|provider upsert accepts a test provider
mgmt-write|trust_suggest|trust/suggest keeps online routing unchanged
mgmt-write|trust_publish_invalid|unknown draft cannot be published
mgmt-write|trust_publish|a real draft can be published explicitly
mgmt-write|replay_reroute_unknown|unknown replay id is rejected
mgmt-write|replay_reroute_estimate|estimate reroute is purely offline
mgmt-write|users_disabled_unknown|unknown user cannot be disabled
mgmt-write|users_email_unknown|unknown user email update is rejected
mgmt-write|settings_missing_field|allow_register without a field is rejected
mgmt-write|settings_toggle|allow_register toggle is restored afterwards
mgmt-write|auth_email_roundtrip|own email is changed and restored
mgmt-write|serve_account_sync|sync is skipped unless explicitly allowed
dashboard|index|/ serves the SPA index
dashboard|route_*|every SPA route serves the index
dashboard|asset_*|referenced JS/CSS assets are reachable
dashboard|unknown_path_fallback|unknown SPA path falls back or is 404 in API mode
local-optional|cases_json|shared FFI case matrix has 12 cases
local-optional|ffi_*|C ABI header exports all documented functions
local-optional|binding_dir_*|all eight bindings are present
local-optional|binding_runner_*|binding runner references the host test commands
local-optional|dashboard_dist|prebuilt dashboard assets are present
local-optional|tiny_*|offline example policies are present
local-optional|fast_response|canned fast-response fixture is present
local-optional|ffi_prebuilt_lib|a prebuilt FFI library is available
local-optional|runtime_*|language runtimes are detected
local-optional|host_binding_tests|host binding tests are intentionally not run
local-optional|upgrade_not_executed|upgrade is intentionally not executed
bench-optional|help_contract|bench routing-accuracy documents --router
bench-optional|live_accuracy|live accuracy report is generated through the URL
task-static|static_<id>|one static contract probe per real task.md number
task-http|services_present|config exposes global.services
task-http|redecision_config|redecision switches are present and typed
task-http|redecision_header|redecision header uses a documented boundary
task-http|redecision_fail_closed|unknown same-turn history fails closed
task-http|trust_thresholds|trust exposes evidence gates
task-http|trust_suggest_insufficient|insufficient evidence yields no draft
task-http|trust_publish_confirm_required|publish requires explicit confirm
task-http|trust_estimate_no_change|estimate reroute does not change online routing
task-http|model_perf_empty_state|empty state is not reported as zero
task-http|model_perf_route|model-performance SPA route is served
task-http|evidence_replay_fields|replay records carry execution evidence
task-http|evidence_replay_disabled|replay disabled keeps records empty
task-http|quota_fields|quota exposes reservations and settlement errors
task-http|quota_default_off|soft quota is off by default
task-http|quota_key_spend_fields|key spend policy exposes limit/group/period
task-http|quota_exhaust_429|429 requires an isolated fixture instance
task-fixtures|three_question_afmd|afm-d three-question template routes
task-fixtures|protection_state|low-score protection trigger and release
task-fixtures|evidence_scene|production evidence fields in replay
task-fixtures|failover_candidates|backend candidate chain switches
task-offline|policy_consistency_help|policy-consistency documents --records
task-offline|policy_consistency_report|offline recompute report is generated
task-offline|audit_ui_list_rules|UI audit rule set is enumerable
task-offline|audit_ui_static|static UI audit has zero primary findings
EOF
}

find_cli_bin() {
  if [[ -n "$ARIA_ROUTER_BIN" ]]; then
    [[ -x "$ARIA_ROUTER_BIN" ]] || return 1
    printf '%s' "$ARIA_ROUTER_BIN"
    return 0
  fi
  if command -v aria-router >/dev/null 2>&1; then
    command -v aria-router
    return 0
  fi
  local c
  for c in "$ROOT/target/release/aria-router" "$ROOT/target/release/aria-router.exe" \
           "$ROOT/target/debug/aria-router" "$ROOT/target/debug/aria-router.exe"; do
    if [[ -x "$c" ]]; then printf '%s' "$c"; return 0; fi
  done
  return 1
}

chat_body() {
  local model="$1" prompt="$2" extra="${3:-}"
  if [[ -n "$extra" ]]; then
    jq -nc --arg model "$model" --arg prompt "$prompt" --argjson extra "$extra" \
      '{model:$model,messages:[{role:"user",content:$prompt}],max_tokens:1} + $extra'
  else
    jq -nc --arg model "$model" --arg prompt "$prompt" \
      '{model:$model,messages:[{role:"user",content:$prompt}],max_tokens:1}'
  fi
}

header_value() {
  local name="$1" file="$2"
  awk -v want="$(printf '%s' "$name" | tr 'A-Z' 'a-z')" '
    {
      line = $0; sub(/\r$/, "", line)
      pos = index(line, ":")
      if (pos == 0) next
      if (tolower(substr(line, 1, pos - 1)) != want) next
      val = substr(line, pos + 1); sub(/^[ \t]+/, "", val); found = val
    }
    END { print found }
  ' "$file"
}

upstream_unavailable() {
  case "$1" in
    500|502|503|504) return 0 ;;
    *) return 1 ;;
  esac
}

probe_chat() {
  local url="$1" group="$2" id="$3" body="$4"
  shift 4
  http_req POST "$url" "$body" "$@"
  case "$LAST_STATUS" in
    200)
      check "$group" "$id" 200 'type=="object"' \
        'hdr:x-aria-router-layer=any' \
        'hdr:x-aria-router-decision=any' \
        'hdr:x-aria-router-model=any'
      ;;
    401)
      skip_case "$group" "$id" "data plane requires a valid API key (HTTP 401)"
      ;;
    403)
      check "$group" "$id" 403 '.error.message|test("fail closed")'
      ;;
    501)
      check "$group" "$id" 501 '.error.message|test("unsupported")'
      ;;
    500|502|503|504)
      skip_case "$group" "$id" "upstream unavailable (HTTP $LAST_STATUS); routing decision not verifiable"
      ;;
    *)
      check "$group" "$id" 200 -
      ;;
  esac
}

detect_require_api_key() {
  (( E2E_REQUIRE_KEY_PROBED )) && return 0
  E2E_REQUIRE_KEY_PROBED=1
  http_req GET "$MGMT_URL/v1/router/config" - "${SESSION_ARGS[@]}"
  if jq -e '.document.global.require_api_key == true' "$LAST_BODY" >/dev/null 2>&1; then
    E2E_REQUIRE_KEY=1
  else
    E2E_REQUIRE_KEY=0
  fi
}

skip_mutating() {
  skip_case "$1" "$2" "skipped by --read-only (mutating request)"
}

skip_irreversible() {
  skip_case "$1" "$2" "needs --allow-mutating ($3)"
}

run_group_cli() {
  local bin
  if ! bin="$(find_cli_bin)"; then
    skip_case cli unavailable "aria-router binary not found (ARIA_ROUTER_BIN, PATH or target/release)"
    return 0
  fi
  check_local cli version_flag 0 '^aria-router ' "$bin" --version
  check_local cli version_subcommand 0 '^aria-router ' "$bin" version
  check_local cli no_args_help 2 'Usage|Commands' "$bin"
  check_local cli serve_help 0 '\-\-mgmt-bind' "$bin" serve --help
  check_local cli setup_status 0 '^config:' "$bin" setup --status
  check_local cli validate_ok 0 '^ok' "$bin" validate --config "$ROOT/config/examples/semantic-tiny.yaml"
  check_local cli validate_missing 1 'io:|missing|No such file' "$bin" validate --config "$WORK_DIR/no-such-config.yaml"
  check_local cli upgrade_help 0 'Usage|upgrade' "$bin" upgrade --help
  skip_case cli upgrade_not_executed "real upgrade downloads and replaces the CLI; intentionally not executed"
}

run_group_auth() {
  local body tok tmp_tok
  http_req GET "$MGMT_URL/v1/router/auth/register-status" -
  check auth register_status 200 '((.allow_register|type)=="boolean") and ((.needs_setup|type)=="boolean")'

  body="$(jq -nc '{username:"__e2e_invalid__",password:"wrong-password"}')"
  http_req POST "$MGMT_URL/v1/router/auth/login" "$body"
  check auth login_bad_password 401 'has("error")'

  http_req GET "$MGMT_URL/v1/router/auth/me" -
  check auth me_unauthenticated 401 'has("error")'

  if [[ -n "$ADMIN_USER" && -n "$ADMIN_PASSWORD" ]]; then
    body="$(jq -nc --arg u "$ADMIN_USER" --arg p "$ADMIN_PASSWORD" '{username:$u,password:$p}')"
    http_req POST "$MGMT_URL/v1/router/auth/login" "$body"
    if check auth login_success 200 '((.token|type)=="string") and ((.user.username|type)=="string")'; then
      tok="$(jq -r '.token // empty' "$LAST_BODY")"
      if [[ -n "$tok" ]]; then
        SESSION_TOKEN="$tok"
        E2E_HAS_SESSION=1
        SESSION_ARGS=(-H "X-Aria-E2E: 1" -H "Authorization: Bearer $tok")
        http_req GET "$MGMT_URL/v1/router/auth/me" - "${SESSION_ARGS[@]}"
        check auth me_authenticated 200 '(.user.username|type)=="string"'
      else
        skip_case auth me_authenticated "login response contained no token"
      fi
    else
      skip_case auth me_authenticated "login did not succeed"
    fi
  elif [[ -n "$SESSION_TOKEN" ]]; then
    skip_case auth login_success "session token supplied directly (--session-token)"
    E2E_HAS_SESSION=1
    http_req GET "$MGMT_URL/v1/router/auth/me" - "${SESSION_ARGS[@]}"
    check auth me_authenticated 200 '(.user.username|type)=="string"'
  else
    skip_case auth login_success "no admin credentials (--admin-user/--admin-password)"
    skip_case auth me_authenticated "no admin credentials or session token"
  fi

  body="$(jq -nc '{site:"com"}')"
  http_req POST "$MGMT_URL/v1/router/auth/oauth/start" "$body"
  if [[ "$LAST_STATUS" == 200 ]]; then
    check auth oauth_start 200 '((.authorize_url|type)=="string") and ((.state|type)=="string")'
  else
    skip_case auth oauth_start "oauth site not configured on this instance (HTTP $LAST_STATUS)"
  fi
  body="$(jq -nc '{site:"not-a-site"}')"
  http_req POST "$MGMT_URL/v1/router/auth/oauth/start" "$body"
  check auth oauth_start_invalid_site 400 'has("error")'

  http_req GET "$MGMT_URL/v1/router/auth/oauth/callback" -
  check auth oauth_callback_missing_code 302 - 'hdr:location=re:error='

  http_req GET "$MGMT_URL/v1/router/serve/account" - "${SESSION_ARGS[@]}"
  if [[ "$LAST_STATUS" == 200 ]]; then
    check auth serve_account 200 '(.linked|type)=="boolean"'
  else
    skip_case auth serve_account "requires a session or is not configured (HTTP $LAST_STATUS)"
  fi

  body="$(jq -nc '{username:"",password:""}')"
  http_req POST "$MGMT_URL/v1/router/auth/register" "$body"
  check auth register_rejects_empty '400|401|503' 'has("error")'

  body="$(jq -nc '{password:"short"}')"
  http_req POST "$MGMT_URL/v1/router/auth/password" "$body" "${SESSION_ARGS[@]}"
  check auth password_short_negative '400|401' 'has("error")'

  body="$(jq -nc '{email:"not-an-email"}')"
  http_req PUT "$MGMT_URL/v1/router/auth/email" "$body" "${SESSION_ARGS[@]}"
  check auth email_invalid_negative '400|401' 'has("error")'

  if [[ -n "$ADMIN_USER" && -n "$ADMIN_PASSWORD" ]]; then
    body="$(jq -nc --arg u "$ADMIN_USER" --arg p "$ADMIN_PASSWORD" '{username:$u,password:$p}')"
    http_req POST "$MGMT_URL/v1/router/auth/login" "$body"
    tmp_tok="$(jq -r '.token // empty' "$LAST_BODY" 2>/dev/null)"
    if [[ -n "$tmp_tok" ]]; then
      http_req POST "$MGMT_URL/v1/router/auth/logout" - -H "Authorization: Bearer $tmp_tok"
      check auth logout 200 '.ok == true'
    else
      skip_case auth logout "could not obtain a dedicated session for logout"
    fi
  else
    skip_case auth logout "no admin credentials"
  fi
}

run_group_keys() {
  local body kid secret existing_kid="" keys_file="$WORK_DIR/keys_list.json"
  http_req GET "$MGMT_URL/v1/router/keys" - "${SESSION_ARGS[@]}"
  if [[ "$LAST_STATUS" != 200 ]]; then
    skip_case keys list "requires a management session (HTTP $LAST_STATUS)"
    skip_case keys create "requires a management session"
    skip_case keys list_contains_created "no key list available"
    skip_case keys spend_view "no key available"
    skip_case keys spend_invalid_period "no key available"
    skip_case keys spend_reset_empty "no key available"
    return 0
  fi
  check keys list 200 '(.keys|type)=="array"'
  cp "$LAST_BODY" "$keys_file"
  existing_kid="$(jq -r '.keys[0].id // empty' "$keys_file")"

  if (( READ_ONLY )); then
    skip_mutating keys create
    skip_mutating keys list_contains_created
  else
    body="$(jq -nc --arg n "e2e-$TEST_PREFIX" '{name:$n}')"
    http_req POST "$MGMT_URL/v1/router/keys" "$body" "${SESSION_ARGS[@]}"
    if check keys create 200 '((.id|type)=="string") and ((.secret|startswith("sk-aria_"))) and ((.prefix|type)=="string")'; then
      kid="$(jq -r '.id // empty' "$LAST_BODY")"
      secret="$(jq -r '.secret // empty' "$LAST_BODY")"
      if [[ -n "$kid" && -n "$secret" ]]; then
        E2E_CREATED_KEY_ID="$kid"
        register_cleanup delete_key "$kid"
        DATA_KEY_ARGS=(-H "X-Aria-E2E: 1" -H "Authorization: Bearer $secret")
        http_req GET "$MGMT_URL/v1/router/keys" - "${SESSION_ARGS[@]}"
        check keys list_contains_created 200 "(.keys|map(.id)|index(\"$kid\")) != null"
      else
        skip_case keys list_contains_created "key creation returned no id/secret"
      fi
    else
      skip_case keys list_contains_created "key creation did not succeed"
    fi
  fi

  local spend_kid=""
  if [[ -n "$E2E_CREATED_KEY_ID" ]]; then
    spend_kid="$E2E_CREATED_KEY_ID"
  else
    spend_kid="$existing_kid"
  fi
  if [[ -n "$spend_kid" ]]; then
    http_req GET "$MGMT_URL/v1/router/keys/$spend_kid/spend" - "${SESSION_ARGS[@]}"
    check keys spend_view 200 '((.key_id|type)=="string") and ((.used_usd|type)=="number")'
    body="$(jq -nc '{period:"weekly"}')"
    http_req PUT "$MGMT_URL/v1/router/keys/$spend_kid/spend" "$body" "${SESSION_ARGS[@]}"
    check keys spend_invalid_period 400 'has("error")'
    if (( READ_ONLY )); then
      skip_mutating keys spend_reset_empty
    else
      http_req PUT "$MGMT_URL/v1/router/keys/$spend_kid/spend" '{}' "${SESSION_ARGS[@]}"
      check keys spend_reset_empty 200 '(.key_id|type)=="string"'
    fi
  else
    skip_case keys spend_view "no key available on this instance"
    skip_case keys spend_invalid_period "no key available on this instance"
    if (( READ_ONLY )); then skip_mutating keys spend_reset_empty; else skip_case keys spend_reset_empty "no key available"; fi
  fi

  http_req DELETE "$MGMT_URL/v1/router/keys/e2e-unknown-$TEST_PREFIX" - "${SESSION_ARGS[@]}"
  check keys revoke_unknown '400|401' -
}

run_group_data() {
  local body
  http_req GET "$DATA_URL/v1/models" -
  check data models 200 '(.object=="list") and ((.data|type)=="array") and ((.data|length)>0)'
  cp "$LAST_BODY" "$WORK_DIR/models.json"
  E2E_MODELS_BODY="$WORK_DIR/models.json"

  http_req GET "$MGMT_URL/v1/router/models" - "${SESSION_ARGS[@]}"
  check data models_mgmt 200 '(.object=="list") and ((.data|type)=="array")'

  detect_require_api_key

  body="$(chat_body "$ENTRY_SEMANTIC" "please explain rust")"
  probe_chat "$DATA_URL/v1/chat/completions" data chat_json "$body" "${DATA_KEY_ARGS[@]}"
  probe_chat "$MGMT_URL/v1/router/chat" data chat_mgmt_playground "$body" "${SESSION_ARGS[@]}"

  body="$(chat_body "$ENTRY_SEMANTIC" "ping" '{"stream":true}')"
  http_stream POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}"
  if [[ "$LAST_STATUS" == 200 ]]; then
    check data chat_sse 200 - sse
  elif [[ "$LAST_STATUS" == "000" ]] && grep -q 'data:' "$LAST_BODY" 2>/dev/null; then
    record data chat_sse PASS "stream delivers data frames" "stream-timeout" "data frames received before read timeout"
  elif [[ "$LAST_STATUS" == 401 ]]; then
    skip_case data chat_sse "data plane requires a valid API key (HTTP 401)"
  elif upstream_unavailable "$LAST_STATUS"; then
    skip_case data chat_sse "upstream unavailable (HTTP $LAST_STATUS)"
  else
    check data chat_sse 200 - sse
  fi

  body="$(jq -nc '{messages:[{role:"user",content:"ping"}]}')"
  http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}"
  check data chat_missing_model '400|401|422' -

  body="$(chat_body "e2e-unknown-model-$TEST_PREFIX" "ping")"
  http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}"
  case "$LAST_STATUS" in
    403) check data chat_unknown_model 403 '.error.message|test("fail closed")' ;;
    401) skip_case data chat_unknown_model "requires a valid API key (HTTP 401)" ;;
    *)   check data chat_unknown_model 403 - ;;
  esac

  if (( E2E_REQUIRE_KEY )); then
    body="$(chat_body "$ENTRY_SEMANTIC" "ping")"
    http_req POST "$DATA_URL/v1/chat/completions" "$body"
    check data auth_required_when_enabled 401 '.error.message|test("api key required")'
    if [[ -n "$ROUTER_API_KEY" || -n "$E2E_CREATED_KEY_ID" ]]; then
      http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}"
      if [[ "$LAST_STATUS" == 401 ]]; then
        record data auth_key_accepted FAIL "not 401 with key" "401" "the supplied key was rejected"
      else
        record data auth_key_accepted PASS "not 401 with key" "$LAST_STATUS" "key accepted by the data plane"
      fi
    else
      skip_case data auth_key_accepted "no valid API key available"
    fi
  else
    skip_case data auth_required_when_enabled "global.require_api_key is false on this instance"
    skip_case data auth_key_accepted "global.require_api_key is false on this instance"
  fi

  skip_case data chat_unsupported "requires a policy with learned signals and no ml feature; not assumed"
}

run_group_decisioners() {
  local models_file="$E2E_MODELS_BODY"
  if [[ -z "$models_file" || ! -f "$models_file" ]]; then
    http_req GET "$DATA_URL/v1/models" -
    cp "$LAST_BODY" "$WORK_DIR/models.json"
    models_file="$WORK_DIR/models.json"
  fi
  local names=(semantic agent afm-d bypass)
  local models=("$ENTRY_SEMANTIC" "$ENTRY_AGENT" "$ENTRY_AFMD" "$BYPASS_MODEL")
  local layers=(semantic agent afm-d bypass)
  local i name model layer body
  for i in 0 1 2 3; do
    name="${names[i]}"
    model="${models[i]}"
    layer="${layers[i]}"
    if ! jq -e --arg m "$model" '.data|any(.[]; .id == $m)' "$models_file" >/dev/null 2>&1; then
      skip_case decisioners "models_include_$name" "model $model is not configured on this instance"
      skip_case decisioners "chat_$name" "model $model is not configured on this instance"
      continue
    fi
    record decisioners "models_include_$name" PASS "model advertised" "found" "$model"
    body="$(chat_body "$model" "please explain rust")"
    http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}"
    case "$LAST_STATUS" in
      200)
        cp "$LAST_HEADERS" "$WORK_DIR/decision-$name.headers"
        if [[ "$name" == bypass ]]; then
          check decisioners "chat_$name" 200 'type=="object"' \
            "hdr:x-aria-router-layer=$layer" \
            'hdr:x-aria-router-decision=passthrough' \
            'hdr:x-aria-router-bypass=true'
        else
          check decisioners "chat_$name" 200 'type=="object"' \
            "hdr:x-aria-router-layer=$layer" \
            'hdr:x-aria-router-decision=any' \
            'hdr:x-aria-router-model=any' \
            'hdr:x-aria-router-confidence=any' \
            'hdr:x-aria-router-bypass=any'
        fi
        ;;
      401) skip_case decisioners "chat_$name" "requires a valid API key (HTTP 401)" ;;
      403|501) check decisioners "chat_$name" "$LAST_STATUS" 'has("error")' ;;
      500|502|503|504) skip_case decisioners "chat_$name" "upstream unavailable (HTTP $LAST_STATUS)" ;;
      *) check decisioners "chat_$name" 200 - ;;
    esac
  done

  local sem_headers="$WORK_DIR/decision-semantic.headers"
  if [[ -f "$sem_headers" ]]; then
    local rd
    rd="$(header_value x-aria-router-redecision "$sem_headers")"
    if [[ -z "$rd" ]]; then
      skip_case decisioners redecision_header "header not emitted (redecision disabled or emit_header off)"
    else
      case "$rd" in
        new-user-message|context-compaction|tool-continuation|same-turn|unknown)
          record decisioners redecision_header PASS "documented boundary" "$rd" "redecision boundary is valid" ;;
        *) record decisioners redecision_header FAIL "documented boundary" "$rd" "unknown boundary value" ;;
      esac
    fi
    if grep -qi '^x-aria-router-retention-' "$sem_headers"; then
      record decisioners retention_headers PASS "retention headers present" "found" "retention directives emitted"
    else
      skip_case decisioners retention_headers "retention is not enabled on this instance"
    fi
  else
    skip_case decisioners redecision_header "no successful semantic chat response"
    skip_case decisioners retention_headers "no successful semantic chat response"
  fi

  if ! jq -e --arg m "$ENTRY_SEMANTIC" '.data|any(.[]; .id == $m)' "$models_file" >/dev/null 2>&1; then
    skip_case decisioners session_stability "semantic entrypoint is not configured"
    return 0
  fi
  local sess_body m1 m2 s1 s2
  sess_body="$(jq -nc --arg s "$TEST_PREFIX" '{session:$s}')"
  body="$(chat_body "$ENTRY_SEMANTIC" "please explain rust" "$sess_body")"
  http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}" -H "x-aria-session: $TEST_PREFIX"
  s1="$LAST_STATUS"
  m1="$(header_value x-aria-router-model "$LAST_HEADERS")"
  http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}" -H "x-aria-session: $TEST_PREFIX"
  s2="$LAST_STATUS"
  m2="$(header_value x-aria-router-model "$LAST_HEADERS")"
  if [[ "$s1" != 200 || "$s2" != 200 ]]; then
    skip_case decisioners session_stability "chat unavailable (HTTP $s1/$s2); decision stability not verifiable"
  elif [[ -z "$m1" || -z "$m2" ]]; then
    skip_case decisioners session_stability "route model header missing"
  elif [[ "$m1" == "$m2" ]]; then
    record decisioners session_stability PASS "same model for one session" "$m1" "two requests shared x-aria-session"
  else
    record decisioners session_stability FAIL "same model for one session" "$m1 -> $m2" "model changed within one session"
  fi
}

run_group_mgmt_read() {
  http_req GET "$MGMT_URL/health" -
  check mgmt-read health 200 '.status=="ok"'
  http_req GET "$MGMT_URL/v1/router/version" - "${SESSION_ARGS[@]}"
  check mgmt-read version 200 '(.version|type)=="string"'
  http_req POST "$MGMT_URL/v1/router/validate" - "${SESSION_ARGS[@]}"
  check mgmt-read validate 200 '.ok==true'
  http_req GET "$MGMT_URL/v1/router/config" - "${SESSION_ARGS[@]}"
  check mgmt-read config 200 '((.document|type)=="object") and ((.yaml|type)=="string")'
  http_req GET "$MGMT_URL/v1/router/providers" - "${SESSION_ARGS[@]}"
  check mgmt-read providers 200 '(.models|type)=="array"'
  http_req GET "$MGMT_URL/v1/router/overview" - "${SESSION_ARGS[@]}"
  check mgmt-read overview 200 '(.status=="ok") and ((.entrypoints|type)=="number")'
  http_req GET "$MGMT_URL/v1/router/topology" - "${SESSION_ARGS[@]}"
  check mgmt-read topology 200 '((.nodes|type)=="array") and ((.edges|type)=="array")'
  http_req GET "$MGMT_URL/v1/router/models" - "${SESSION_ARGS[@]}"
  check mgmt-read models 200 '(.data|type)=="array"'
  http_req GET "$MGMT_URL/v1/router/cost" - "${SESSION_ARGS[@]}"
  check mgmt-read cost 200 '((.totals|type)=="object") and ((.recent|type)=="array")'
  http_req GET "$MGMT_URL/v1/router/cost?n=1" - "${SESSION_ARGS[@]}"
  check mgmt-read cost_limited 200 '(.recent|length)<=1'
  http_req GET "$MGMT_URL/v1/router/cost?n=not-a-number" - "${SESSION_ARGS[@]}"
  check mgmt-read cost_invalid_n 400 -
  http_req GET "$MGMT_URL/v1/router/cost/by-competence" - "${SESSION_ARGS[@]}"
  check mgmt-read cost_by_competence 200 '(.by_tier|type)=="object"'
  http_req GET "$MGMT_URL/v1/router/cost/model-performance" - "${SESSION_ARGS[@]}"
  check mgmt-read cost_model_performance 200 '(.by_model|type)=="array"'
  http_req GET "$MGMT_URL/v1/router/quota" - "${SESSION_ARGS[@]}"
  check mgmt-read quota 200 '(.enabled|type)=="boolean"'
  http_req GET "$MGMT_URL/v1/router/trust" - "${SESSION_ARGS[@]}"
  check mgmt-read trust 200 '((.thresholds|type)=="object") and (.online_config_unchanged==true)'
  http_req GET "$MGMT_URL/v1/router/replay" - "${SESSION_ARGS[@]}"
  check mgmt-read replay 200 '((.items|type)=="array") and ((.records|type)=="array")'
  http_req GET "$MGMT_URL/v1/router/replay?n=1" - "${SESSION_ARGS[@]}"
  check mgmt-read replay_limited 200 '(.items|length)<=1'
  if (( E2E_HAS_SESSION )); then
    http_req GET "$MGMT_URL/v1/router/users" - "${SESSION_ARGS[@]}"
    check mgmt-read users 200 '(.users|type)=="array"'
  else
    http_req GET "$MGMT_URL/v1/router/users" -
    check mgmt-read users_unauthenticated 401 'has("error")'
    skip_case mgmt-read users "no management session; only the unauthenticated case ran"
  fi
}

run_group_mgmt_write() {
  local body rid allow inverted draft_id original_email
  detect_require_api_key

  E2E_CT="text/plain"
  http_req PUT "$MGMT_URL/v1/router/config" 'this: [is: not: valid'
  check mgmt-write config_rejects_invalid 400 -
  skip_case mgmt-write config_write "PUT config rewrites router.yml verbatim and can lose \${VAR}/comments; only the rejecting negative case runs"

  http_req PUT "$MGMT_URL/v1/router/providers" '{}'
  check mgmt-write providers_missing_fields '400|401|422' -

  body="$(jq -nc --arg d "e2e-nonexistent-$TEST_PREFIX" '{draft_id:$d,confirm:true}')"
  http_req POST "$MGMT_URL/v1/router/trust/publish" "$body"
  check mgmt-write trust_publish_invalid 400 'has("error")'

  http_req POST "$MGMT_URL/v1/router/replay/e2e-unknown-$TEST_PREFIX/reroute" -
  check mgmt-write replay_reroute_unknown 400 'has("error")'

  http_req GET "$MGMT_URL/v1/router/replay?n=1" - "${SESSION_ARGS[@]}"
  rid="$(jq -r '.records[0].id // empty' "$LAST_BODY" 2>/dev/null)"
  if [[ -n "$rid" ]]; then
    http_req POST "$MGMT_URL/v1/router/replay/$rid/reroute?estimate=true" -
    check mgmt-write replay_reroute_estimate 200 '(.estimate==true) and (.online_routing_unchanged==true)'
  else
    skip_case mgmt-write replay_reroute_estimate "no replay records on this instance yet"
  fi

  body="$(jq -nc '{disabled:true}')"
  http_req PUT "$MGMT_URL/v1/router/users/e2e-unknown-$TEST_PREFIX/disabled" "$body" "${SESSION_ARGS[@]}"
  check mgmt-write users_disabled_unknown '400|401' -
  body="$(jq -nc '{email:"not-an-email"}')"
  http_req PUT "$MGMT_URL/v1/router/users/e2e-unknown-$TEST_PREFIX/email" "$body" "${SESSION_ARGS[@]}"
  check mgmt-write users_email_unknown '400|401' -
  http_req PUT "$MGMT_URL/v1/router/settings/allow_register" '{}' "${SESSION_ARGS[@]}"
  check mgmt-write settings_missing_field '400|401|422' -

  if (( READ_ONLY )); then
    skip_mutating mgmt-write trust_suggest
    skip_mutating mgmt-write providers_upsert
    skip_mutating mgmt-write trust_publish
    skip_mutating mgmt-write settings_toggle
    skip_mutating mgmt-write auth_email_roundtrip
    skip_mutating mgmt-write serve_account_sync
    return 0
  fi

  http_req POST "$MGMT_URL/v1/router/trust/suggest" -
  if check mgmt-write trust_suggest 200 '(.status|test("^(disabled|insufficient_evidence|low_confidence|ready)$")) and (.online_routing_unchanged==true)'; then
    cp "$LAST_BODY" "$WORK_DIR/trust-suggest.json"
    draft_id="$(jq -r '.draft.id // .draft.draft_id // empty' "$WORK_DIR/trust-suggest.json")"
  else
    draft_id=""
  fi

  if (( ALLOW_MUTATING )); then
    local -a pargs=(-H "X-Aria-E2E: 1")
    if (( E2E_REQUIRE_KEY )); then pargs=("${DATA_KEY_ARGS[@]}"); fi
    if (( E2E_REQUIRE_KEY )) && [[ -z "$ROUTER_API_KEY" && -z "$E2E_CREATED_KEY_ID" ]]; then
      skip_case mgmt-write providers_upsert "require_api_key is true and no valid key is available"
    else
      body="$(jq -nc --arg n "e2e-probe-$TEST_PREFIX" '{name:$n,endpoint:"http://127.0.0.1:9"}')"
      http_req PUT "$MGMT_URL/v1/router/providers" "$body" "${pargs[@]}"
      check mgmt-write providers_upsert 200 '.ok==true'
      printf 'WARN: test provider e2e-probe-%s remains in memory until the router restarts (no delete endpoint)\n' \
        "$TEST_PREFIX" >&2
    fi

    if [[ -n "$draft_id" ]]; then
      body="$(jq -nc --arg d "$draft_id" '{draft_id:$d,confirm:true}')"
      http_req POST "$MGMT_URL/v1/router/trust/publish" "$body"
      check mgmt-write trust_publish 200 '.online_routing_unchanged==true'
    else
      skip_case mgmt-write trust_publish "no publishable draft was produced by trust/suggest"
    fi

    if (( E2E_HAS_SESSION )); then
      http_req GET "$MGMT_URL/v1/router/auth/register-status" - "${SESSION_ARGS[@]}"
      allow="$(jq -r '.allow_register' "$LAST_BODY")"
      if [[ "$allow" == "true" || "$allow" == "false" ]]; then
        register_cleanup restore_allow_register "$allow"
        inverted="true"
        [[ "$allow" == "true" ]] && inverted="false"
        body="$(jq -nc --argjson v "$inverted" '{allow_register:$v}')"
        http_req PUT "$MGMT_URL/v1/router/settings/allow_register" "$body" "${SESSION_ARGS[@]}"
        check mgmt-write settings_toggle 200 ".allow_register == $inverted"
      else
        skip_case mgmt-write settings_toggle "register-status returned no boolean allow_register"
      fi

      http_req GET "$MGMT_URL/v1/router/auth/me" - "${SESSION_ARGS[@]}"
      original_email="$(jq -r '.user.email // empty' "$LAST_BODY")"
      register_cleanup restore_email "$original_email"
      body="$(jq -nc --arg e "e2e-$TEST_PREFIX@example.invalid" '{email:$e}')"
      http_req PUT "$MGMT_URL/v1/router/auth/email" "$body" "${SESSION_ARGS[@]}"
      check mgmt-write auth_email_roundtrip 200 '.ok==true'
    else
      skip_case mgmt-write settings_toggle "no management session (admin required)"
      skip_case mgmt-write auth_email_roundtrip "no management session"
    fi
  else
    skip_irreversible mgmt-write providers_upsert "adds an in-memory provider that cannot be deleted"
    skip_irreversible mgmt-write trust_publish "publishes a trust draft"
    skip_irreversible mgmt-write settings_toggle "changes a live setting"
    skip_irreversible mgmt-write auth_email_roundtrip "changes the admin email"
  fi

  if (( ALLOW_MUTATING && E2E_HAS_SESSION )); then
    http_req POST "$MGMT_URL/v1/router/serve/account/sync" - "${SESSION_ARGS[@]}"
    check mgmt-write serve_account_sync '200|400|502' -
  else
    skip_irreversible mgmt-write serve_account_sync "contacts the external serve site and rewrites OAuth keys"
  fi
}

run_group_dashboard() {
  local routes=(/login /register /account /cost /cost-competence /model-performance \
                /trust /keys /users /config /topology /providers /replay /playground)
  local r id assets a
  http_req GET "$MGMT_URL/" -
  if [[ "$LAST_STATUS" != 200 ]]; then
    skip_case dashboard index "dashboard not served (HTTP $LAST_STATUS; --no-dashboard?)"
    for r in "${routes[@]}"; do
      skip_case dashboard "route${r//\//_}" "dashboard not served"
    done
    skip_case dashboard assets "dashboard not served"
    skip_case dashboard unknown_path_fallback "dashboard not served"
    return 0
  fi
  check dashboard index 200 - 'body:<div id="root">' 'hdr:content-type=re:text/html'
  cp "$LAST_BODY" "$WORK_DIR/dashboard-index.html"

  for r in "${routes[@]}"; do
    id="route${r//\//_}"
    http_req GET "$MGMT_URL$r" -
    if [[ "$LAST_STATUS" == 200 ]]; then
      check dashboard "$id" 200 - 'body:<div id="root">'
    else
      skip_case dashboard "$id" "SPA route returned HTTP $LAST_STATUS"
    fi
  done

  assets="$(grep -oE '/assets/[A-Za-z0-9._-]+\.(js|css)' "$WORK_DIR/dashboard-index.html" | sort -u)"
  if [[ -z "$assets" ]]; then
    skip_case dashboard assets "index.html references no /assets/ files"
  else
    while IFS= read -r a; do
      [[ -z "$a" ]] && continue
      http_req GET "$MGMT_URL$a" -
      check dashboard "asset_${a##*/}" 200 - 'hdr:content-type=any'
    done <<< "$assets"
  fi

  http_req GET "$MGMT_URL/e2e-unknown-$TEST_PREFIX" -
  if [[ "$LAST_STATUS" == 200 ]]; then
    check dashboard unknown_path_fallback 200 - 'body:<div id="root">'
  elif [[ "$LAST_STATUS" == 404 ]]; then
    record dashboard unknown_path_fallback PASS "status 200|404" "404" "pure API or embedded-assets mode"
  else
    check dashboard unknown_path_fallback '200|404' -
  fi
}

run_group_local_optional() {
  local fn lang spec tool
  check_file_jq local-optional cases_json "$ROOT/bindings/testdata/cases.json" \
    '(.version==1) and ((.cases|length)==12) and ((.cases|map(.id)|index("init_ok"))!=null) and ((.cases|map(.id)|index("fail_closed_unknown_model"))!=null)'
  for fn in aria_router_last_error aria_router_init aria_router_connect aria_router_destroy \
            aria_router_setup aria_router_complete aria_router_complete_stream \
            aria_router_models aria_router_last_route; do
    check_file_grep local-optional "ffi_$fn" "$ROOT/ffi/include/aria_router.h" "$fn"
  done
  for lang in rust python go typescript react-native flutter swift kotlin; do
    if [[ -d "$ROOT/bindings/$lang" ]]; then
      record local-optional "binding_dir_$lang" PASS "binding directory present" "found" "bindings/$lang"
    else
      record local-optional "binding_dir_$lang" FAIL "binding directory present" "missing" "bindings/$lang"
    fi
  done
  check_file_grep local-optional binding_runner_python "$ROOT/scripts/run-binding-tests.sh" "python3 -m unittest"
  check_file_grep local-optional binding_runner_go "$ROOT/scripts/run-binding-tests.sh" "go test -tags aria_router_ffi"
  check_file_grep local-optional binding_runner_ts "$ROOT/scripts/run-binding-tests.sh" "node --test"
  check_file_grep local-optional binding_runner_flutter "$ROOT/scripts/run-binding-tests.sh" "dart test"
  check_file_exists local-optional dashboard_dist "$ROOT/dashboard/dist/index.html"
  if [[ -d "$ROOT/dashboard/dist/assets" ]]; then
    record local-optional dashboard_assets PASS "assets directory present" "found" "dashboard/dist/assets"
  else
    record local-optional dashboard_assets FAIL "assets directory present" "missing" "dashboard/dist/assets"
  fi
  check_file_grep local-optional tiny_semantic "$ROOT/config/examples/semantic-tiny.yaml" "ariacompute/semantic-auto"
  check_file_grep local-optional tiny_agent "$ROOT/config/examples/agent-tiny.yaml" "ariacompute/agent-auto"
  check_file_grep local-optional tiny_afm_d "$ROOT/config/examples/afm-d-tiny.yaml" "ariacompute/afm-d-auto"
  check_file_grep local-optional tiny_offline "$ROOT/config/examples/semantic-tiny.yaml" "require_api_key: false"
  check_file_grep local-optional fast_response "$ROOT/bindings/testdata/fast-response.yaml" "hello-from-router"

  local lib="" cand
  if [[ -n "${ARIA_ROUTER_FFI_LIB:-}" && -f "${ARIA_ROUTER_FFI_LIB:-}" ]]; then
    lib="$ARIA_ROUTER_FFI_LIB"
  fi
  for cand in "$ROOT/target/debug/libaria_router_ffi.so" "$ROOT/target/release/libaria_router_ffi.so" \
              "$ROOT/target/debug/libaria_router_ffi.dylib" "$ROOT/target/release/libaria_router_ffi.dylib" \
              "$ROOT/target/debug/aria_router_ffi.dll" "$ROOT/target/release/aria_router_ffi.dll"; do
    [[ -n "$lib" ]] && break
    [[ -f "$cand" ]] && lib="$cand"
  done
  if [[ -n "$lib" ]]; then
    record local-optional ffi_prebuilt_lib PASS "prebuilt FFI library present" "found" "$lib"
  else
    skip_case local-optional ffi_prebuilt_lib "no prebuilt FFI library; building is disabled by design"
  fi

  for spec in "python:python3" "go:go" "typescript:node" "flutter:dart" "kotlin:java" "swift:swift"; do
    lang="${spec%%:*}"
    tool="${spec#*:}"
    if command -v "$tool" >/dev/null 2>&1; then
      record local-optional "runtime_$lang" PASS "runtime detected" "found" "$tool"
    else
      skip_case local-optional "runtime_$lang" "$tool not installed"
    fi
  done
  skip_case local-optional host_binding_tests "requires cargo build + npm/dart install; not executed by this black-box script"
  skip_case local-optional upgrade_not_executed "upgrade downloads and replaces the CLI; only its --help contract is checked"
  skip_case local-optional swift_tests "swift test requires a toolchain and build; not executed"
}

run_group_bench_optional() {
  if ! command -v python3 >/dev/null 2>&1; then
    skip_case bench-optional help_contract "python3 not available"
    skip_case bench-optional live_accuracy "python3 not available"
    return 0
  fi
  local out rc=0 corpus report md scored
  out="$(cd "$ROOT" && python3 -m bench routing-accuracy --help 2>&1)" || rc=$?
  if (( rc == 0 )) && printf '%s' "$out" | grep -q -- '--router'; then
    record bench-optional help_contract PASS "bench documents --router" "rc=0" "routing-accuracy --help"
  else
    record bench-optional help_contract FAIL "bench documents --router" "rc=$rc" "unexpected help output"
  fi

  corpus="$BENCH_CORPUS"
  if [[ -z "$corpus" ]]; then
    corpus="$WORK_DIR/e2e-corpus.json"
    jq -n '[
      {id:"e2e-small",domain:"misc",prompt:"Say hi in one word.",expected_model:"glm-5.3-flash",intent:"small"},
      {id:"e2e-large",domain:"sci",prompt:"Explain quantum entanglement briefly.",expected_model:"deepseek-v4-pro",intent:"large"}
    ]' > "$corpus"
  fi
  if [[ ! -f "$corpus" ]]; then
    skip_case bench-optional live_accuracy "corpus not found: $corpus"
    return 0
  fi
  report="$WORK_DIR/e2e-accuracy.json"
  md="$WORK_DIR/e2e-accuracy.md"
  rc=0
  if [[ -n "$ROUTER_API_KEY" ]]; then
    out="$(cd "$ROOT" && python3 -m bench routing-accuracy \
      --router "aria_router=$DATA_URL" --entrypoint "$ENTRY_SEMANTIC" \
      --pick-header x-aria-router-model --corpus "$corpus" \
      --report "$report" --report-md "$md" --timeout "$TIMEOUT" --max-tokens 8 \
      --api-key "aria_router=$ROUTER_API_KEY" 2>&1)" || rc=$?
  else
    out="$(cd "$ROOT" && python3 -m bench routing-accuracy \
      --router "aria_router=$DATA_URL" --entrypoint "$ENTRY_SEMANTIC" \
      --pick-header x-aria-router-model --corpus "$corpus" \
      --report "$report" --report-md "$md" --timeout "$TIMEOUT" --max-tokens 8 2>&1)" || rc=$?
  fi
  if (( rc != 0 )); then
    record bench-optional live_accuracy FAIL "bench exits 0" "rc=$rc" "bench failed (output suppressed)"
  elif [[ ! -f "$report" ]]; then
    record bench-optional live_accuracy FAIL "report generated" "missing" "no accuracy report was written"
  elif ! jq -e '.mode=="routing_accuracy" and (.summary|has("overall_accuracy"))' "$report" >/dev/null 2>&1; then
    record bench-optional live_accuracy FAIL "report schema" "mismatch" "unexpected accuracy report structure"
  else
    scored="$(jq -r '.summary.scored // 0' "$report")"
    if [[ "$scored" == "0" ]]; then
      skip_case bench-optional live_accuracy "no scorable responses (upstream unavailable or no decisions)"
    else
      record bench-optional live_accuracy PASS "report schema" "scored=$scored" "accuracy $(jq -r '.summary.overall_accuracy' "$report")"
    fi
  fi
}

task_http_json() {
  local id="$1" file="$2" filter="$3" note="$4"
  if jq -e "$filter" "$file" >/dev/null 2>&1; then
    record task-http "$id" PASS "json contract" "ok" "$note"
  else
    record task-http "$id" FAIL "json contract" "mismatch" "$note"
  fi
}

run_group_task_static() {
  local i id target keyword label
  for i in "${!TASK_IDS[@]}"; do
    id="${TASK_IDS[i]}"
    target="${TASK_TARGET[i]}"
    keyword="${TASK_KEYWORD[i]}"
    label="static_$id"
    set_task "$id" static
    if [[ -z "$target" || "$target" == "-" ]]; then
      skip_case task-static "$label" "no static contract target; functional coverage lives in other groups"
    elif [[ ! -f "$ROOT/$target" ]]; then
      record task-static "$label" FAIL "static target present" "missing" "$target"
    elif [[ -z "$keyword" || "$keyword" == "-" ]] \
         || grep -Fq -- "$keyword" "$ROOT/$target" 2>/dev/null; then
      record task-static "$label" PASS "static target present" "ok" "$target"
    else
      record task-static "$label" FAIL "static keyword present" "missing" "$target lacks keyword"
    fi
    clear_task
  done
}

run_group_task_http() {
  local srv body rid kid min_ev ev rd out
  clear_task
  http_req GET "$MGMT_URL/v1/router/config" - "${SESSION_ARGS[@]}"
  if [[ "$LAST_STATUS" != 200 ]] || ! jq -e . "$LAST_BODY" >/dev/null 2>&1; then
    set_task T90 http
    skip_case task-http services_present "GET /v1/router/config unavailable (HTTP $LAST_STATUS)"
    skip_case task-http redecision_config "config unavailable"
    skip_case task-http redecision_header "config unavailable"
    skip_case task-http redecision_fail_closed "config unavailable"
    set_task T93 http
    skip_case task-http trust_thresholds "config unavailable"
    skip_case task-http trust_suggest_insufficient "config unavailable"
    skip_case task-http trust_publish_confirm_required "config unavailable"
    skip_case task-http trust_estimate_no_change "config unavailable"
    set_task T94 http
    skip_case task-http model_perf_empty_state "config unavailable"
    skip_case task-http model_perf_route "config unavailable"
    set_task T95 http
    skip_case task-http evidence_replay_fields "config unavailable"
    skip_case task-http evidence_replay_disabled "config unavailable"
    set_task T96 http
    skip_case task-http quota_fields "config unavailable"
    skip_case task-http quota_default_off "config unavailable"
    skip_case task-http quota_key_spend_fields "config unavailable"
    skip_case task-http quota_exhaust_429 "config unavailable"
    clear_task
    return 0
  fi
  srv="$WORK_DIR/services.json"
  cp "$LAST_BODY" "$srv"

  set_task T90 http
  task_http_json services_present "$srv" \
    '(.document.global.services|type)=="object"' "global.services present"
  if jq -e '.document.global.services.redecision|type=="object"' "$srv" >/dev/null 2>&1; then
    task_http_json redecision_config "$srv" \
      '((.document.global.services.redecision.enabled|type)=="boolean") and ((.document.global.services.redecision.emit_header|type)=="boolean")' \
      "redecision enabled/emit_header switches"
  else
    skip_case task-http redecision_config "redecision config not present on this instance"
  fi
  body="$(chat_body "$ENTRY_SEMANTIC" "please explain rust" '{"max_tokens":1}')"
  if jq -e '.document.global.services.redecision.enabled == true' "$srv" >/dev/null 2>&1; then
    http_req POST "$DATA_URL/v1/chat/completions" "$body" "${DATA_KEY_ARGS[@]}" \
      -H "x-aria-session: $TEST_PREFIX"
    if [[ "$LAST_STATUS" == 200 ]]; then
      rd="$(header_value x-aria-router-redecision "$LAST_HEADERS")"
      case "$rd" in
        new-user-message|context-compaction|tool-continuation|same-turn|unknown|disabled)
          record task-http redecision_header PASS "documented boundary" "$rd" "redecision header" ;;
        "") skip_case task-http redecision_header "header not emitted (emit_header off or boundary disabled)" ;;
        *) record task-http redecision_header FAIL "documented boundary" "$rd" "unknown boundary value" ;;
      esac
    elif upstream_unavailable "$LAST_STATUS"; then
      skip_case task-http redecision_header "upstream unavailable (HTTP $LAST_STATUS)"
    else
      skip_case task-http redecision_header "chat returned HTTP $LAST_STATUS"
    fi
  else
    skip_case task-http redecision_header "redecision disabled on this instance"
  fi
  skip_case task-http redecision_fail_closed "needs controlled multi-turn history; use --with-fixtures"

  set_task T93 http
  http_req GET "$MGMT_URL/v1/router/trust" - "${SESSION_ARGS[@]}"
  cp "$LAST_BODY" "$WORK_DIR/trust-status.json"
  task_http_json trust_thresholds "$WORK_DIR/trust-status.json" \
    '((.thresholds|type)=="object") and (.online_config_unchanged==true) and (has("evidence_count"))' \
    "trust evidence gates"
  min_ev="$(jq -r '.thresholds.min_evidence // 0' "$WORK_DIR/trust-status.json" 2>/dev/null)"
  ev="$(jq -r '.evidence_count // 0' "$WORK_DIR/trust-status.json" 2>/dev/null)"
  [[ "$min_ev" =~ ^[0-9]+$ ]] || min_ev=0
  [[ "$ev" =~ ^[0-9]+$ ]] || ev=0
  http_req POST "$MGMT_URL/v1/router/trust/suggest" - "${SESSION_ARGS[@]}"
  cp "$LAST_BODY" "$WORK_DIR/trust-suggest.json"
  if [[ "$LAST_STATUS" == 200 ]]; then
    if (( ev < min_ev )); then
      task_http_json trust_suggest_insufficient "$WORK_DIR/trust-suggest.json" \
        '(.draft == null) and (.online_routing_unchanged==true)' "insufficient evidence yields no draft"
    else
      skip_case task-http trust_suggest_insufficient "evidence_count=$ev >= min_evidence=$min_ev"
    fi
  else
    skip_case task-http trust_suggest_insufficient "trust/suggest returned HTTP $LAST_STATUS"
  fi
  body="$(jq -nc --arg d "e2e-nonexistent-$TEST_PREFIX" '{draft_id:$d,confirm:false}')"
  http_req POST "$MGMT_URL/v1/router/trust/publish" "$body" "${SESSION_ARGS[@]}"
  check task-http trust_publish_confirm_required 400 'has("error")'
  http_req GET "$MGMT_URL/v1/router/replay?n=1" - "${SESSION_ARGS[@]}"
  rid="$(jq -r '.records[0].id // empty' "$LAST_BODY" 2>/dev/null)"
  if [[ -n "$rid" ]]; then
    http_req POST "$MGMT_URL/v1/router/replay/$rid/reroute?estimate=true" -
    check task-http trust_estimate_no_change 200 \
      '(.estimate==true) and (.online_routing_unchanged==true)'
  else
    skip_case task-http trust_estimate_no_change "no replay records on this instance"
  fi

  set_task T94 http
  http_req GET "$MGMT_URL/v1/router/cost/model-performance" - "${SESSION_ARGS[@]}"
  cp "$LAST_BODY" "$WORK_DIR/model-perf.json"
  task_http_json model_perf_empty_state "$WORK_DIR/model-perf.json" \
    '((.has_evidence|type)=="boolean") and ((.by_model|type)=="array") and ((.by_tier|type)=="array") and (if .has_evidence == false then ((.by_model|length)==0) and ((.by_tier|length)==0) and ((.estimate_note|type)=="string") else true end)' \
    "empty state is not reported as zero"
  http_req GET "$MGMT_URL/model-performance" -
  if [[ "$LAST_STATUS" == 200 ]]; then
    check task-http model_perf_route 200 - 'body:<div id="root">'
  else
    skip_case task-http model_perf_route "SPA route returned HTTP $LAST_STATUS (--no-dashboard?)"
  fi

  set_task T95 http
  http_req GET "$MGMT_URL/v1/router/replay" - "${SESSION_ARGS[@]}"
  cp "$LAST_BODY" "$WORK_DIR/replay-evidence.json"
  if jq -e '.records|any(.[]; has("task_category") and has("stage_competence") and has("accepted_output") and has("tool_outcomes"))' \
      "$WORK_DIR/replay-evidence.json" >/dev/null 2>&1; then
    task_http_json evidence_replay_fields "$WORK_DIR/replay-evidence.json" \
      '(.records|any(.[]; has("task_category") and has("stage_competence") and has("accepted_output") and has("tool_outcomes")))' \
      "four execution evidence fields"
  elif jq -e '.records|length > 0' "$WORK_DIR/replay-evidence.json" >/dev/null 2>&1; then
    skip_case task-http evidence_replay_fields \
      "no production execution evidence recorded yet; use --with-fixtures to trigger a scored turn"
  else
    skip_case task-http evidence_replay_fields "no replay records on this instance yet"
  fi
  if jq -e '.document.global.services.router_replay.enabled == false' "$srv" >/dev/null 2>&1; then
    task_http_json evidence_replay_disabled "$WORK_DIR/replay-evidence.json" \
      '(.records|length)==0' "replay disabled keeps records empty"
  else
    skip_case task-http evidence_replay_disabled "router_replay enabled on this instance"
  fi

  set_task T96 http
  http_req GET "$MGMT_URL/v1/router/quota" - "${SESSION_ARGS[@]}"
  cp "$LAST_BODY" "$WORK_DIR/quota.json"
  task_http_json quota_fields "$WORK_DIR/quota.json" \
    '((.enabled|type)=="boolean") and ((.pending_reservations|type)=="number") and ((.settlement_errors|type)=="array")' \
    "quota reservation and settlement fields"
  if jq -e '.document.global.services.quota.enabled == false' "$srv" >/dev/null 2>&1; then
    task_http_json quota_default_off "$WORK_DIR/quota.json" '(.enabled==false)' "soft quota off by default"
  else
    skip_case task-http quota_default_off "quota enabled on this instance"
  fi
  kid="$E2E_CREATED_KEY_ID"
  if [[ -z "$kid" ]]; then
    http_req GET "$MGMT_URL/v1/router/keys" - "${SESSION_ARGS[@]}"
    kid="$(jq -r '.keys[0].id // empty' "$LAST_BODY" 2>/dev/null)"
  fi
  if [[ -n "$kid" ]]; then
    http_req GET "$MGMT_URL/v1/router/keys/$kid/spend" - "${SESSION_ARGS[@]}"
    check task-http quota_key_spend_fields 200 \
      '((.key_id|type)=="string") and (has("limit_usd")) and (has("group")) and (has("period"))'
  else
    skip_case task-http quota_key_spend_fields "no key available on this instance"
  fi
  skip_case task-http quota_exhaust_429 "would exhaust a real key; only safe on isolated --with-fixtures instances"
  clear_task
}

run_group_task_offline() {
  local out rc records report md
  clear_task
  set_task T98 offline
  if ! command -v python3 >/dev/null 2>&1; then
    skip_case task-offline policy_consistency_help "python3 not available"
    skip_case task-offline policy_consistency_report "python3 not available"
  else
    rc=0
    out="$(cd "$ROOT" && python3 -m bench policy-consistency --help 2>&1)" || rc=$?
    if (( rc == 0 )) && printf '%s' "$out" | grep -q -- '--records'; then
      record task-offline policy_consistency_help PASS "documents --records" "rc=0" "policy-consistency --help"
    else
      record task-offline policy_consistency_help FAIL "documents --records" "rc=$rc" "unexpected help output"
    fi
    records="$WORK_DIR/consistency-records.json"
    http_req GET "$MGMT_URL/v1/router/replay" - "${SESSION_ARGS[@]}"
    if [[ "$LAST_STATUS" == 200 ]] && jq -e 'has("records")' "$LAST_BODY" >/dev/null 2>&1; then
      cp "$LAST_BODY" "$records"
    else
      jq -n '{records:[{id:"e2e-consistency-1",entrypoint:"ariacompute/semantic-auto",task_category:"general",stage_competence:"small",decision:{model:"glm-5.3-flash"}}]}' > "$records"
    fi
    report="$WORK_DIR/consistency.json"
    md="$WORK_DIR/consistency.md"
    rc=0
    out="$(cd "$ROOT" && python3 -m bench policy-consistency --records "$records" \
      --report "$report" --report-md "$md" 2>&1)" || rc=$?
    if (( rc != 0 )); then
      record task-offline policy_consistency_report FAIL "bench exits 0" "rc=$rc" "offline recompute failed"
    elif [[ ! -f "$report" ]]; then
      record task-offline policy_consistency_report FAIL "report generated" "missing" "no consistency report"
    elif ! jq -e 'has("mode") and (has("summary") or has("policies"))' "$report" >/dev/null 2>&1; then
      record task-offline policy_consistency_report FAIL "report schema" "mismatch" "unexpected consistency report"
    else
      record task-offline policy_consistency_report PASS "report schema" "generated" "offline recompute completed"
    fi
  fi

  set_task T99 offline
  AUDIT_TMP_DIR="e2e-audit-$$"
  if ! command -v python3 >/dev/null 2>&1; then
    skip_case task-offline audit_ui_list_rules "python3 not available"
  else
    rc=0
    out="$(cd "$ROOT" && AUDIT_OUT_DIR="$AUDIT_TMP_DIR" ./scripts/audit-ui.sh --list-rules 2>&1)" || rc=$?
    if (( rc == 0 )) && printf '%s' "$out" | grep -Eq 'primary[[:space:]]+color/aa-contrast'; then
      record task-offline audit_ui_list_rules PASS "rule set enumerable" "rc=0" "primary rules listed"
    else
      record task-offline audit_ui_list_rules FAIL "rule set enumerable" "rc=$rc" "unexpected rule listing"
    fi
  fi
  if ! command -v node >/dev/null 2>&1; then
    skip_case task-offline audit_ui_static \
      "node runtime unavailable; vitest is a node script so --static cannot run (skipped, not a pass)"
  elif [[ ! -x "$ROOT/dashboard/node_modules/.bin/vitest" ]] && ! command -v vitest >/dev/null 2>&1; then
    skip_case task-offline audit_ui_static "vitest unavailable; --static would exit 2 (skipped is not a pass)"
  else
    rc=0
    out="$(cd "$ROOT" && AUDIT_OUT_DIR="$AUDIT_TMP_DIR" ./scripts/audit-ui.sh --static 2>&1)" || rc=$?
    if (( rc == 0 )) && [[ -f "$ROOT/$AUDIT_TMP_DIR/static-status.json" ]] \
       && jq -e '.status=="passed"' "$ROOT/$AUDIT_TMP_DIR/static-status.json" >/dev/null 2>&1; then
      record task-offline audit_ui_static PASS "zero primary findings" "passed" "static UI audit"
    elif printf '%s' "$out" | grep -Eqi \
         'vitest not installed|module_not_found|command not found|/usr/bin/env.*node|node.*no such file|node.*not found'; then
      skip_case task-offline audit_ui_static \
        "static audit toolchain could not start in this environment (skipped, not a pass)"
    else
      record task-offline audit_ui_static FAIL "zero primary findings" "rc=$rc" "static audit reported findings"
    fi
  fi
  clear_task
}
