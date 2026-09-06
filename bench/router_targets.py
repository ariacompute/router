"""Named live-router targets for multi-router bench."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Sequence

from .http_client import EndpointConfig

_DEFAULT_ARIA_PICK = "x-aria-router-model"


@dataclass
class RouterSpec:
    """One live OpenAI-compatible router under comparison."""

    name: str
    endpoint: EndpointConfig
    entrypoint: str = "aria/semantic-auto"
    pick_headers: list[str] = field(default_factory=list)

    def __post_init__(self) -> None:
        if self.pick_headers is None:
            self.pick_headers = []


def _looks_like_url(s: str) -> bool:
    return s.startswith("http://") or s.startswith("https://")


def parse_router_args(
    router_args: Sequence[str],
    *,
    entrypoint_args: Sequence[str] | None = None,
    pick_header_args: Sequence[str] | None = None,
    api_keys: dict[str, str] | None = None,
    timeout_s: float = 120.0,
    default_entrypoint: str = "aria/semantic-auto",
) -> list[RouterSpec]:
    """Parse CLI ``--router`` / ``--entrypoint`` / ``--pick-header`` lists.

    - ``--router NAME=URL`` or bare URL → name ``aria_router``
    - ``--entrypoint NAME=MODEL`` or bare string as default for all
    - ``--pick-header NAME=HEADER``; ``aria_router`` defaults to ``x-aria-router-model``
    - API key: ``router_<name>``, ``<name>``, or ``router`` (single / fallback)
    """
    keys = api_keys or {}
    specs: list[RouterSpec] = []
    for spec in router_args:
        spec = spec.strip()
        if not spec:
            continue
        if "=" in spec and not _looks_like_url(spec.split("=", 1)[0]):
            name, url = spec.split("=", 1)
            name, url = name.strip(), url.strip()
        elif _looks_like_url(spec):
            name, url = "aria_router", spec
        else:
            raise ValueError(f"router must be NAME=URL or URL, got {spec!r}")
        if not name or not url:
            raise ValueError(f"router must be NAME=URL or URL, got {spec!r}")
        key = (
            keys.get(f"router_{name}")
            or keys.get(name)
            or keys.get("router")
            or ""
        )
        specs.append(
            RouterSpec(
                name=name,
                endpoint=EndpointConfig(base_url=url, api_key=key, timeout_s=timeout_s),
                entrypoint=default_entrypoint,
                pick_headers=[],
            )
        )

    # Entrypoints
    default_ep = default_entrypoint
    named_ep: dict[str, str] = {}
    for item in entrypoint_args or []:
        item = item.strip()
        if not item:
            continue
        if "=" in item:
            k, v = item.split("=", 1)
            k, v = k.strip(), v.strip()
            if k and v and not _looks_like_url(k):
                named_ep[k] = v
                continue
        default_ep = item

    # Pick headers
    named_ph: dict[str, list[str]] = {}
    for item in pick_header_args or []:
        item = item.strip()
        if "=" not in item:
            raise ValueError(f"pick-header must be NAME=HEADER, got {item!r}")
        k, v = item.split("=", 1)
        k, v = k.strip(), v.strip()
        if not k or not v:
            raise ValueError(f"pick-header must be NAME=HEADER, got {item!r}")
        named_ph.setdefault(k, []).append(v)

    for s in specs:
        s.entrypoint = named_ep.get(s.name, default_ep)
        if s.name in named_ph:
            s.pick_headers = list(named_ph[s.name])
        elif s.name == "aria_router":
            s.pick_headers = [_DEFAULT_ARIA_PICK]
        else:
            s.pick_headers = []  # body model only

    return specs


def resolve_pick(
    picked: str | None,
    *,
    models: Sequence[str],
    alias_to_model: dict[str, str],
    pick_map: dict[str, str] | None = None,
) -> tuple[str | None, str | None]:
    """Map a raw pick into a pool model id.

    Returns ``(resolved_model_or_raw, error_or_None)``.
    """
    if not picked:
        return None, "empty pick"
    pmap = pick_map or {}
    cand = pmap.get(picked, picked)
    if cand in models:
        return cand, None
    if cand in alias_to_model:
        return alias_to_model[cand], None
    # alias whose value equals cand already handled; try reverse via values
    return cand, f"pick {picked!r} (→ {cand!r}) not in pool {list(models)}"
