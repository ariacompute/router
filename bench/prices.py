"""USD / million-token price table for cost estimates."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Mapping

# Built-in small table (USD per 1M tokens). Keys are model ids or pool aliases.
DEFAULT_USD_PER_MTOK: dict[str, float] = {
    "local/small": 0.10,
    "local/large": 0.60,
    "local/general": 0.30,
    "small": 0.10,
    "large": 0.60,
    "default": 0.50,
}


def load_prices(path: str | Path | None = None) -> dict[str, float]:
    """Merge DEFAULT with optional JSON object of model -> USD/MTok."""
    out = dict(DEFAULT_USD_PER_MTOK)
    if path is None:
        return out
    p = Path(path)
    data = json.loads(p.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"prices file must be a JSON object, got {type(data).__name__}")
    for k, v in data.items():
        out[str(k)] = float(v)
    return out


def rate_of(model: str, prices: Mapping[str, float] | None = None) -> float:
    table = prices if prices is not None else DEFAULT_USD_PER_MTOK
    if model in table:
        return float(table[model])
    # strip vendor prefix variants
    short = model.split("/")[-1] if "/" in model else model
    if short in table:
        return float(table[short])
    return float(table.get("default", 0.50))


def cost_of(
    model: str,
    tokens: int | float,
    prices: Mapping[str, float] | None = None,
) -> float:
    """USD cost for ``tokens`` at model rate (per million)."""
    if tokens <= 0:
        return 0.0
    return (float(tokens) / 1_000_000.0) * rate_of(model, prices)
