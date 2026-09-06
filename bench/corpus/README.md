# Bench corpora

Bundled **tiny** fixtures for offline CI. They are **not** official Perplexity DRACO or full MMLU-Pro.

| File | Mode | Notes |
|------|------|-------|
| `routing_tiny.json` | `routing` | ~4 questions with `expected_model` (`local/small` / `local/large`) and `sci-` / `tech-` domains |
| `research_tiny.jsonl` | `research` | 2 tasks with mini rubrics (4 axes) + `expected_hits` for label mode |
| `mmlu_tiny.jsonl` | `compare` | ~12 synthetic MCQ / yes-no / short-answer items |

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
