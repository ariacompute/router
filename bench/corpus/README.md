# Bench corpora

Bundled **tiny** fixtures for offline CI. They are **not** official Perplexity DRACO or full MMLU-Pro.

换 small/mid/large 上游或命名时，见 **[MODEL_CHECKLIST.md](../MODEL_CHECKLIST.md)**。

| File | Mode | Notes |
|------|------|-------|
| `routing_tiny.json` | `routing` | ~4 questions with `expected_model` (`local/small` / `local/large`) and `sci-` / `tech-` domains |
| `routing_gateway.json` | `routing` | 5 Gateway/TokenHub items: explain → large (`deepseek-v4-pro`); systems/trade-off → mid (`glm-5.3`); else → small (`glm-5.3-flash`) |
| `routing_gateway_hard.json` | `routing` | ~12 items for §6.6 Track B **and** §6.8 Agent Track B; same TokenHub ids; use with `--prices bench/prices/ariamodel.json` and `--pick-map ariacompute/ariamodel-*=…` |
| `routing_gateway_eval.json` | `routing-accuracy` | **≥200** gold-labeled prompts (templates; optional MMLU/DRACO merge). Fields: `id`, `domain`, `prompt`, `expected_model` (upstream), `intent` (`small`/`mid`/`large`/`trap`). Shared gold for semantic / agent / afm-d. Rebuild: `python -m bench build-routing-eval` |
| `research_tiny.jsonl` | `research` | 2 tasks with mini rubrics (4 axes) + `expected_hits` for label mode |
| `mmlu_tiny.jsonl` | `compare` | ~14 MCQ / yes-no / short-answer items; includes trap rows (`trap-arch-1`, `trap-stand-for`) so vllm keyword mid/large can diverge from aria factoid→small; **reused** for §6.8 agent compare |

## Gold label policy (eval corpus)

| Intent | `expected_model` | Examples |
|--------|------------------|----------|
| large | `deepseek-v4-pro` | explain / teach / how-it-works |
| mid | `glm-5.3` | systems / trade-off / multi-Q / placement |
| small | `glm-5.3-flash` | factoid / acronym / unit / greeting / arithmetic |
| trap → small | `glm-5.3-flash` | factoid with mid/large bait words |

Priority when intents overlap: explanatory (large) > systems (mid) > factoid (small).

```bash
# Regenerate templates (≥210). Optionally merge local HF dumps (do not commit them):
python -m bench build-routing-eval
python -m bench build-routing-eval --from-mmlu ./out/mmlu_pro.jsonl --from-draco ./out/draco_test.jsonl
```

## Download full DRACO (optional)

Dataset: [perplexity-ai/draco](https://huggingface.co/datasets/perplexity-ai/draco) on Hugging Face.

```bash
python -m bench download-draco --out ./out/draco_test.jsonl
```

## MMLU-Pro for `compare` (optional)

Full set: [TIGER-Lab/MMLU-Pro](https://huggingface.co/datasets/TIGER-Lab/MMLU-Pro). Convert locally to JSONL with fields `id`, `question`, `choices` (list), `answer` (letter), optional `category`.

```bash
# Helper may skip (parquet / network); convert yourself and pass --corpus
python -m bench download-mmlu --out ./out/mmlu_pro.jsonl
```

Example convert sketch (optional `datasets` dep, not required by bench):

```python
from datasets import load_dataset
import json
ds = load_dataset("TIGER-Lab/MMLU-Pro", split="test")
with open("mmlu_pro.jsonl", "w") as f:
    for i, row in enumerate(ds):
        f.write(json.dumps({
            "id": f"mmlu-{i}",
            "category": row.get("category"),
            "question": row["question"],
            "choices": row["options"],
            "answer": row["answer"],  # letter
        }) + "\n")
```

Do **not** commit full DRACO / MMLU-Pro dumps or API keys.

## vs vLLM Semantic Router ports

| Component | Suggested bind |
|-----------|----------------|
| Shared backend | `:8000` / `:9001+` |
| aria-router | `:8899` |
| vLLM Semantic Router (Envoy `/v1`) | `:8890` (avoid colliding with aria’s default 8899) |

Bundled minimal config + commands: [`../vllm-sr/`](../vllm-sr/) (`config.yaml`, `vllm-sr validate` / `serve`). This harness does not start `vllm-sr`.
