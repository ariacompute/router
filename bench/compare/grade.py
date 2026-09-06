"""MCQ answer extraction and grading (VSR-style accuracy)."""

from __future__ import annotations

import re
from typing import Any


_LETTER_RE = re.compile(
    r"(?:answer\s*(?:is|=|:)?\s*|option\s+|^|\b)"
    r"[\(\[\{]?\s*([A-Ja-j])\s*[\)\]\}]?\.?\b",
    re.IGNORECASE | re.MULTILINE,
)
_YES_NO_RE = re.compile(r"\b(yes|no)\b", re.IGNORECASE)


def extract_letter(text: str) -> str | None:
    """Best-effort MCQ letter from model completion."""
    if not text:
        return None
    # Prefer last explicit "answer is X" style match
    matches = list(_LETTER_RE.finditer(text))
    if matches:
        return matches[-1].group(1).upper()
    # Single letter line
    for line in reversed(text.strip().splitlines()):
        s = line.strip().strip("()[]{}.").upper()
        if len(s) == 1 and s.isalpha() and s <= "J":
            return s
    return None


def extract_yes_no(text: str) -> str | None:
    if not text:
        return None
    matches = list(_YES_NO_RE.finditer(text))
    if matches:
        return matches[-1].group(1).lower()
    return None


def normalize_gold(answer: Any) -> str:
    if answer is None:
        return ""
    s = str(answer).strip()
    if len(s) == 1 and s.isalpha():
        return s.upper()
    low = s.lower()
    if low in ("yes", "no"):
        return low
    return s


def grade_answer(*, completion: str, gold: Any, choices: list[str] | None = None) -> dict[str, Any]:
    """Return ``{extracted, gold, is_correct}``."""
    g = normalize_gold(gold)
    extracted: str | None
    if g in ("yes", "no"):
        extracted = extract_yes_no(completion)
        ok = extracted == g if extracted else False
    elif len(g) == 1 and g.isalpha():
        extracted = extract_letter(completion)
        ok = extracted == g if extracted else False
    else:
        # Free-form / short answer: casefold substring or exact
        extracted = (completion or "").strip()
        ok = g.casefold() in extracted.casefold() if g and extracted else False
    return {
        "extracted": extracted,
        "gold": g,
        "is_correct": bool(ok),
        "choices_n": len(choices) if choices else None,
    }


def format_mcq_prompt(question: str, choices: list[str] | None) -> str:
    """Build a plain MCQ prompt (NR / no CoT), similar to VSR Router_NR."""
    lines = [question.strip(), ""]
    if choices:
        for i, c in enumerate(choices):
            letter = chr(ord("A") + i)
            # choices may already be "A. foo"
            text = c.strip()
            if text and text[0].upper() == letter and len(text) > 1 and text[1] in ".):":
                lines.append(text)
            else:
                lines.append(f"{letter}. {text}")
        lines.append("")
        lines.append("Answer with the letter of the correct option only.")
    else:
        lines.append("Answer concisely.")
    return "\n".join(lines)
