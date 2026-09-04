"""Mode B LLM-as-judge via OpenAI-compatible chat."""

from __future__ import annotations

import re
from typing import Any

from ..http_client import ChatFn, ChatResult, EndpointConfig, chat_completion


_FLOAT_RE = re.compile(r"(?<![0-9.])(0(?:\.\d+)?|1(?:\.0+)?)(?![0-9.])")


def parse_score_0_1(text: str) -> float | None:
    """Extract first 0–1 float from judge response."""
    if not text:
        return None
    m = _FLOAT_RE.search(text.strip())
    if not m:
        # try bare number anywhere
        m2 = re.search(r"([01](?:\.\d+)?)", text)
        if not m2:
            return None
        val = float(m2.group(1))
    else:
        val = float(m.group(1))
    if 0.0 <= val <= 1.0:
        return val
    return None


def judge_overall(
    *,
    prompt: str,
    completion: str,
    judge_url: str | None,
    judge_model: str = "judge",
    judge_api_key: str = "",
    timeout_s: float = 120.0,
    chat_fn: ChatFn | None = None,
) -> dict[str, Any]:
    """Ask judge for a single 0–1 quality float.

    Returns ``{status, score?, reason?}``. Without ``judge_url`` → skipped.
    """
    if not judge_url:
        return {
            "status": "skipped",
            "score": None,
            "reason": "no --judge-url; Mode B judge unavailable",
        }
    cfg = EndpointConfig(base_url=judge_url, api_key=judge_api_key, timeout_s=timeout_s)
    system = (
        "You are a strict evaluator. Reply with ONLY a single floating-point number "
        "between 0 and 1 inclusive measuring answer quality (1=perfect)."
    )
    user = f"Question:\n{prompt}\n\nAnswer:\n{completion}\n\nScore (0-1):"
    fn = chat_fn or chat_completion
    result: ChatResult = fn(
        cfg, model=judge_model, prompt=user, max_tokens=16, temperature=0.0, system=system
    )
    if result.status != "ok":
        return {
            "status": "error",
            "score": None,
            "reason": result.error or "judge chat failed",
        }
    score = parse_score_0_1(result.content)
    if score is None:
        return {
            "status": "error",
            "score": None,
            "reason": f"could not parse 0-1 score from: {result.content[:120]!r}",
        }
    return {"status": "ok", "score": score, "reason": None}


def judge_criterion(
    *,
    problem: str,
    completion: str,
    criterion: str,
    judge_url: str | None,
    judge_model: str = "judge",
    judge_api_key: str = "",
    timeout_s: float = 120.0,
    chat_fn: ChatFn | None = None,
) -> dict[str, Any]:
    """Binary MET/UNMET for one rubric criterion."""
    if not judge_url:
        return {
            "status": "skipped",
            "met": None,
            "reason": "no --judge-url; Mode B judge unavailable",
        }
    cfg = EndpointConfig(base_url=judge_url, api_key=judge_api_key, timeout_s=timeout_s)
    system = (
        "You evaluate one rubric criterion. Reply with exactly MET or UNMET."
    )
    user = (
        f"Problem:\n{problem}\n\nAnswer:\n{completion}\n\n"
        f"Criterion:\n{criterion}\n\nVerdict (MET or UNMET):"
    )
    fn = chat_fn or chat_completion
    result = fn(
        cfg, model=judge_model, prompt=user, max_tokens=8, temperature=0.0, system=system
    )
    if result.status != "ok":
        return {
            "status": "error",
            "met": None,
            "reason": result.error or "judge chat failed",
        }
    text = (result.content or "").strip().upper()
    if "MET" in text and "UNMET" not in text.replace("UNMET", ""):
        # Prefer explicit UNMET if present
        if text.startswith("UNMET") or " UNMET" in f" {text}":
            return {"status": "ok", "met": False, "reason": None}
        if text.startswith("MET") or text == "MET":
            return {"status": "ok", "met": True, "reason": None}
    if "UNMET" in text:
        return {"status": "ok", "met": False, "reason": None}
    if re.search(r"\bMET\b", text):
        return {"status": "ok", "met": True, "reason": None}
    return {
        "status": "error",
        "met": None,
        "reason": f"could not parse MET/UNMET from: {result.content[:120]!r}",
    }
