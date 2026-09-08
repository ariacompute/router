# vLLM Semantic Router (external) for bench Track B

Reference [v0.3 config](https://vllm-sr.ai/docs/installation/configuration/) so local **vLLM Semantic Router** listens on **`:8890`** (aria-router stays on `:8899`). This repo does **not** start `vllm-sr`; install the upstream CLI yourself, then validate and serve with a bundled YAML.

| File | Backends |
|------|----------|
| [`config.yaml`](config.yaml) | Local OpenAI-compatible `:8000` (`host.docker.internal`) |
| [`config-gateway.yaml`](config-gateway.yaml) | Aria Gateway — **keyword baseline (frozen)** for Track B §6.6; not synced to aria advanced [`semantic-gateway.yaml`](../../config/examples/semantic-gateway.yaml) |

## Prerequisites

- Upstream `vllm-sr` CLI ([vLLM Semantic Router](https://github.com/vllm-project/semantic-router))
- Docker (required by `vllm-sr serve` on Linux / macOS / WSL2)
- **Local config**: backend on host `:8000` (edit `provider_model_id` / `endpoint`; do **not** append `/v1`)
- **Gateway config**: `export GATEWAY_API_KEY=…` (never commit secrets); `base_url` **must** end with `/v1`

## Validate and serve

From the **router repo root**:

```bash
# Local backend
vllm-sr validate --config bench/vllm-sr/config.yaml
vllm-sr serve --config bench/vllm-sr/config.yaml

# Aria Gateway (keyword baseline — deliberately simpler than semantic-gateway)
export GATEWAY_API_KEY=…   # do not commit
vllm-sr validate --config bench/vllm-sr/config-gateway.yaml
vllm-sr serve --config bench/vllm-sr/config-gateway.yaml
```

Data plane for clients / bench: `http://127.0.0.1:8890`.

`config-gateway.yaml` routing (**baseline frozen**): keyword explain → large; systems/trade-off (`needs_mid` incl. `architecture` → `systems_mid`) → mid; multi-turn / multi-question → mid; else static → small. `default_model` = mid. No projection conditions / factoid_small / pricing — those stay on aria only. Bench hard corpus: [`../corpus/routing_gateway_hard.json`](../corpus/routing_gateway_hard.json) + [`../prices/ariamodel.json`](../prices/ariamodel.json).

Gateway `backend_refs` must set `provider`/`type: openai` (+ Bearer `api_key_env`); missing `type` → auth 500 after route. Use `base_url: https://gateway.ariacompute.com/v1` (without `/v1`, Envoy posts `/chat/completions` → Gateway 405). Keep `global.stores.semantic_cache.enabled: false` unless mmbert embeddings are ready.

## Point bench at it

```bash
python -m bench routing \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint vllm_sr=auto \
  ...

python -m bench compare \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint vllm_sr=auto \
  ...
```

See root [`README.md`](../../README.md) Track B for the full aria-router vs vLLM SR ladder.
