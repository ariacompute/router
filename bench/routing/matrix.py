"""ADR-040 routing matrix helpers."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable


@dataclass
class Cell:
    quality: float
    tokens: int
    signal: float | None = None
    cost_usd: float = 0.0
    status: str = "ok"  # ok | error | skipped
    detail: dict[str, Any] = field(default_factory=dict)


@dataclass
class RoutingMatrix:
    """Question × model cells."""

    question_ids: list[str]
    models: list[str]
    cells: dict[tuple[str, str], Cell]  # (qid, model) -> Cell
    meta: dict[str, Any] = field(default_factory=dict)

    def get(self, qid: str, model: str) -> Cell | None:
        return self.cells.get((qid, model))

    def qualities(self, qid: str) -> dict[str, float]:
        return {
            m: self.cells[(qid, m)].quality
            for m in self.models
            if (qid, m) in self.cells and self.cells[(qid, m)].status == "ok"
        }

    def tokens_map(self, qid: str) -> dict[str, int]:
        return {
            m: self.cells[(qid, m)].tokens
            for m in self.models
            if (qid, m) in self.cells and self.cells[(qid, m)].status == "ok"
        }

    def costs(self, qid: str) -> dict[str, float]:
        return {
            m: self.cells[(qid, m)].cost_usd
            for m in self.models
            if (qid, m) in self.cells and self.cells[(qid, m)].status == "ok"
        }

    def to_serializable(self) -> dict[str, Any]:
        rows = []
        for qid in self.question_ids:
            for m in self.models:
                c = self.cells.get((qid, m))
                if c is None:
                    continue
                rows.append(
                    {
                        "question_id": qid,
                        "model": m,
                        "quality": c.quality,
                        "tokens": c.tokens,
                        "signal": c.signal,
                        "cost_usd": c.cost_usd,
                        "status": c.status,
                        **({"detail": c.detail} if c.detail else {}),
                    }
                )
        return {
            "question_ids": list(self.question_ids),
            "models": list(self.models),
            "cells": rows,
            "meta": dict(self.meta),
        }


def build_matrix(
    question_ids: Iterable[str],
    models: Iterable[str],
    cell_fn,
) -> RoutingMatrix:
    """Build matrix by calling ``cell_fn(qid, model) -> Cell``."""
    qids = list(question_ids)
    mods = list(models)
    cells: dict[tuple[str, str], Cell] = {}
    for q in qids:
        for m in mods:
            cells[(q, m)] = cell_fn(q, m)
    return RoutingMatrix(question_ids=qids, models=mods, cells=cells)
