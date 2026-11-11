<div align="center">
<a name="readme-top"></a>

# router

**Aria Compute inference gateway**

An OpenAI-compatible HTTP gateway with **three parallel routers** —<br/>
**semantic** (YAML v0.3 signal → decision), a **lightweight builtin agent**<br/>
(fixed in-process tools + limited turns), and **afm-d** (System One Choice via<br/>
aria-engine) — sharing providers, hard constraints, and forwarding.

**English** · [简体中文](./README_cn.md) · [Docs](./AGENTS.md) · [Release notes](./releasenotes.md)

[![][release-shield]][release-link] [![][rust-shield]][rust-link] [![][license-shield]][license-link] [![][lastcommit-shield]][lastcommit-link]
<br/>
[![][routers-shield]][routers-link] [![][sdk-shield]][sdk-link] [![][coverage-shield]][coverage-link] [![][dashboard-shield]][dashboard-link]
</div>

<details>
<summary><kbd>Table of contents</kbd></summary>

#### TOC

**Get started**

- [✨ Features](#-features)
- [🚀 Quick start](#-quick-start)
- [🧰 Command line](#-command-line)
- [🧪 User-facing E2E checks](#-user-facing-e2e-checks)
- [🔧 Configuration](#-configuration)

**Deploy & serve**

- [🛳 Release assets](#-release-assets)
- [🔌 OpenAI API](#-openai-api)
- [🔐 API keys and cost](#-api-keys-and-cost)
- [🌐 Dashboard](#-dashboard)

**Internals**

- [🧭 Three routers](#-three-routers)
- [🔗 ContextHold](#-contexthold)
- [🧩 SDK bindings](#-sdk-bindings)
- [📊 Bench](#-bench)
- [⚖️ Comparison](#-comparison)

**Trust & governance**

- [🔏 Trust store security](#-trust-store-security)
- [🔨 Development](#-development)
- [📚 Documentation](#-documentation)
- [📝 License](#-license)
- [🤝 Engineering conventions](#-engineering-conventions)

</details>

## ✨ Features

aria-router answers one question for an OpenAI-compatible gateway: *which model should answer this request, and under what constraints?* The answer is never a single hard-coded target. Entries map to one of three decision paths; hard constraints prune before ranking; and everything falls back to a fail-closed decision.

- 🔀 **Three parallel routers** — `semantic` (signal → decision), `agent` (in-process builtin tool-loop), and `afm-d` (System One Choice). One request never runs more than one of them.
- 🔒 **Hard constraints before ranking** — location / auth / modality / tool pruning runs first; compute is ranking-only; no eligible path → fail closed.
- 🧠 **AFM-D System One Choice** — ambiguous requests reach `POST {endpoint}/v1/systemone` against aria-engine; low-score protection and a fail-closed fallback keep the decision safe.
- 🪶 **Lightweight builtin agent** — fixed in-process tools + a bounded `max_turns`, `submit_route` → typed `RouteDecision`; no subprocess harness.
- 🔌 **OpenAI-compatible HTTP** — a data plane (`/v1/chat/completions`, `/v1/models`) plus a management plane (health, validate, replay, providers, topology, playground, dashboard).
- 🔑 **Multi-tenant auth & billing** — API keys, OAuth, users, and a six-factor cost view; named provider models bypass recipes and forward straight to the backend.
- 🧩 **Native C ABI + eight-language SDKs** — `ariacompute-router-ffi` / `libaria-router_ffi` with Rust, Python, Go, TypeScript, React Native, Flutter, Swift, and Kotlin wrappers.
- 📊 **Observability & evidence** — competence-aware downgrade, ContextHold prefix reuse, explicit re-decision boundaries, and offline policy-consistency reporting.
- 🖥 **Production dashboard** — a monochrome, flat, accessible Vite React SPA (no Grafana, no ML wizard).

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🚀 Quick start

```bash
# Setup — writes ~/.ariacompute/router.yml (semantic-gateway / agent-gateway / afm-d-gateway)
# and ~/.ariacompute/router-cli.yml (upgrade_url for `aria-router upgrade`).
# Interactive: template → models (base_url, api_key_env name, gateway API key,
#   three provider_model_id tiers; agent also endpoint/model/fallback; afm-d also
#   System One endpoint/fallback) → admin → upgrade_url.
# CI: --template + --admin-user + --admin-password skips model prompts (gateway defaults).
cargo run -p aria-router -- setup
cargo run -p aria-router -- setup --status

# Upgrade CLI + libaria-router_ffi from GitHub/Gitee Releases (needs setup upgrade_url)
cargo run -p aria-router -- upgrade
cargo run -p aria-router -- upgrade 0.1.0

# Validate (default path after setup, or pass --config)
cargo run -p aria-router -- validate

# Serve — data plane from YAML listeners (examples use 127.0.0.1:8899);
# management defaults to 127.0.0.1:8080. Omit --config after setup.
# Offline CI / demos (no GATEWAY_API_KEY): explicit --config *-tiny.yaml
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
```

`--bind` is the **data** plane (`POST /v1/chat/completions`, `GET /v1/models`). `--mgmt-bind` is the **management** plane (`/health`, validate, replay, providers, config, topology, playground chat, and the ops dashboard). One request never runs more than one of semantic, agent, or afm-d. Concrete provider names **bypass** recipes and forward straight to that backend.

### Serve examples

```bash
# Keep --mgmt-bind off engine's 8080 if you will register aria-engine.

# Daily semantic gold path (ariacompute/semantic-auto → local/general)
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Semantic catalog (ariacompute/semantic-auto + ariacompute/semantic-catalog; catalog chat → Unsupported)
cargo run -p aria-router -- serve \
  --config config/examples/semantic.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Stateful semantic (retention sticky + stores.memory / semantic_cache / replay; no ML weights)
cargo run -p aria-router -- serve \
  --config config/examples/semantic-stateful.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Daily agent gold path (ariacompute/agent-auto; builtin tool-loop)
cargo run -p aria-router -- serve \
  --config config/examples/agent-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Agent catalog (ariacompute/agent-auto + ariacompute/agent-catalog; catalog chat → Unsupported)
cargo run -p aria-router -- serve \
  --config config/examples/agent.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Daily AFM-D gold path (ariacompute/afm-d-auto; no endpoint → first-eligible)
cargo run -p aria-router -- serve \
  --config config/examples/afm-d-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# AFM-D catalog (ariacompute/afm-d-auto + ariacompute/afm-d-catalog)
cargo run -p aria-router -- serve \
  --config config/examples/afm-d.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Aria Gateway backends (export GATEWAY_API_KEY first). Same topology as setup default.
# providers.models[].tier marks small/mid/large; provider_model_id is the upstream id (swappable).
export GATEWAY_API_KEY=…
cargo run -p aria-router -- serve \
  --config config/examples/semantic-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

export GATEWAY_API_KEY=…
cargo run -p aria-router -- serve \
  --config config/examples/agent-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# AFM-D gateway (also needs aria-engine at DECISION_MODEL_URL, default :8010 encoder)
export GATEWAY_API_KEY=…
cargo run -p aria-router -- serve \
  --config config/examples/afm-d-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
```

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🧰 Command line

The `aria-router` binary has four subcommands:

| Command | Key flags | Purpose |
|---|---|---|
| `setup` | `--template` · `--admin-user` · `--admin-password` · `--base-url` · `--api-key-env` · `--api-key` · `--model-small` · `--model-mid` · `--model-large` · `--agent-endpoint` · `--agent-model` · `--agent-fallback` · `--afm-d-endpoint` · `--afm-d-fallback` · `--upgrade-url` | Write `~/.ariacompute/router.yml` + `router-cli.yml` |
| `validate` | `--config` | Validate a YAML policy (default path after setup) |
| `serve` | `--config` · `--bind` · `--mgmt-bind` · `--no-dashboard` | Start the data + management planes |
| `upgrade [version]` | — | Fetch CLI + `libaria-router_ffi` from GitHub/Gitee Releases (needs `upgrade_url`) |
| `--version` | — | Print version |

`setup` is interactive: template → models (`base_url`, `api_key_env` name, gateway API key, three `provider_model_id` tiers; agent also `endpoint`/`model`/`fallback`; afm-d also System One `endpoint`/`fallback`) → admin → `upgrade_url`. The gateway API key is stored in `router.yml` (`backend_refs.api_key`); serve loads that path by default (or falls back to `$GATEWAY_API_KEY` / `--api-key-env`). Logical names `ariacompute/ariamodel-{small,mid,large}` stay fixed; swap upstream IDs only. CI: `--template` + `--admin-user` + `--admin-password` skips model prompts (gateway defaults).

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🧪 User-facing E2E checks

[`scripts/run-user-e2e.sh`](scripts/run-user-e2e.sh) is a black-box suite that exercises aria-router from a user's point of view and maps every case to a `task.md` task number. **By default** it only probes an already-running instance (data + management planes); it never builds, installs, starts, stops or upgrades that instance. Two opt-in modes add isolated coverage: `--with-fixtures` starts short-lived **local** instances from temporary YAML fixtures (prebuilt binary only, private `ARIA_COMPUTE_HOME` + loopback ports, stopped on exit), and `--with-local` runs offline static / bench / UI-audit commands. Neither mode touches the default instance.

```bash
# Default data :8899 / management :8090
./scripts/run-user-e2e.sh

# Explicit URLs plus admin credentials for the login/key journey
./scripts/run-user-e2e.sh \
  --data-url http://127.0.0.1:8899 --mgmt-url http://127.0.0.1:8090 \
  --admin-user admin --admin-password '…'

# Safe against a production instance: skip all mutating requests
./scripts/run-user-e2e.sh --read-only

# Task-number coverage, including isolated fixture scenarios
./scripts/run-user-e2e.sh --list
./scripts/run-user-e2e.sh --self-test
./scripts/run-user-e2e.sh --with-local --with-fixtures
./scripts/run-user-e2e.sh --task-focus T90,T91,T92 --with-fixtures
```

Case groups: `cli`, `auth`, `keys`, `data`, `decisioners`, `mgmt-read`, `mgmt-write`, `dashboard`, `task-static`, `task-http`, `local-optional`, `bench-optional`, `task-fixtures` (needs `--with-fixtures`), `task-offline` (needs `--with-local`). `task-static` performs a read-only static contract probe for every real T-number in `task.md` (see `scripts/user-e2e-task-map.tsv`; the T30 / T60–T80 gaps and the duplicate T18 are not invented). `task-http` covers T90/T93/T94/T95/T96 endpoints on the running instance; the fixture group actively triggers the T91 three-question template, T92 low-score protection, T95 execution evidence and T97 backend failover; `task-offline` recomputes T98 policy consistency and runs the T99 UI-audit rule/static checks.

`--list` prints `GROUP / CASE / TASK / DESCRIPTION`. Each result is `PASS`, `FAIL` or `SKIP` and carries a `feature` (T-number) plus a `kind` (`http` / `static` / `fixture` / `offline`). `out/user-e2e-report.json` keeps the original fields and adds `modes`, `coverage` (a per-number PASS/FAIL/SKIP/UNCOVERED matrix with `kind` = declared type and `kinds` = verification types actually executed; T90–T99 listed first in the text report) and the tagged `cases`. `--task-focus` only accepts numbers present in the map; unmapped gaps such as `T30` are rejected with exit 2. Exit codes: `0` all executed cases passed, `1` at least one failure (or skipped cases with `--strict-skip`), `2` usage/environment error such as an unreachable service.

Mutating cases on the default instance use a per-run random prefix and revoke test keys on exit; `--read-only` skips them. `PUT /v1/router/config` writes `router.yml` verbatim and a real `upgrade` replaces the CLI, so both are never executed on the default instance — only their rejecting/help contracts are checked. Provider upsert, trust publish, serve account sync and other irreversible writes need explicit `--allow-mutating`. Fixture instances are isolated, so their scenarios can freely enable `global.services.*`, `afm-d.question_template` and `routing.competence.protection`. Upstream failures (502/504) are `SKIP`, not router defects; tokens/key secrets are redacted from stdout and reports.

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🔧 Configuration

Routing policy is YAML v0.3 (`--config`). Secrets expand as `${VAR}` / `${VAR:-default}`. `entrypoint.router` must equal `recipe.router`. Semantic recipes must not contain `agent:` / `afm-d:`; agent recipes must not contain `signals` / `decisions` / `afm-d:`; afm-d recipes must not contain `routing` / `agent:`. Unknown top-level keys fail `validate`. Unimplemented YAML capabilities return `Unsupported` (no silent no-op). There is **no** top-level `extensions` block and no pi / deepseek-harness subprocess.

| Block | Meaning |
|-------|---------|
| `listeners` | Data-plane bind (`address` + `port`; default `--bind`) |
| `providers` | `defaults.default_model` + named models / `backend_refs` |
| `entrypoints` | Virtual model names → `router: semantic\|agent\|afm-d` + `recipe` |
| `recipes` | Semantic `routing.*`, agent `agent.*`, or afm-d `afm-d.*` (System One endpoint / optional `track` / timeout / fallback) |
| `global` | Auth / keys / users paths (optional) |

### Example policies

| File | Role |
|------|------|
| [`semantic-tiny.yaml`](config/examples/semantic-tiny.yaml) | Daily semantic gold path — `ariacompute/semantic-auto`; keyword heuristics → `local/general`; optional retention; `validate` + `serve` with no ML weights |
| [`semantic.yaml`](config/examples/semantic.yaml) | Semantic catalog — gold `ariacompute/semantic-auto` plus `ariacompute/semantic-catalog` (learned signal + unimplemented algorithm → chat `Unsupported` unless `--features ml`); prefer tiny for demos |
| [`semantic-gateway.yaml`](config/examples/semantic-gateway.yaml) | Semantic + Aria Gateway — pool `ariamodel-{small,mid,large}` via `cloud/gateway` (`tier` + swappable `provider_model_id`); **setup default** for `--template semantic`; needs `GATEWAY_API_KEY` |
| [`semantic-stateful.yaml`](config/examples/semantic-stateful.yaml) | Retention sticky + `stores.memory` / `semantic_cache` / replay |
| [`agent-tiny.yaml`](config/examples/agent-tiny.yaml) | Daily agent gold path — `ariacompute/agent-auto`; in-process builtin tool-loop (no `endpoint` → first-eligible); demos / CI |
| [`agent.yaml`](config/examples/agent.yaml) | Agent catalog — symmetric to `semantic.yaml`: gold `ariacompute/agent-auto` plus `ariacompute/agent-catalog` (intentional `Unsupported`); prefer tiny / gateway for demos |
| [`agent-gateway.yaml`](config/examples/agent-gateway.yaml) | Agent + Aria Gateway — same three cloud models (`tier` + swappable upstream); **setup default** for `--template agent`; needs `GATEWAY_API_KEY` |
| [`afm-d-tiny.yaml`](config/examples/afm-d-tiny.yaml) | Daily AFM-D gold path — `ariacompute/afm-d-auto`; no `endpoint` → first-eligible; demos / CI |
| [`afm-d.yaml`](config/examples/afm-d.yaml) | AFM-D catalog — symmetric to `agent.yaml`: gold `ariacompute/afm-d-auto` plus `ariacompute/afm-d-catalog`; prefer tiny / gateway for demos |
| [`afm-d-gateway.yaml`](config/examples/afm-d-gateway.yaml) | AFM-D + Aria Gateway — System One Choice among three tiers; **setup** `--template afm-d`; needs `GATEWAY_API_KEY` + aria-engine at `DECISION_MODEL_URL` |

Validate any example:

```bash
cargo run -p aria-router -- validate --config config/examples/semantic-tiny.yaml
cargo run -p aria-router -- validate --config config/examples/semantic.yaml
cargo run -p aria-router -- validate --config config/examples/semantic-gateway.yaml
cargo run -p aria-router -- validate --config config/examples/semantic-stateful.yaml
cargo run -p aria-router -- validate --config config/examples/agent-tiny.yaml
cargo run -p aria-router -- validate --config config/examples/agent.yaml
cargo run -p aria-router -- validate --config config/examples/agent-gateway.yaml
cargo run -p aria-router -- validate --config config/examples/afm-d-tiny.yaml
cargo run -p aria-router -- validate --config config/examples/afm-d.yaml
cargo run -p aria-router -- validate --config config/examples/afm-d-gateway.yaml
```

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🛳 Release assets

On each GitHub Release, [`.github/workflows/release.yml`](.github/workflows/release.yml) uploads platform archives next to the `aria-router` CLI:

| Asset | Contents |
|-------|----------|
| `aria-router_<ver>_linux_x86_64.tar.gz` | `aria-router` |
| `aria-router_<ver>_linux_arm64.tar.gz` | `aria-router` |
| `aria-router_<ver>_macos.tar.gz` | `aria-router` |
| `aria-router_<ver>_windows_x86_64.zip` | `aria-router.exe` |
| `libaria-router_ffi_<ver>_<os>.tar.gz` | `libaria-router_ffi.so` / `.dylib` / `aria-router_ffi.dll` |

```bash
# Example: Linux x86_64
tar -xzf aria-router_0.1.0_linux_x86_64.tar.gz
chmod +x aria-router
./aria-router --version

# Or: aria-router upgrade  (after setup; replaces CLI + installs FFI under ~/.ariacompute/lib/)
```

Cut a **GitHub Release** — language package publishes are **fail-pass** and do not block CLI / FFI assets. Version = release tag without leading `v`.

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🔌 OpenAI API

Assuming data plane `http://127.0.0.1:8899` and management `http://127.0.0.1:8090`:

```bash
# List entrypoint + provider names
curl -s http://127.0.0.1:8899/v1/models | jq .

# Semantic entry (keyword recipe in semantic-tiny.yaml)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "ariacompute/semantic-auto",
    "messages":[{"role":"user","content":"please explain rust"}],
    "max_tokens": 32
  }' | jq .

# Agent entry (agent-tiny.yaml)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "ariacompute/agent-auto",
    "messages":[{"role":"user","content":"Hello"}],
    "max_tokens": 32
  }' | jq .

# AFM-D entry (afm-d-tiny.yaml; no endpoint → first-eligible)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "ariacompute/afm-d-auto",
    "messages":[{"role":"user","content":"Hello"}],
    "max_tokens": 32
  }' | jq .

# Chat (SSE)
curl -sN http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "ariacompute/semantic-auto",
    "messages":[{"role":"user","content":"please explain rust"}],
    "stream": true
  }'

# Bypass a concrete provider name
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "local/general",
    "messages":[{"role":"user","content":"Hello"}]
  }' | jq .

# Route headers (layer = semantic | agent | afm-d | bypass)
curl -sD - -o /dev/null http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"ariacompute/semantic-auto","messages":[{"role":"user","content":"please explain rust"}]}' \
  | grep -i x-aria-router

# Management
curl -s http://127.0.0.1:8090/health | jq .
curl -s -X POST http://127.0.0.1:8090/v1/router/validate | jq .
curl -s 'http://127.0.0.1:8090/v1/router/replay?n=20' | jq .
curl -s http://127.0.0.1:8090/v1/router/overview | jq .
curl -s http://127.0.0.1:8090/v1/router/providers | jq .
curl -s http://127.0.0.1:8090/v1/router/topology | jq .
curl -s http://127.0.0.1:8090/v1/router/config | jq .
```

Response headers: `x-aria-router-layer`, `x-aria-router-decision`, `x-aria-router-model`, and `x-aria-router-redecision` (the explicit cross-request boundary: `new-user-message` / `context-compaction` / `tool-continuation` / `same-turn` / `unknown`). When retention emits are set: `x-aria-router-retention-keep-current-model`, `x-aria-router-retention-ttl-turns`, `x-aria-router-retention-drop`, `x-aria-router-retention-prefer-prefix`.

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🔐 API keys and cost

1. Open Dashboard → **API keys** → Generate (`sk-aria_…` shown once). Or `POST /v1/router/keys`.
2. Setup writes `global.require_api_key: true` by default (chat and `PUT /v1/router/providers` need Bearer). Set `false` in YAML to open the data plane.
3. Clients and `aria-engine` use Bearer `sk-aria_…` or `sk-bf-…` (`router_api_key` / `--router-api-key`).
4. **Cost** page / `GET /v1/router/cost` shows six-factor spend plus `by_local_user` / `by_serve_user` / `by_key` (YAML `pricing.input_per_mtok` / `output_per_mtok`).
5. OAuth: Dashboard → Account for `sk-bf-…` (link / paste / reveal).

```bash
# Chat with API key (when require_api_key: true)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -H 'Authorization: Bearer sk-aria_…' \
  -d '{"model":"local/general","messages":[{"role":"user","content":"Hi"}],"max_tokens":16}'

# Issue a key via management (no Bearer needed on 127.0.0.1 mgmt)
curl -s -X POST http://127.0.0.1:8090/v1/router/keys \
  -H 'content-type: application/json' \
  -d '{"name":"ops"}' | jq .
```

Hard constraints (location / auth / modality / tools) prune **before** ranking. Compute is ranking only. No eligible path → fail closed.

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🌐 Dashboard

The management listener serves a Vite React SPA at `http://{mgmt}/` when `dashboard/dist` exists. After `aria-router setup`, open **Login** (local username/password). Pages:

| Section title (same as CLI) | Pages |
|-----------------------------|--------|
| **Local (router Dashboard)** | Login / Register · API keys (`sk-aria_…`) · Users (admin) |
| **OAuth (Aria Compute)** | Account — linked user, `sk-bf-…` reveal/paste/OAuth |
| **Routing** | Config · Topology · Providers · Replay · **Playground** (multi-turn chat, SSE, route headers; no MCP/Claw) · Cost · Cost by Competence · **Model Performance** · **Trust Tiers** |

Cost splits **Local users** vs **OAuth users**. Build:

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
# open http://127.0.0.1:8090/
```

`--no-dashboard` serves JSON APIs only (keys/cost CRUD still work via curl). There is no Grafana or ML wizard; local Login is required when users exist. Bind `127.0.0.1` unless you accept an open admin port.

**Design contract & UI regression audit**: the dashboard is monochrome / flat / square / AA / ≥12px with transform-or-opacity-only motion and a local font stack (no web-font CDN). See [`PRODUCT.md`](PRODUCT.md) for the contract. UI guard tests: `npm --prefix dashboard run test`.

### Local rendering check (re-run)

`scripts/audit-ui.sh` runs the impeccable-style audit in five modes: `--static` (CSS + `index.html` primary rules), `--dom` (jsdom render of 15 routes × light/dark), `--url` (live page + Chrome + impeccable binary), `--browser` (same, but a missing browser is a hard error), and `--list-rules` (print rule ids + page matrix). A mode that cannot run is reported as `skipped`, never as a pass.

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
npm --prefix dashboard run test
./scripts/audit-ui.sh --list-rules
./scripts/audit-ui.sh --static
./scripts/audit-ui.sh --dom
# On a machine with Chrome and the impeccable binary:
IMPECCABLE_BROWSER=/usr/bin/google-chrome ./scripts/audit-ui.sh --url
```

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🧭 Three routers

Every entrypoint maps to exactly one router; a single request never runs more than one. Hard constraints (location / auth / modality / tools) prune **before** ranking, and any eligible path that survives ranking produces a typed `RouteDecision`. When no path is eligible, the gateway fails closed.

| Router | Recipe block | What it does |
|--------|--------------|--------------|
| `semantic` | `routing.*` | Signals → projections → Boolean decision → algorithm / looper. Keyword heuristics need no ML weights; learned signals + unimplemented algorithms return `Unsupported` unless built with `--features ml`. |
| `agent` | `agent.*` | In-process `BuiltinAgent` tool-loop with fixed tools + bounded `max_turns`; `submit_route` yields a typed `RouteDecision`. No `endpoint` → first-eligible backend. |
| `afm-d` | `afm-d.*` | Eligible candidates → System One Choice → `POST {endpoint}/v1/systemone` → typed `RouteDecision`. Named models bypass the recipe and forward straight to that backend. Low-score protection and a fail-closed fallback keep the decision safe. |

The data plane exposes `x-aria-router-layer` (`semantic` | `agent` | `afm-d` | `bypass`), `x-aria-router-decision`, `x-aria-router-model`, and `x-aria-router-redecision` so every re-decision boundary is observable. See [AGENTS.md](./AGENTS.md) for the full decision contract.

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🔗 ContextHold

Hold the routed model through in-turn tool / assistant rounds; re-decide only at natural boundaries (new user message or context compaction). That keeps KV-cache / prefix reuse useful when mixing cheaper and stronger models.

- **`prefix_reuse` defaults to `true`** on semantic, agent, and afm-d recipes (`false` turns it off). Within the same **explicit session** and the same **user-message count**, the frozen `RouteDecision` is reused.
- **New user message** (turn count increases) → fresh decision.
- **Compaction** (message list gets shorter, or system-prompt / `tools` fingerprint changes) → clear DecisionCache + sticky for that session, then re-route.
- **Sticky TTL** (`stores.memory`) decrements only when the user-message count rises — not on every tool-call HTTP request. Re-pinning the same model does not refresh TTL; switching models starts a new TTL budget.
- **afm-d / agent sticky**: with `global.stores.memory.enabled` and `prefix_reuse`, a successful route pins the model; on a later user turn, if it is still hard-eligible, the router short-circuits (skips System One / builtin agent) with reason `sticky:…`. Semantic sticky still comes from decision `emits.retention.keep_current_model` after algorithm selection.
- **Explicit session only**: ContextHold keys off `x-aria-session` or body `session`. Auto-generated `req_*` ids stay cost-only — they do not freeze decisions or write sticky.

Send an explicit session and accumulating chat history (growing user-message count) for turn keys and TTL. Last-message-only clients keep `turn_of == 1` and will not advance sticky TTL. Without an explicit session, routing stickiness matches “decide every request.”

### Behavior at a glance

```text
request + explicit session
        │
        ▼
  compacted? ──yes──► clear DecisionCache + sticky ──► re-route
        │ no
        ▼
  tick sticky TTL if user-turn count increased
        │
        ▼
  same (session, turn) cached? ──yes──► reuse frozen model
        │ no
        ▼
  afm-d/agent sticky still eligible? ──yes──► short-circuit
        │ no
        ▼
  run semantic / agent / afm-d → pin (if applicable)
```

### Config sketch

```yaml
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      prefix_reuse: true   # default true if omitted
      # …

  - name: agent-default
    router: agent
    agent:
      prefix_reuse: true
      # …

  - name: afm-d-default
    router: afm-d
    afm-d:
      prefix_reuse: true
      # …

global:
  stores:
    memory:
      enabled: true
      max_sessions: 4096
      default_ttl_turns: 2
```

Semantic sticky across turns still needs a decision emit, for example:

```yaml
emits:
  - kind: retention
    retention:
      ttl_turns: 2
      keep_current_model: true
```

See [`semantic-stateful.yaml`](config/examples/semantic-stateful.yaml) and the `*-gateway.yaml` examples (`prefix_reuse: true`).

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🧩 SDK bindings

Native C ABI (`ariacompute-router-ffi` / `libaria-router_ffi`) plus thin wrappers under `bindings/`. **Do not** mix with `libaria_ffi` / `ariacompute-engine`.

| Binding | Path | Package |
|---------|------|---------|
| Rust | `bindings/rust` | crates.io `ariacompute-router` (native; no dlopen) |
| Python | `bindings/python` | PyPI `ariacompute-router` (`aria_router`) |
| Go | `bindings/go` | `github.com/ariacompute/router/bindings/go` (cgo; build tag `aria_router_ffi`) |
| TypeScript | `bindings/typescript` | npm `@ariacompute/router-ts` |
| React Native | `bindings/react-native` | npm `@ariacompute/router-rn` |
| Flutter | `bindings/flutter` | pub.dev `aria_router` |
| Swift | `bindings/swift` | CocoaPods / SPM `AriaComputeRouter` |
| Kotlin | `bindings/kotlin` | Maven `com.ariacompute:router` |

C header: [`ffi/include/aria_router.h`](ffi/include/aria_router.h) — `aria_router_init` (in-process YAML; `NULL`/empty → `~/.ariacompute/router.yml`), `aria_router_connect` (HTTP to a running `serve`), `aria_router_complete` / `_stream`, `aria_router_models`, `aria_router_last_route`, `aria_router_destroy`, `aria_router_last_error`.

Dynamic lib order: `ARIA_ROUTER_FFI_LIB` → package-bundled path → `~/.ariacompute/lib/`. Instance `setup` is in-memory `base_url` / `token` only and **never** writes `router.yml`. `init` with no path uses the default `router.yml` from `aria-router setup` (export `GATEWAY_API_KEY` before `serve` for gateway templates). Offline binding CI uses [`bindings/testdata/fast-response.yaml`](bindings/testdata/fast-response.yaml) (canned completion, no upstream). `init` runs semantic and in-process builtin agent (fixed tools + `max_turns`); there is no subprocess harness.

```bash
cargo test -p ariacompute-router-ffi -p ariacompute-router
./scripts/run-binding-tests.sh   # host matrix (Rust / Python / Go / TS / RN setup)
```

C ABI changes must update [`bindings/testdata/cases.json`](bindings/testdata/cases.json) and host tests.

### Examples

In-process `init` + `complete` does **not** need a Dashboard API key. `connect` to a running `serve` with `require_api_key` still needs `setup(token=…)`. Instance `setup` is in-memory only.

**Python** (needs `ARIA_ROUTER_FFI_LIB` or a bundled/cached `libaria-router_ffi`):

```bash
pip install ariacompute-router
```

```python
from aria_router import Router

# After aria-router setup: loads ~/.ariacompute/router.yml
r = Router().init()
print(r.models())
print(r.complete(
    [{"role": "user", "content": "hi"}],
    {"model": "ariacompute/semantic-auto"},
))
print(r.last_route())
r.close()

# Or attach to a running serve (needs token when require_api_key):
r = Router().connect("http://127.0.0.1:8899")
r.setup(base_url="http://127.0.0.1:8899", token="")
```

**Rust** (`ariacompute-router` — native API; does not dlopen `libaria-router_ffi`):

```bash
cargo add ariacompute-router
```

```rust
use ariacompute_router::{Router, SetupUpdates};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut r = Router::new();
    // Empty path → ~/.ariacompute/router.yml (after aria-router setup)
    r.init("")?;
    println!("{}", r.models()?);
    println!(
        "{}",
        r.complete(
            json!([{"role": "user", "content": "hi"}]),
            json!({"model": "ariacompute/semantic-auto"}),
        )?
    );
    println!("{}", r.last_route());

    // Or attach to a running serve (needs token when require_api_key):
    r.connect("http://127.0.0.1:8899");
    r.setup(&SetupUpdates {
        base_url: Some("http://127.0.0.1:8899".into()),
        token: Some("".into()),
    })?;
    Ok(())
}
```

**Go** (cgo + `libaria-router_ffi`; build tag `aria_router_ffi`):

```bash
go get github.com/ariacompute/router/bindings/go
```

```go
package main

import (
	"fmt"
	"log"

	aria "github.com/ariacompute/router/bindings/go"
)

func main() {
	r := aria.NewRouter()
	// Empty path → ~/.ariacompute/router.yml (after aria-router setup)
	if err := r.Init(""); err != nil {
		log.Fatal(err)
	}
	defer r.Close()
	models, err := r.Models()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(models)
	out, err := r.Complete(
		[]map[string]string{{"role": "user", "content": "hi"}},
		map[string]string{"model": "ariacompute/semantic-auto"},
	)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(out)
	route, err := r.LastRoute()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(route)

	// Or attach to a running serve (needs token when require_api_key):
	_ = r.Connect("http://127.0.0.1:8899")
	r.Setup("http://127.0.0.1:8899", "")
}
```

**TypeScript** (`@ariacompute/router-ts`):

```bash
npm install @ariacompute/router-ts
```

```ts
import { Router } from "@ariacompute/router-ts";

// After aria-router setup: loads ~/.ariacompute/router.yml
const r = new Router().init();
console.log(r.models());
console.log(
  r.complete([{ role: "user", content: "hi" }], {
    model: "ariacompute/semantic-auto",
  }),
);
console.log(r.lastRoute());
r.close();

// Or attach to a running serve (needs token when require_api_key):
const c = new Router().connect("http://127.0.0.1:8899");
c.setup({ base_url: "http://127.0.0.1:8899", token: "" });
```

**React Native** (`@ariacompute/router-rn`):

```bash
npm install @ariacompute/router-rn
```

```js
const { Router } = require("@ariacompute/router-rn");

const r = new Router().init();
console.log(r.models());
console.log(
  r.complete([{ role: "user", content: "hi" }], {
    model: "ariacompute/semantic-auto",
  }),
);
console.log(r.lastRoute());
r.close();

// Or attach to a running serve (needs token when require_api_key):
const c = new Router().connect("http://127.0.0.1:8899");
c.setup({ base_url: "http://127.0.0.1:8899", token: "" });
```

**Flutter** (`ariacompute_router`):

```bash
flutter pub add ariacompute_router
```

```dart
import 'package:ariacompute_router/ariacompute_router.dart';

void main() {
  // After aria-router setup: loads ~/.ariacompute/router.yml
  final r = AriaRouter()..init();
  print(r.models());
  print(r.complete(
    [
      {'role': 'user', 'content': 'hi'}
    ],
    {'model': 'ariacompute/semantic-auto'},
  ));
  print(r.lastRoute());
  r.close();

  // Or attach to a running serve (needs token when require_api_key):
  final c = AriaRouter()..connect('http://127.0.0.1:8899');
  c.setup(baseUrl: 'http://127.0.0.1:8899', token: '');
}
```

**Swift** (dlopen `libaria-router_ffi`; SPM / CocoaPods `AriaComputeRouter`):

```bash
pod 'AriaComputeRouter'
```

```swift
import AriaComputeRouter
import Foundation

do {
    // load(nil) → ~/.ariacompute/router.yml (Swift keyword; maps to aria_router_init)
    let r = try Router().load(nil)
    print(try r.models())
    print(try r.complete(
        messages: [["role": "user", "content": "hi"]],
        options: ["model": "ariacompute/semantic-auto"]
    ))
    print(r.lastRoute())
    r.close()

    // Or attach to a running serve (needs token when require_api_key):
    let c = try Router().connect("http://127.0.0.1:8899")
    c.setup(baseUrl: "http://127.0.0.1:8899", token: "")
} catch {
    fputs("\(error)\n", stderr)
}
```

**Kotlin** (JNA + `libaria-router_ffi`; Maven `com.ariacompute:router`):

```kotlin
implementation("com.ariacompute:router:+")
```

```kotlin
import com.ariacompute.router.Router

fun main() {
    // After aria-router setup: loads ~/.ariacompute/router.yml
    Router().init().use { r ->
        println(r.models())
        println(
            r.complete(
                listOf(mapOf("role" to "user", "content" to "hi")),
                mapOf("model" to "ariacompute/semantic-auto"),
            ),
        )
        println(r.lastRoute())
    }

    // Or attach to a running serve (needs token when require_api_key):
    Router().connect("http://127.0.0.1:8899").use { c ->
        c.setup(baseUrl = "http://127.0.0.1:8899", token = "")
    }
}
```

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 📊 Bench

Python ≥3.10 **report-only** harness under [`bench/`](bench/) (stdlib; aligns with `engine/bench`). Does **not** start `aria-router`, [vLLM Semantic Router](https://github.com/vllm-project/semantic-router), or providers — start them yourself, then point `--router` / `--pool` at live OpenAI-compatible URLs. Reports always set `ci_fail: false`.

| Subcommand | Purpose |
|------------|---------|
| `routing` | ADR-040 Q×M matrix + always / oracle / domain / knn / **multi live router** ladder |
| `routing-accuracy` | **Router-API-only** label accuracy (pick vs `expected_model`; no `--pool`) |
| `build-routing-eval` | Generate `routing_gateway_eval.json` (≥200 templates; optional MMLU/DRACO) |
| `research` | Perplexity DRACO-shaped JSONL + rubric axes vs always_X + routers |
| `compare` | MCQ **accuracy + E2E latency + tokens** (parity with VSR public bench narrative) |
| `policy-consistency` | Offline recompute of 4 deterministic policies (cheapest / strongest / expected-label / by-config-expected) from saved decision records — no live router; report-only, `ci_fail: false` |
| `list-corpus` | Print bundled tiny fixtures |
| `download-draco` / `download-mmlu` | Optional HF fetch (skip on network failure) |

`--router` is repeatable as `NAME=URL` (bare URL → `aria_router`). Use `--entrypoint NAME=MODEL`, `--pick-header NAME=HEADER`, `--pick-map FOREIGN=POOL_MODEL`.

Suggested binds when comparing side-by-side: **aria-router `:8899`**, **vLLM SR `:8890`**, shared backends `:8000` / `:9001+`.

Quality modes for routing/research (`--quality`): `label`, `overlap`, `judge` (needs `--judge-url`).

Two common tracks (same `routing` / `compare` CLIs; different what you measure):

| Track | What runs | Flags | Measures |
|-------|-----------|-------|----------|
| Upstream pool-only | Chat **directly** at TokenHub (same triad as Track B) | `--pool` + `--model-id` + `--api-key` + `--prices`; **no** `--router` | Backend quality / cost ladder among `glm-5.3-flash` / `glm-5.3` / `deepseek-v4-pro` (no local router process) |
| Multi-router ladder | Chat via **live routers**, then (optionally) shared pools | `--router` + `--entrypoint` / `--pick-header`, plus `--pool` for always/oracle baselines | Router pick quality (aria-router vs vLLM SR, etc.) on ADR-040 / MCQ |

Optional middle step: serve `semantic-gateway` / `agent-gateway` / `afm-d-gateway` locally and add `--router aria_router=…` (+ `--pick-map`) to the pool-only track so picks go through the router into the same upstream pool.

Tracks in the snippets below: **A** = upstream pool-only; **B** = semantic-auto vs vLLM SR; **C** = agent-auto vs vLLM SR; **D** = afm-d-auto vs vLLM SR; **E** = routing-accuracy (triad corpus, API-only).

```bash
python -m unittest discover -s bench/tests -t .

# --- Track A: upstream pool-only (no local router required) ---
# Same TokenHub triad as Track B semantic routing; corpus expected_model = upstream names.
export GATEWAY_BASE=https://tokenhub.tencentmaas.com
export GATEWAY_API_KEY=your-api-key

# ADR-040 ladder: chat pools directly (always_* / oracle / domain / knn; no --router).
# --quality label scores against corpus expected_model; writes out/gateway_routing.{json,md}.
python -m bench routing \
  --pool small=$GATEWAY_BASE --pool mid=$GATEWAY_BASE --pool large=$GATEWAY_BASE \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --quality label \
  --corpus bench/corpus/routing_gateway.json \
  --report ./out/gateway_routing.json

# MCQ accuracy + E2E latency + tokens on mmlu_tiny via the same pools (always_* only).
python -m bench compare \
  --pool small=$GATEWAY_BASE --pool mid=$GATEWAY_BASE --pool large=$GATEWAY_BASE \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --report ./out/gateway_compare.json
```

```bash
# --- Track B: semantic ladder (aria-router vs vLLM Semantic Router) ---
# Requires local aria-router (:8899), vLLM SR (:8890), and pool backends (:9001+ / :8000).
# Start routers yourself (see bench/vllm-sr/README.md). Pick ONE aria-router config
# (semantic XOR agent XOR afm-d — same --bind). --router = live pick quality; --pool = always/oracle baselines.
export GATEWAY_BASE=https://tokenhub.tencentmaas.com
export GATEWAY_API_KEY=your-api-key

# aria-router data plane :8899 — semantic keyword route → Gateway ariamodel-{small,mid,large}.
# Use --entrypoint ariacompute/semantic-auto in bench below.
aria-router serve \
  --config config/examples/semantic-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Alternative to the block above: builtin agent route (same port; do not run both).
# Use --entrypoint ariacompute/agent-auto instead of semantic-auto.
# aria-router serve \
#   --config config/examples/agent-gateway.yaml \
#   --bind 127.0.0.1:8899 \
#   --mgmt-bind 127.0.0.1:8090

# vLLM Semantic Router data plane :8890 — **keyword baseline** (frozen; not synced to aria advanced).
# validate first: vllm-sr validate --config bench/vllm-sr/config-gateway.yaml
# Bench uses --entrypoint vllm_sr=auto.
vllm-sr validate --config bench/vllm-sr/config-gateway.yaml
vllm-sr serve --config bench/vllm-sr/config-gateway.yaml
# vllm-sr status
# vllm-sr stop

# Live-router ADR-040 ladder (asymmetric §6.6): aria = advanced mom; vllm_sr = keyword baseline.
# Prefer hard corpus + --prices so cost / trap factoids show the gap.
python -m bench routing \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --quality label \
  --corpus bench/corpus/routing_gateway_hard.json \
  --timeout 300 \
  --report ./out/vs_vsr_routing.json

# MCQ compare through each live router (accuracy / latency / tokens / cost) vs always_* on Gateway pool.
# Fair latency: restart aria first to clear in-memory response-cache (p50≈ms otherwise = cache hits).
# After rebuild/restart, shared reqwest Client (§6.7) reuses TLS to TokenHub across compare tasks.
# (In the aria serve terminal) Ctrl-C, then re-run the same `aria-router serve …` with GATEWAY_API_KEY set.
python -m bench compare \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --timeout 300 \
  --report ./out/vs_vsr_compare.json
```

```bash
# --- Track C: agent ladder (aria-router vs vLLM Semantic Router) ---
export GATEWAY_BASE=https://tokenhub.tencentmaas.com
export GATEWAY_API_KEY=your-api-key

# Stop semantic-gateway if running, then:
aria-router serve \
  --config config/examples/agent-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
# vllm-sr keyword baseline remains on :8890 (config-gateway.yaml).

python -m bench routing \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/agent-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --quality label \
  --corpus bench/corpus/routing_gateway_hard.json \
  --timeout 300 \
  --report ./out/agent_vs_vsr_routing.json

python -m bench compare \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/agent-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --timeout 300 \
  --report ./out/agent_vs_vsr_compare.json
```

```bash
# --- Track D: AFM-D ladder (aria-router vs vLLM Semantic Router) ---
# aria = System One Choice (afm-d-auto); vllm_sr = keyword baseline (auto).
export GATEWAY_BASE=https://tokenhub.tencentmaas.com
export GATEWAY_API_KEY=your-api-key
# Encoder System One for afm-d (afm-d-gateway defaults track: encoder @ :8010):
#   aria-engine serve --track encoder --bind 127.0.0.1:8010
# Decoder track uses :8011 (`--track decoder`); set afm-d.track: decoder if needed.
export DECISION_MODEL_URL=http://127.0.0.1:8010

# Stop semantic/agent gateway if running on :8899, then:
aria-router serve \
  --config config/examples/afm-d-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
# vllm-sr keyword baseline remains on :8890 (config-gateway.yaml).

python -m bench routing \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/afm-d-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --quality label \
  --corpus bench/corpus/routing_gateway_hard.json \
  --timeout 300 \
  --report ./out/afmd_vs_vsr_routing.json

python -m bench compare \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/afm-d-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash \
  --model-id mid=glm-5.3 \
  --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --timeout 300 \
  --report ./out/afmd_vs_vsr_compare.json
```

```bash
# --- Track E: routing-accuracy (router API only; ≥200 gold labels) ---
# Measures pick correctness vs expected_model — no --pool / no TokenHub matrix.
# Serve all three entrypoints once via triad-gateway (or --start-router).
# AFM-D needs System One: aria-engine serve --track encoder --bind 127.0.0.1:8010
# GATEWAY_API_KEY must be in the *serve* process env (TokenHub). Do not pass it as
# client Authorization to :8899 (triad-gateway has require_api_key: false).
export GATEWAY_API_KEY=your-api-key
export DECISION_MODEL_URL=http://127.0.0.1:8010

aria-router serve \
  --config config/examples/triad-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# One-shot: semantic → agent → afm-d; writes out/acc_{semantic,agent,afmd}.json + acc_summary.json
# (no GATEWAY_API_KEY needed in this shell; optional ROUTER_API_KEY if require_api_key)
./scripts/run-routing-accuracy.sh
# Or: ./scripts/run-routing-accuracy.sh --start-router
# Skip AFM-D if decision model is down: ./scripts/run-routing-accuracy.sh --skip-afmd

# Single decisioner:
python -m bench routing-accuracy \
  --router http://127.0.0.1:8899 \
  --entrypoint ariacompute/semantic-auto \
  --pick-header x-aria-router-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --corpus bench/corpus/routing_gateway_eval.json \
  --report ./out/acc_semantic.json

# Regenerate eval corpus (templates; optional local MMLU/DRACO merge — do not commit full dumps):
python -m bench build-routing-eval
```

See [`bench/corpus/README.md`](bench/corpus/README.md) and [`bench/vllm-sr/`](bench/vllm-sr/) (external `vllm-sr` config + validate/serve).

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## ⚖️ Comparison

| Dimension | Router (aria-router) | HiRoute | vLLM Semantic Router |
|-----------|:---:|:---:|:---:|
| OpenAI-compatible HTTP gateway (server-side) | ✅ | ❌ | ❌ |
| Three parallel routers (semantic / agent / afm-d) | ✅ | ❌ | ❌ |
| Semantic routing (signal → decision) | ✅ | ✅ | ✅ |
| Hard-constraint pruning (location/auth/modality/tools, pre-rank) | ✅ | ❌ | ❌ |
| Competence-aware downgrade (economic, capability-floored) | ✅ | ✅ | ❌ |
| Bounded failover / fail closed (no silent degrade) | ✅ | ✅ | ❌ |
| Execution evidence retention (task/model/tool/output/competence chain) | ✅ | ✅ | ❌ |
| Long-horizon agent orchestration / delegation | ✅ | ✅ | ❌ |
| Pluggable decision-model extension API | ✅ | ✅ | ❌ |
| Multi-tenant / auth / billing (API key · OAuth · Users · six-factor cost) | ✅ | ❌ | ❌ |
| Cost & competence observability (tier-attributed cost + dashboard) | ✅ | △ | ❌ |
| Explicit re-decision boundary observability (`x-aria-router-redecision`) | ✅ | ❌ | ❌ |
| Offline policy-consistency reporting (recompute from saved records, no router) | ✅ | ❌ | ❌ |
| Context-prefix reuse (frozen decision within a turn) | ✅ | ✅ | ❌ |
| Integration surface (C ABI + eight-language SDK / deploy form) | ✅ | △ | △ |
| Production server-side posture (multi-tenant · governance · operable) | ✅ | ❌ | ❌ |

Legend: ✅ full support · △ partial / limited · ❌ not supported.

**Takeaway**: Router keeps its hard-constraint + fail-closed + three-router-isolation contract while adopting HiRoute's competence-aware soft downgrade, structured execution evidence, pluggable decision extension, and long-horizon delegation — and wraps them in a production-grade OpenAI-compatible gateway with multi-tenant billing and a C ABI / eight-language SDK, covering the full server-side stack beyond vLLM SR's semantic layer.

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🔏 Trust store security

The 2026-10 eMudhra (emSign) intermediate CA key compromise is covered at the binary level: the `aria-router` / `ariacompute-router-ffi` binaries perform outbound TLS for provider API calls and `bin` `upgrade` GitHub/Gitee Release downloads through `reqwest` + `rustls-tls`, whose trust store is the compiled-in **`webpki-roots`**, pinned `>=1.0.10` in `http/Cargo.toml` (1.0.9 → 1.0.10 in `Cargo.lock` via `cargo update -p webpki-roots --precise 1.0.10`). 1.0.10+ removes the compromised emSign roots. The binaries do **not** use the system bundle, so the lock bump is the decisive fix.

The dashboard is a static Vite React SPA served by the Rust binary; it has **no runtime trust store** and makes no outbound TLS of its own. Build-time `npm install` uses the host `ca-certificates`, but that is only the build environment. The `scripts/check-emudhra.sh` detector scans a system or image CA bundle offline for the known root/intermediate SHA-256 fingerprints (exit `0` clean, `1` hit, `2` unreadable).

```bash
scripts/check-emudhra.sh /etc/ssl/certs/ca-certificates.crt
```

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🔨 Development

| Target | Effect |
|---|---|
| `cargo test` | Run the workspace test suite |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint, deny warnings |
| `./scripts/coverage.sh` | 100% line-coverage gate (llvm-cov) |
| `./scripts/run-binding-tests.sh` | Host matrix (Rust / Python / Go / TS / RN setup) |
| `cargo build --release -p aria-router -p ariacompute-router-ffi` | Build the CLI + FFI |

The ops dashboard has its own guard tests and impeccable-style audit (run from the repo root):

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
npm --prefix dashboard run test   # style tokens + 15 routes × light/dark jsdom snapshots
./scripts/audit-ui.sh --static    # CSS / index.html static rules
./scripts/audit-ui.sh --dom       # all routes × both themes (jsdom)
./scripts/audit-ui.sh --url       # Chrome page load (skipped without a browser)
```

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 📚 Documentation

The detailed engineering context lives in the repository root:

| Document | What it covers |
|---|---|
| [AGENTS.md](./AGENTS.md) | Agent engineering context entry and directory index |
| [requirements.md](./requirements.md) | Requirements spec (feature boundaries / exceptions / acceptance criteria), human-review-gated before implementation |
| [task.md](./task.md) | Implementation task checklist |
| [PRODUCT.md](./PRODUCT.md) | Dashboard design contract (monochrome / flat / square / AA / ≥12px) |
| [releasenotes.md](./releasenotes.md) | Release notes |

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 📝 License

MIT. See [LICENSE](./LICENSE).

<div align="right">

[![][back-to-top]](#readme-top)

</div>

## 🤝 Engineering conventions

This repository follows the Harness Engineering philosophy: the repository is the agent's context, and the documents that govern it are versioned alongside the code.

- [`AGENTS.md`](./AGENTS.md) — agent engineering context entry and directory index
- [`requirements.md`](./requirements.md) — requirements spec (feature boundaries / exceptions / acceptance criteria), human-review-gated before implementation
- [`task.md`](./task.md) — implementation task checklist

<div align="right">

[![][back-to-top]](#readme-top)

</div>

---

<div align="center">

Copyright © 2026 [ariacompute][org]. <br/>
This project is [MIT](./LICENSE) licensed.

</div>

<!-- LINK GROUP -->

[org]: https://github.com/ariacompute
[release-shield]: https://img.shields.io/github/v/release/ariacompute/router?style=flat-square&labelColor=black&color=555
[release-link]: https://github.com/ariacompute/router/releases
[rust-shield]: https://img.shields.io/github/rust/cargo/v/ariacompute-router?style=flat-square&labelColor=black&color=555
[rust-link]: ./Cargo.toml
[license-shield]: https://img.shields.io/github/license/ariacompute/router?style=flat-square&labelColor=black&color=555
[license-link]: ./LICENSE
[lastcommit-shield]: https://img.shields.io/github/last-commit/ariacompute/router?style=flat-square&labelColor=black&color=555
[lastcommit-link]: https://github.com/ariacompute/router/commits

[routers-shield]: https://img.shields.io/badge/routers-3_parallel-555?style=flat-square&labelColor=black
[routers-link]: #-three-routers
[sdk-shield]: https://img.shields.io/badge/SDK-8_languages-555?style=flat-square&labelColor=black
[sdk-link]: #-sdk-bindings
[coverage-shield]: https://img.shields.io/badge/coverage-100%25-555?style=flat-square&labelColor=black
[coverage-link]: ./scripts/coverage.sh
[dashboard-shield]: https://img.shields.io/badge/dashboard-monochrome_SPA-555?style=flat-square&labelColor=black
[dashboard-link]: #-dashboard

[back-to-top]: https://img.shields.io/badge/-BACK_TO_TOP-151515?style=flat-square
