"""Build routing_gateway_eval.json (≥210 gold-labeled prompts).

Usage:
  python -m bench build-routing-eval
  python -m bench build-routing-eval --from-mmlu ./out/mmlu.jsonl --from-draco ./out/draco.jsonl
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any, Iterable

# Upstream TokenHub ids (must match MODEL_CHECKLIST / pick-map targets).
SMALL = "qwen3.5-flash"
MID = "glm-5.3"
LARGE = "deepseek-v4-pro"

_TIER_TO_MODEL = {"small": SMALL, "mid": MID, "large": LARGE, "trap": SMALL}

_CORPUS_DIR = Path(__file__).resolve().parent / "corpus"
_DEFAULT_OUT = _CORPUS_DIR / "routing_gateway_eval.json"

# Explain / teach verbs → large (priority over systems/factoid when exclusive).
_EXPLAIN_RE = re.compile(
    r"\b(explain|walk me through|how does|how do|teach|introduce|"
    r"overview of|walk-through|walkthrough)\b",
    re.I,
)
_SYSTEMS_RE = re.compile(
    r"\b(trade-?off|reverse proxy|when would you|when to use|"
    r"design choice|cache eviction|placement)\b",
    re.I,
)
_FACTOID_RE = re.compile(
    r"\b(stand for|boiling point|what does|acronym|in celsius|"
    r"convert|capital of|emergency|arithmetic|\d+\s*[+\-×x*/]\s*\d+)\b",
    re.I,
)
_MULTI_Q_RE = re.compile(r"\?\s*.+\?", re.S)


def _item(
    *,
    id_: str,
    domain: str,
    prompt: str,
    intent: str,
) -> dict[str, Any]:
    if intent not in _TIER_TO_MODEL:
        raise ValueError(f"unknown intent {intent!r}")
    return {
        "id": id_,
        "domain": domain,
        "prompt": prompt,
        "expected_model": _TIER_TO_MODEL[intent],
        "intent": intent,
    }


def _template_items() -> list[dict[str, Any]]:
    """Deterministic ≥210 prompts covering small / mid / large / trap."""
    out: list[dict[str, Any]] = []

    # --- LARGE: explain / teach / how-it-works (~70) ---
    large_topics = [
        ("sci", "photosynthesis"),
        ("sci", "cellular respiration"),
        ("sci", "DNA replication"),
        ("sci", "natural selection"),
        ("sci", "the greenhouse effect"),
        ("sci", "osmosis"),
        ("sci", "enzyme catalysis"),
        ("sci", "the water cycle"),
        ("sci", "plate tectonics"),
        ("sci", "mitosis"),
        ("sci", "meiosis"),
        ("sci", "antibody response"),
        ("sci", "blackbody radiation"),
        ("sci", "Bernoulli's principle"),
        ("sci", "the Doppler effect"),
        ("tech", "an operating system scheduler"),
        ("tech", "TCP congestion control"),
        ("tech", "how TLS handshakes work"),
        ("tech", "how a hash table resolves collisions"),
        ("tech", "how garbage collection works"),
        ("tech", "how a B-tree index works"),
        ("tech", "how Raft consensus elects a leader"),
        ("tech", "how DNS recursive resolution works"),
        ("tech", "how a CPU pipeline works"),
        ("tech", "how virtual memory paging works"),
        ("tech", "how Kubernetes schedules pods"),
        ("tech", "how a bloom filter works"),
        ("tech", "how MapReduce shuffles data"),
        ("tech", "how WebSockets differ from HTTP polling"),
        ("tech", "how a compiler frontend parses source"),
        ("math", "the chain rule in calculus"),
        ("math", "Bayes' theorem"),
        ("math", "gradient descent"),
        ("math", "the central limit theorem"),
        ("math", "eigenvalues of a matrix"),
        ("math", "how RSA key exchange works at a high level"),
        ("math", "maximum likelihood estimation"),
        ("math", "how singular value decomposition works"),
        ("systems", "how a load balancer chooses backends"),
        ("systems", "how consistent hashing works"),
        ("systems", "how a write-ahead log provides durability"),
        ("systems", "how two-phase commit works"),
        ("systems", "how a CDN caches content"),
        ("systems", "how idempotent APIs avoid double charges"),
        ("systems", "how circuit breakers protect services"),
        ("systems", "how backpressure propagates in queues"),
    ]
    templates_large = [
        "Explain {t} in one sentence.",
        "Please explain what {t} does.",
        "Walk me through how {t} works.",
        "Teach me how {t} works briefly.",
        "How does {t} work?",
        "Introduce {t} for a beginner.",
        "Give an overview of {t}.",
        "Explain the mechanism behind {t}.",
    ]
    n = 0
    for domain, topic in large_topics:
        for tmpl in templates_large:
            if n >= 70:
                break
            prompt = tmpl.format(t=topic)
            # Avoid "how does" + systems keywords colliding oddly; topics are fine.
            out.append(
                _item(
                    id_=f"large-{n:03d}",
                    domain=domain,
                    prompt=prompt,
                    intent="large",
                )
            )
            n += 1
        if n >= 70:
            break

    # --- MID: systems / trade-off / multi-Q / placement (~70) ---
    mid_prompts = [
        ("tech", "Name one benefit of using an HTTP reverse proxy."),
        ("tech", "What is a common design trade-off when choosing a cache eviction policy?"),
        ("tech", "When would you place a reverse proxy in front of app servers?"),
        ("tech", "What is REST? What is GraphQL?"),
        ("tech", "When to use a message queue versus synchronous RPC?"),
        ("tech", "What trade-off do you accept with eventual consistency?"),
        ("tech", "Name a benefit of a reverse proxy for TLS termination."),
        ("tech", "What is a CDN? What is an origin server?"),
        ("tech", "When would you choose LRU over LFU for cache eviction?"),
        ("tech", "Design choice: monolith first or services first for a small team?"),
        ("tech", "What is gRPC? What is JSON-RPC?"),
        ("tech", "When would you place a reverse proxy as an API gateway?"),
        ("tech", "Trade-off between strong and eventual consistency in a shopping cart."),
        ("tech", "What is a sidecar proxy? What problem does it solve?"),
        ("tech", "When to use Redis versus Memcached for caching?"),
        ("tech", "Name one benefit of using an HTTP reverse proxy for rate limiting."),
        ("tech", "What is blue-green deploy? What is canary deploy?"),
        ("tech", "Cache eviction trade-off: memory vs hit rate — when prefer TTL?"),
        ("tech", "When would you place a reverse proxy for path-based routing?"),
        ("tech", "What is CQRS? What is event sourcing?"),
        ("systems", "Trade-off of sync vs async replication for a primary database."),
        ("systems", "When to use a push queue versus a pull queue?"),
        ("systems", "What is sharding? What is partitioning?"),
        ("systems", "Design choice: shared DB vs DB-per-service."),
        ("systems", "When would you place a reverse proxy for sticky sessions?"),
        ("systems", "Name one benefit of using an HTTP reverse proxy for compression."),
        ("systems", "What is a circuit breaker? What is a bulkhead?"),
        ("systems", "Trade-off between batch and streaming ETL pipelines."),
        ("systems", "When to use leader election versus gossip membership?"),
        ("systems", "What is an outbox pattern? What problem does it solve?"),
        ("systems", "Cache eviction: write-through vs write-back trade-off."),
        ("systems", "When would you place a reverse proxy for A/B routing?"),
        ("systems", "What is saga orchestration? What is choreography?"),
        ("systems", "Design choice: push vs pull metrics collection."),
        ("systems", "Trade-off of columnar vs row storage for analytics."),
        ("sci", "What is a controlled experiment? What is an observational study?"),
        ("sci", "Trade-off between sensitivity and specificity in a diagnostic test."),
        ("sci", "When to use a parametric versus nonparametric statistical test?"),
        ("sci", "What is precision? What is recall?"),
        ("sci", "Design choice: cross-sectional vs longitudinal study."),
        ("math", "What is bias? What is variance in estimation?"),
        ("math", "Trade-off between model complexity and generalization."),
        ("math", "When to use L1 versus L2 regularization?"),
        ("math", "What is precision? What is accuracy as metrics?"),
        ("math", "Design choice: batch GD vs SGD for large datasets."),
        ("tech", "Name one benefit of using an HTTP reverse proxy for authentication."),
        ("tech", "When would you place a reverse proxy for WebSocket upgrade handling?"),
        ("tech", "What is mTLS? What is plain TLS termination at the edge?"),
        ("tech", "Trade-off of client-side vs server-side rendering for SEO apps."),
        ("tech", "When to use feature flags versus long-lived branches?"),
        ("tech", "What is an API gateway? What is a service mesh?"),
        ("tech", "Cache eviction trade-off when keys are skewed (hot keys)."),
        ("tech", "When would you place a reverse proxy for geo-based routing?"),
        ("tech", "What is idempotency? What is exactly-once delivery?"),
        ("tech", "Design choice: polling vs webhooks for integrations."),
        ("systems", "Trade-off of quorum size in Raft for latency vs durability."),
        ("systems", "When to use a shared-nothing vs shared-disk architecture?"),
        ("systems", "What is a bloom filter? What false-positive trade-off do you accept?"),
        ("systems", "Name one benefit of using an HTTP reverse proxy for request buffering."),
        ("systems", "When would you place a reverse proxy for connection pooling to backends?"),
        ("systems", "What is backpressure? What is load shedding?"),
        ("systems", "Trade-off between push and pull for CDN invalidation."),
        ("systems", "Design choice: leaderful vs leaderless replication."),
        ("systems", "When to use a reverse proxy versus direct client-to-service mesh."),
        ("chat", "What is a good API error format? What is a good retry policy?"),
        ("tech", "Name a reverse proxy trade-off when enabling request body buffering."),
        ("tech", "When would you choose sticky sessions behind a reverse proxy?"),
        ("tech", "What is horizontal scaling? What is vertical scaling?"),
        ("tech", "Cache eviction: size-based vs time-based — which trade-off?"),
        ("tech", "When to use an API gateway reverse proxy for multi-tenant routing?"),
    ]
    for i, (domain, prompt) in enumerate(mid_prompts[:70]):
        out.append(_item(id_=f"mid-{i:03d}", domain=domain, prompt=prompt, intent="mid"))

    # --- SMALL: factoids / greetings / arithmetic / acronyms (~70) ---
    small_prompts = [
        ("sci", "What is the boiling point of water at sea level in Celsius?"),
        ("sci", "Convert 100C to Fahrenheit in celsius reference terms briefly."),
        ("sci", "What is the chemical symbol for gold?"),
        ("sci", "How many chromosomes do humans typically have?"),
        ("sci", "What planet is closest to the Sun?"),
        ("sci", "What is H2O?"),
        ("sci", "What is the speed of light in vacuum approximately in m/s?"),
        ("sci", "Capital of France?"),
        ("sci", "What gas do plants absorb during photosynthesis? Answer with the formula."),
        ("sci", "Freezing point of water in Celsius?"),
        ("tech", "What does CPU stand for?"),
        ("tech", "What does HTTP stand for?"),
        ("tech", "What does RAM stand for?"),
        ("tech", "What does SSD stand for?"),
        ("tech", "What does API stand for?"),
        ("tech", "What does URL stand for?"),
        ("tech", "What does DNS stand for?"),
        ("tech", "What does JSON stand for?"),
        ("tech", "What does SQL stand for?"),
        ("tech", "What does GPU stand for?"),
        ("tech", "What does TLS stand for?"),
        ("tech", "What does CDN stand for as an acronym quiz?"),
        ("math", "What is 7×8?"),
        ("math", "1+1=?"),
        ("math", "What is 12*12?"),
        ("math", "Convert 2 kilometers to meters."),
        ("math", "What is the square root of 81?"),
        ("math", "What is 15% of 200?"),
        ("math", "How many degrees in a right angle?"),
        ("math", "What is 2^10?"),
        ("chat", "hi"),
        ("chat", "hello"),
        ("chat", "hey there"),
        ("chat", "thanks"),
        ("chat", "good morning"),
        ("chat", "ok"),
        ("chat", "bye"),
        ("chat", "yes"),
        ("chat", "no"),
        ("chat", "ping"),
        ("sci", "china emergency call number?"),
        ("sci", "US emergency phone number?"),
        ("sci", "Capital of Japan?"),
        ("sci", "Atomic number of carbon?"),
        ("sci", "What does DNA stand for?"),
        ("sci", "What does RNA stand for?"),
        ("tech", "What does HTML stand for?"),
        ("tech", "What does CSS stand for?"),
        ("tech", "What does SSH stand for?"),
        ("tech", "What does VPC stand for?"),
        ("tech", "Default HTTP port number?"),
        ("tech", "Default HTTPS port number?"),
        ("tech", "What does IPv4 stand for?"),
        ("tech", "What does MAC address stand for?"),
        ("math", "What is 9+10?"),
        ("math", "Convert 1 hour to minutes."),
        ("math", "How many bits in a byte?"),
        ("math", "What is 1000 millimeters in meters?"),
        ("chat", "hi — quick fact: what does FAQ stand for?"),
        ("tech", "What does CRUD stand for?"),
        ("tech", "What does ACID stand for in databases?"),
        ("tech", "What does REST stand for?"),
        ("tech", "What does YAML stand for?"),
        ("tech", "What does CLI stand for?"),
        ("sci", "Boiling point of water in Kelvin approximately?"),
        ("sci", "What does ATP stand for in biology?"),
        ("math", "What is 5*5?"),
        ("math", "Convert 32F reference: is water frozen? yes or no"),
        ("chat", "hello — what does IDK stand for?"),
        ("tech", "What does OOM stand for in ops slang?"),
    ]
    for i, (domain, prompt) in enumerate(small_prompts[:70]):
        out.append(
            _item(id_=f"small-{i:03d}", domain=domain, prompt=prompt, intent="small")
        )

    # --- TRAPS: factoid with mid/large bait words → still small (≥15) ---
    traps = [
        ("tech", "What does the term architecture stand for as a CS acronym quiz?"),
        ("tech", "What does the word tradeoff stand for in this acronym quiz?"),
        ("tech", "In one word, what does PROXY stand for as an acronym quiz?"),
        ("tech", "What does CACHE stand for as an ops acronym quiz?"),
        ("tech", "What does SCHEDULER stand for in this acronym quiz?"),
        ("tech", "Boiling point quiz: what does OS stand for?"),
        ("tech", "What does the term reverse stand for as a networking acronym quiz?"),
        ("sci", "Explain nothing — what does DNA stand for?"),
        ("tech", "Walk me through nothing; what does HTTP stand for?"),
        ("tech", "Introduction to acronyms: what does CPU stand for?"),
        ("tech", "How to spell the expansion: what does RAM stand for?"),
        ("systems", "Overview of letters only: what does API stand for?"),
        ("tech", "Teach the acronym: what does SSD stand for?"),
        ("tech", "What does the term eviction stand for as a quiz acronym?"),
        ("tech", "Design quiz acronym: what does LRU stand for?"),
        ("math", "Explain digits only: what is 2+2?"),
        ("chat", "Please explain nothing — just say hi"),
        ("tech", "What does GRAPHQL stand for as an acronym quiz?"),
        ("tech", "Trade-off quiz letters: what does CAP stand for?"),
        ("sci", "How does spelling work for ATP — what does ATP stand for?"),
    ]
    for i, (domain, prompt) in enumerate(traps):
        out.append(_item(id_=f"trap-{i:03d}", domain=domain, prompt=prompt, intent="trap"))

    return out


def label_prompt(prompt: str, *, has_choices: bool = False) -> str:
    """Heuristic intent for imported MMLU/DRACO rows.

    Priority: explanatory (large) > systems/multi-Q (mid) > factoid/MCQ (small).
    """
    text = (prompt or "").strip()
    if not text:
        return "small"
    # Explicit explain beats everything except pure trap-style "stand for" lines.
    if _FACTOID_RE.search(text) and not _EXPLAIN_RE.search(text):
        return "small"
    if _EXPLAIN_RE.search(text):
        return "large"
    if _SYSTEMS_RE.search(text) or _MULTI_Q_RE.search(text):
        return "mid"
    if has_choices or len(text) < 80:
        return "small"
    if len(text) > 400:
        return "large"
    if len(text) > 180:
        return "mid"
    return "small"


def _load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            rows.append(json.loads(line))
    return rows


def import_mmlu(path: Path, *, limit: int = 80, prefix: str = "mmlu") -> list[dict[str, Any]]:
    rows = _load_jsonl(path)
    out: list[dict[str, Any]] = []
    for i, row in enumerate(rows):
        if len(out) >= limit:
            break
        q = row.get("question") or row.get("prompt") or ""
        if not q:
            continue
        choices = row.get("choices") or row.get("options")
        intent = label_prompt(str(q), has_choices=bool(choices))
        domain = str(row.get("category") or row.get("domain") or "mmlu")
        out.append(
            _item(
                id_=f"{prefix}-{i:04d}",
                domain=domain[:32],
                prompt=str(q).strip(),
                intent=intent if intent != "trap" else "small",
            )
        )
    return out


def import_draco(path: Path, *, limit: int = 40, prefix: str = "draco") -> list[dict[str, Any]]:
    rows = _load_jsonl(path)
    out: list[dict[str, Any]] = []
    for i, row in enumerate(rows):
        if len(out) >= limit:
            break
        q = row.get("problem") or row.get("prompt") or row.get("question") or ""
        if not q:
            continue
        intent = label_prompt(str(q), has_choices=False)
        # Long research problems skew mid/large.
        if intent == "small" and len(str(q)) > 200:
            intent = "mid"
        domain = str(row.get("domain") or "research")
        out.append(
            _item(
                id_=f"{prefix}-{i:04d}",
                domain=domain[:32],
                prompt=str(q).strip(),
                intent=intent,
            )
        )
    return out


def merge_corpus(parts: Iterable[list[dict[str, Any]]]) -> list[dict[str, Any]]:
    seen: set[str] = set()
    out: list[dict[str, Any]] = []
    for part in parts:
        for item in part:
            pid = str(item["id"])
            if pid in seen:
                continue
            seen.add(pid)
            out.append(item)
    return out


def build_corpus(
    *,
    mmlu_path: Path | None = None,
    draco_path: Path | None = None,
    mmlu_limit: int = 80,
    draco_limit: int = 40,
) -> list[dict[str, Any]]:
    parts: list[list[dict[str, Any]]] = [_template_items()]
    if mmlu_path is not None:
        parts.append(import_mmlu(mmlu_path, limit=mmlu_limit))
    if draco_path is not None:
        parts.append(import_draco(draco_path, limit=draco_limit))
    return merge_corpus(parts)


def write_corpus(items: list[dict[str, Any]], out: Path) -> dict[str, Any]:
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(items, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    counts: dict[str, int] = {}
    for it in items:
        key = str(it.get("intent") or it.get("expected_model"))
        counts[key] = counts.get(key, 0) + 1
    return {"path": str(out), "n": len(items), "by_intent": counts}


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Build routing_gateway_eval.json")
    p.add_argument("--out", type=Path, default=_DEFAULT_OUT)
    p.add_argument("--from-mmlu", type=Path, default=None)
    p.add_argument("--from-draco", type=Path, default=None)
    p.add_argument("--mmlu-limit", type=int, default=80)
    p.add_argument("--draco-limit", type=int, default=40)
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    items = build_corpus(
        mmlu_path=args.from_mmlu,
        draco_path=args.from_draco,
        mmlu_limit=args.mmlu_limit,
        draco_limit=args.draco_limit,
    )
    if len(items) < 200:
        raise SystemExit(f"expected ≥200 items, got {len(items)}")
    meta = write_corpus(items, args.out)
    print(json.dumps(meta, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
