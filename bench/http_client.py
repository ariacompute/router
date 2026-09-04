"""OpenAI-compatible chat client (urllib); injectable for tests."""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping


@dataclass
class EndpointConfig:
    """Base URL + optional bearer for a pool member or router."""

    base_url: str
    api_key: str = ""
    timeout_s: float = 120.0

    def __post_init__(self) -> None:
        self.base_url = self.base_url.rstrip("/")


@dataclass
class ChatResult:
    status: str  # ok | error | skipped
    content: str = ""
    latency_ms: float = 0.0
    prompt_tokens: int | None = None
    completion_tokens: int | None = None
    total_tokens: int | None = None
    model: str | None = None
    headers: dict[str, str] = field(default_factory=dict)
    raw: dict[str, Any] = field(default_factory=dict)
    error: str | None = None

    @property
    def router_model(self) -> str | None:
        """Prefer x-aria-router-model header, else body model."""
        for k, v in self.headers.items():
            if k.lower() == "x-aria-router-model" and v:
                return v.strip()
        if self.model:
            return self.model
        return None


def _headers(cfg: EndpointConfig) -> dict[str, str]:
    h = {"Content-Type": "application/json", "Accept": "application/json"}
    if cfg.api_key:
        h["Authorization"] = f"Bearer {cfg.api_key}"
    return h


def _normalize_headers(raw: Mapping[str, str] | None) -> dict[str, str]:
    if not raw:
        return {}
    return {str(k): str(v) for k, v in raw.items()}


ChatFn = Callable[..., ChatResult]


def chat_completion(
    cfg: EndpointConfig,
    *,
    model: str,
    prompt: str,
    max_tokens: int = 256,
    temperature: float = 0.0,
    system: str | None = None,
) -> ChatResult:
    """POST /v1/chat/completions (non-streaming)."""
    messages: list[dict[str, str]] = []
    if system:
        messages.append({"role": "system", "content": system})
    messages.append({"role": "user", "content": prompt})
    body: dict[str, Any] = {
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": False,
    }
    url = f"{cfg.base_url}/v1/chat/completions"
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=_headers(cfg), method="POST")
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=cfg.timeout_s) as resp:
            raw = resp.read()
            code = resp.getcode()
            hdrs = _normalize_headers(dict(resp.headers.items()))
    except urllib.error.HTTPError as e:
        raw = e.read() if e.fp else b""
        code = e.code
        hdrs = _normalize_headers(dict(e.headers.items()) if e.headers else {})
        elapsed_ms = (time.perf_counter() - t0) * 1000.0
        return ChatResult(
            status="error",
            latency_ms=elapsed_ms,
            headers=hdrs,
            error=f"HTTP {code}: {raw[:400]!r}",
        )
    except Exception as e:
        elapsed_ms = (time.perf_counter() - t0) * 1000.0
        return ChatResult(status="error", latency_ms=elapsed_ms, error=str(e))
    elapsed_ms = (time.perf_counter() - t0) * 1000.0
    if code < 200 or code >= 300:
        return ChatResult(
            status="error",
            latency_ms=elapsed_ms,
            headers=hdrs,
            error=f"HTTP {code}: {raw[:400]!r}",
        )
    try:
        payload = json.loads(raw.decode("utf-8"))
    except json.JSONDecodeError as e:
        return ChatResult(
            status="error",
            latency_ms=elapsed_ms,
            headers=hdrs,
            error=f"bad json: {e}",
        )
    content = ""
    choices = payload.get("choices") or []
    if choices:
        msg = choices[0].get("message") or {}
        content = msg.get("content") or choices[0].get("text") or ""
        if content is None:
            content = ""
    usage = payload.get("usage") or {}
    pt = usage.get("prompt_tokens")
    ct = usage.get("completion_tokens")
    tt = usage.get("total_tokens")
    if tt is None and isinstance(pt, int) and isinstance(ct, int):
        tt = pt + ct
    return ChatResult(
        status="ok",
        content=str(content),
        latency_ms=elapsed_ms,
        prompt_tokens=pt if isinstance(pt, int) else None,
        completion_tokens=ct if isinstance(ct, int) else None,
        total_tokens=tt if isinstance(tt, int) else None,
        model=payload.get("model") or model,
        headers=hdrs,
        raw=payload,
    )


def probe_models(cfg: EndpointConfig) -> tuple[bool, str]:
    """GET /v1/models — soft readiness check."""
    url = f"{cfg.base_url}/v1/models"
    req = urllib.request.Request(url, headers=_headers(cfg), method="GET")
    try:
        with urllib.request.urlopen(req, timeout=min(cfg.timeout_s, 10.0)) as resp:
            if 200 <= resp.getcode() < 300:
                return True, "ok"
            return False, f"HTTP {resp.getcode()}"
    except Exception as e:
        return False, str(e)


def estimate_tokens(text: str, reported: int | None) -> tuple[int, str]:
    if reported is not None and reported > 0:
        return reported, "usage"
    est = max(1, len(text) // 4) if text else 0
    return est, "char_heuristic"
