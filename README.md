# router

[English](README.md) | [中文](README_cn.md)

Aria Compute inference gateway: OpenAI-compatible HTTP, two parallel routers (**semantic** YAML v0.3 and **agent** via extensions). Shared providers, hard constraints, and forwarding.

## Build / Test

```bash
cargo test
cargo clippy --workspace --all-targets -- -D warnings
./scripts/run-binding-tests.sh
```

## Config / Run

Routing policy is YAML v0.3 (`--config`). Secrets expand as `${VAR}` / `${VAR:-default}`. `entrypoint.router` must equal `recipe.router`. Semantic recipes must not contain `agent:`; agent recipes must not contain `signals` / `decisions`. Unknown top-level keys fail `validate`. Unimplemented YAML capabilities return `Unsupported` (no silent no-op). Missing `pi` / `deepseek-harness` binaries fail **serve**, not a silent fallback to semantic.

| Block | Meaning |
|-------|---------|
| `listeners` | Data-plane bind (`address` + `port`; default `--bind`) |
| `providers` | `defaults.default_model` + named models / `backend_refs` |
| `extensions` | Agent adapters: `builtin` / `pi` / `deepseek-harness` |
| `entrypoints` | Virtual model names → `router: semantic\|agent` + `recipe` |
| `recipes` | Semantic `routing.*` or agent `agent.*` |
| `global` | Observability / classifier assets (optional) |

Examples (English comments in every file):

| File | Role |
|------|------|
| [`semantic-tiny.yaml`](config/examples/semantic-tiny.yaml) / [`agent-tiny.yaml`](config/examples/agent-tiny.yaml) / [`ffi-tiny.yaml`](config/examples/ffi-tiny.yaml) | Gold path — `validate` + `serve` / FFI |
| [`semantic.yaml`](config/examples/semantic.yaml) | heuristics plus `aria/semantic-catalog` (learned signal + unimplemented algorithm → chat `Unsupported`) |
| [`agent.yaml`](config/examples/agent.yaml) | `builtin` + `pi` + `deepseek-harness`. `validate` ok; `serve` needs those binaries |
| [`ffi.yaml`](config/examples/ffi.yaml) | gold `fast-response` plus the same catalog recipe |

```bash
# Setup — two credential sections (do not mix prefixes):
#   [1/2] Local (router Dashboard) — admin user/password; sk-aria_ keys minted in Dashboard → Keys
#   [2/2] OAuth (Aria Compute)     — optional bfvk-… into ~/.ariacompute/router-serve.json
aria-router setup
aria-router setup --status
# Non-interactive Local flags: --admin-user --admin-password --allow-register --require-api-key
# Non-interactive OAuth flags: --serve-site com|cn --serve-api-key bfvk-…

# Validate (default path, or pass --config)
aria-router validate
cargo run -p aria-router -- validate --config config/examples/semantic-tiny.yaml

# Serve — data plane from YAML listeners (semantic-tiny: 127.0.0.1:8899);
# management defaults to 127.0.0.1:8080. Omit --config after setup.
cargo run -p aria-router -- serve --config config/examples/semantic-tiny.yaml
aria-router serve --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090

# Explicit binds (keep mgmt off engine's 8080 if you will register aria-engine)
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

cargo run -p aria-router -- serve \
  --config config/examples/agent-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
```

`--bind` is the **data** plane (`POST /v1/chat/completions`, `GET /v1/models`). `--mgmt-bind` is the **management** plane (`/health`, validate, replay, providers, config, topology, playground chat, and the ops dashboard). One request never runs both semantic and agent. Concrete provider names **bypass** recipes and forward straight to that backend.

## Dashboard

The management listener serves a Vite React SPA at `http://{mgmt}/` when `dashboard/dist` exists. After `aria-router setup`, open **Login** (local username/password). Pages:

| Section title (same as CLI) | Pages |
|-----------------------------|--------|
| **Local (router Dashboard)** | Login / Register · API keys (`sk-aria_…`) · Users (admin) |
| **OAuth (Aria Compute)** | Account — linked user, `bfvk-…` reveal/paste/OAuth |

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

### API keys and cost

1. Open Dashboard → **API keys** → Generate (`sk-aria_…` shown once). Or `POST /v1/router/keys`.
2. `aria-router setup` → enable `global.require_api_key` when you want chat and `PUT /v1/router/providers` to require Bearer.
3. Clients and `aria-engine` use the same **Local** secret (`router_api_key` / `--router-api-key`; never paste `bfvk-` there).
4. **Cost** page / `GET /v1/router/cost` shows six-factor spend plus `by_local_user` / `by_serve_user` / `by_key` (YAML `pricing.input_per_mtok` / `output_per_mtok`).
5. OAuth: Dashboard → Account or `aria-router setup` [2/2] for `bfvk-…`.

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

## Register aria-engine

This process routes; `aria-engine serve` optionally registers as a local provider. Use **different ports**: engine `--bind` vs this `--mgmt-bind` (default `127.0.0.1:8080`). Clients talk to the **data** plane.

```bash
# 1. router repo — data :8899, management :8090
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# 2. engine repo — OpenAI on :8080, then PUT to management
# When router require_api_key is true, pass the Dashboard-issued secret:
aria-engine serve gemma-4-e2b-it_q4 \
  --bind 127.0.0.1:8080 \
  --router http://127.0.0.1:8090 \
  --router-api-key sk-aria_… \
  --compute auto

# Persist on the engine side instead of --router / --router-api-key each time:
#   aria-engine setup  # router URL + optional router API key (from Dashboard)
#   # or ~/.ariacompute/engine.yml:
#   # router: http://127.0.0.1:8090
#   # router_api_key: sk-aria_…

# 3. Chat via this gateway (concrete name = bypass → registered engine)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "gemma-4-e2b-it_q4",
    "messages":[{"role":"user","content":"Hello"}],
    "max_tokens": 32
  }' | jq .
```

`serve` on engine does `PUT {router}/v1/router/providers` with `{name, endpoint, provider_model_id, locality}` and **exits** if that fails. `aria/semantic-auto` / `aria/agent-auto` only hit that engine if YAML `modelRefs` / `default_model` use the same registered name.

Manual upsert (same contract):

```bash
curl -s -X PUT http://127.0.0.1:8090/v1/router/providers \
  -H 'content-type: application/json' \
  -d '{
    "name": "gemma-4-e2b-it_q4",
    "endpoint": "127.0.0.1:8080",
    "provider_model_id": "gemma-4-e2b-it_q4",
    "locality": "local"
  }' | jq .
```

## OpenAI API

Assuming data plane `http://127.0.0.1:8899` and management `http://127.0.0.1:8090`:

```bash
# List entrypoint + provider names
curl -s http://127.0.0.1:8899/v1/models | jq .

# Semantic entry (keyword recipe in semantic-tiny.yaml)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "aria/semantic-auto",
    "messages":[{"role":"user","content":"please explain rust"}],
    "max_tokens": 32
  }' | jq .

# Agent entry (agent-tiny.yaml)
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "aria/agent-auto",
    "messages":[{"role":"user","content":"Hello"}],
    "max_tokens": 32
  }' | jq .

# Chat (SSE)
curl -sN http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "aria/semantic-auto",
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

# Route headers (layer = semantic | agent | bypass)
curl -sD - -o /dev/null http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"aria/semantic-auto","messages":[{"role":"user","content":"please explain rust"}]}' \
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

Response headers: `x-aria-router-layer`, `x-aria-router-decision`, `x-aria-router-model`.

## SDK Bindings

Native C ABI (`ariacompute-router-ffi` / `libaria-router_ffi`) plus thin wrappers under `bindings/`. **Do not** mix with `libaria_ffi` / `ariacompute-engine`.

| Binding | Path | Package |
|---------|------|---------|
| Rust | `bindings/rust` | `ariacompute-router` (native; no dlopen) |
| Python | `bindings/python` | `aria_router` |
| Go | `bindings/go` | Go module |
| TypeScript | `bindings/typescript` | npm `@ariacompute/router-ts` |
| React Native | `bindings/react-native` | npm `@ariacompute/router-rn` |
| Flutter | `bindings/flutter` | pub.dev |
| Swift | `bindings/swift` | CocoaPods |
| Kotlin | `bindings/kotlin` | Maven |

C header: [`ffi/include/aria_router.h`](ffi/include/aria_router.h) — `aria_router_init` (in-process YAML), `aria_router_connect` (HTTP to a running `serve`), `aria_router_complete` / `_stream`, `aria_router_models`, `aria_router_last_route`, `aria_router_destroy`, `aria_router_last_error`.

Dynamic lib order: `ARIA_ROUTER_FFI_LIB` → package-bundled path → `~/.ariacompute/lib/`. Instance `setup` is in-memory `base_url` / `token` only and **never** writes `router.yml`. `init` runs semantic and `builtin` agent in-process; `type: pi` / `deepseek-harness` on platforms without subprocess is explicit `Unsupported`.

```bash
cargo test -p ariacompute-router-ffi -p ariacompute-router
./scripts/run-binding-tests.sh   # host matrix (Rust / Python / Go / TS / RN setup)
```

C ABI changes must update [`bindings/testdata/cases.json`](bindings/testdata/cases.json) and host tests.

### Examples

**Python** (needs `ARIA_ROUTER_FFI_LIB` or a bundled/cached `libaria-router_ffi`):

```python
from aria_router import Router

r = Router().init("config/examples/ffi-tiny.yaml")
print(r.models())
print(r.complete(
    [{"role": "user", "content": "hi"}],
    {"model": "aria/semantic-auto"},
))
print(r.last_route())
r.close()

# Or attach to a running serve (data plane):
r = Router().connect("http://127.0.0.1:8899")
r.setup(base_url="http://127.0.0.1:8899", token="")  # memory only
```

**Rust** (`ariacompute-router` — native API; does not dlopen `libaria-router_ffi`):

```rust
use ariacompute_router::Router;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut r = Router::new();
    r.init("config/examples/ffi-tiny.yaml")?;
    let out = r.complete(
        json!([{"role": "user", "content": "hi"}]),
        json!({"model": "aria/semantic-auto"}),
    )?;
    println!("{out}");
    Ok(())
}
```

## Release assets

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
```

Cut a **GitHub Release** — language package publishes are **fail-pass** and do not block CLI / FFI assets. Version = release tag without leading `v`.

## Engineering Conventions

This repository follows the Harness Engineering philosophy:

- [`AGENTS.md`](AGENTS.md): Agent engineering context entry and directory index
- [`requirements.md`](requirements.md): Requirements spec (feature boundaries/exceptions/acceptance criteria, human-review-gated)
- [`task.md`](task.md): Implementation task checklist
