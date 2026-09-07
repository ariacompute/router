# vLLM Semantic Router (external) for bench Track B

Reference [v0.3 config](https://vllm-sr.ai/docs/installation/configuration/) so local **vLLM Semantic Router** listens on **`:8890`** (aria-router stays on `:8899`). This repo does **not** start `vllm-sr`; install the upstream CLI yourself, then validate and serve with the bundled YAML.

## Prerequisites

- Upstream `vllm-sr` CLI ([vLLM Semantic Router](https://github.com/vllm-project/semantic-router))
- Docker (required by `vllm-sr serve` on Linux / macOS / WSL2)
- An OpenAI-compatible backend on the host (default example: `:8000`)

Edit [`config.yaml`](config.yaml): set `providers.models[].provider_model_id` and `backend_refs[].endpoint` to your backend. Do **not** append `/v1` to `endpoint`.

## Validate and serve

From the **router repo root**:

```bash
vllm-sr validate --config bench/vllm-sr/config.yaml
vllm-sr serve --config bench/vllm-sr/config.yaml
```

Data plane for clients / bench: `http://127.0.0.1:8890`.

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
