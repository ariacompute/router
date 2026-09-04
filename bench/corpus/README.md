# Bench corpora

Bundled **tiny** fixtures for offline CI (`label` quality). They are **not** the official Perplexity DRACO set.

| File | Mode | Notes |
|------|------|-------|
| `routing_tiny.json` | `routing` | ~4 questions with `expected_model` (`local/small` / `local/large`) and `sci-` / `tech-` domains |
| `research_tiny.jsonl` | `research` | 2 tasks with mini rubrics (4 axes) + `expected_hits` for label mode |

## Download full DRACO (optional)

Dataset: [perplexity-ai/draco](https://huggingface.co/datasets/perplexity-ai/draco) on Hugging Face.

```bash
# Helper (urllib; skips with exit 0 if unreachable)
python -m bench download-draco --out ./out/draco_test.jsonl

# Or with huggingface-hub / datasets (optional deps, not required by bench/)
# huggingface-cli download perplexity-ai/draco --repo-type dataset
```

Do **not** commit the full ~100-question JSONL or API keys into this repo. Point `--corpus` at a local download when running research Mode B (`--quality judge`).
