"""CLI: ``python -m bench routing|research|compare|list-corpus|download-*``."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.request
from pathlib import Path

from .compare.runner import load_compare_corpus, run_compare
from .http_client import EndpointConfig
from .prices import load_prices
from .report import write_reports
from .research.runner import load_research_corpus, run_research
from .router_targets import parse_router_args
from .routing.runner import load_routing_corpus, run_routing

_CORPUS_DIR = Path(__file__).resolve().parent / "corpus"

_DRACO_URLS = (
    "https://huggingface.co/datasets/perplexity-ai/draco/resolve/main/test.jsonl",
    "https://huggingface.co/datasets/perplexity-ai/draco/resolve/main/data/test.jsonl",
)

# Optional helper URLs for a slim MMLU-Pro-like JSONL (may 404 → skip)
_MMLU_URLS = (
    "https://huggingface.co/datasets/TIGER-Lab/MMLU-Pro/resolve/main/data/test-00000-of-00001.parquet",
)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="python -m bench",
        description=(
            "Router bench: ADR-040 + DRACO research + MCQ compare vs vLLM Semantic Router "
            "(report-only JSON+MD; never fails CI on thresholds)"
        ),
    )
    sub = p.add_subparsers(dest="cmd", required=True)

    def add_common(sp: argparse.ArgumentParser, *, with_quality: bool = True) -> None:
        sp.add_argument(
            "--pool",
            action="append",
            default=[],
            metavar="ALIAS=URL",
            help="repeatable pool member OpenAI base URL",
        )
        sp.add_argument(
            "--model-id",
            action="append",
            default=[],
            metavar="ALIAS=MODEL",
            help="map pool alias to served model id",
        )
        sp.add_argument(
            "--router",
            action="append",
            default=[],
            metavar="NAME=URL",
            help="repeatable live router; bare URL → aria_router",
        )
        sp.add_argument(
            "--entrypoint",
            action="append",
            default=[],
            metavar="NAME=MODEL",
            help="per-router virtual model; bare string = default for all",
        )
        sp.add_argument(
            "--pick-header",
            action="append",
            default=[],
            metavar="NAME=HEADER",
            help="response header for routed model id (aria_router defaults x-aria-router-model)",
        )
        sp.add_argument(
            "--pick-map",
            action="append",
            default=[],
            metavar="FOREIGN=POOL_MODEL",
            help="map foreign routed model id into pool model id",
        )
        if with_quality:
            sp.add_argument(
                "--quality",
                choices=("label", "overlap", "judge"),
                default="label",
            )
        sp.add_argument("--corpus", required=False, help="corpus path")
        sp.add_argument("--report", default="bench_report.json", help="JSON report path")
        sp.add_argument("--report-md", default=None, help="Markdown path")
        sp.add_argument("--prices", default=None, help="JSON USD/MTok overrides")
        sp.add_argument(
            "--api-key",
            action="append",
            default=[],
            metavar="ALIAS=KEY",
            help="bearer per pool alias, router name, router_<name>, or judge",
        )
        sp.add_argument("--timeout", type=float, default=120.0)
        sp.add_argument("--max-tokens", type=int, default=256)
        sp.add_argument("--skip-probe", action="store_true")
        if with_quality:
            sp.add_argument("--ref-model", default=None, help="overlap reference model id")
            sp.add_argument("--judge-url", default=None)
            sp.add_argument("--judge-model", default="judge")
            sp.add_argument("--judge-api-key", default="")

    pr = sub.add_parser("routing", help="ADR-040 routing matrix + policy ladder")
    add_common(pr)
    pr.add_argument("--eps", type=float, default=0.03, help="oracle_cost_optimal epsilon")
    pr.set_defaults(corpus=str(_CORPUS_DIR / "routing_tiny.json"))

    pre = sub.add_parser("research", help="DRACO-shaped research rubric eval")
    add_common(pre)
    pre.set_defaults(corpus=str(_CORPUS_DIR / "research_tiny.jsonl"), max_tokens=512)

    pc = sub.add_parser(
        "compare",
        help="MCQ accuracy + latency + tokens (vs vLLM Semantic Router narrative)",
    )
    add_common(pc, with_quality=False)
    pc.set_defaults(corpus=str(_CORPUS_DIR / "mmlu_tiny.jsonl"), max_tokens=64)

    pl = sub.add_parser("list-corpus", help="List bundled tiny corpora")
    pl.add_argument("--json", action="store_true")

    pd = sub.add_parser("download-draco", help="Download perplexity-ai/draco test.jsonl")
    pd.add_argument("--out", default="draco_test.jsonl")
    pd.add_argument("--timeout", type=float, default=60.0)

    pm = sub.add_parser(
        "download-mmlu",
        help="Attempt MMLU-Pro download (skip on failure; prefer local convert)",
    )
    pm.add_argument("--out", default="mmlu_pro.jsonl")
    pm.add_argument("--timeout", type=float, default=60.0)
    return p


def _parse_kv_list(items: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for spec in items:
        if "=" not in spec:
            raise ValueError(f"expected KEY=VALUE, got {spec!r}")
        k, v = spec.split("=", 1)
        out[k.strip()] = v.strip()
    return out


def _build_pool(
    pool_args: list[str],
    keys: dict[str, str],
    timeout: float,
) -> dict[str, EndpointConfig]:
    pool: dict[str, EndpointConfig] = {}
    for spec in pool_args:
        if "=" not in spec:
            raise ValueError(f"pool must be alias=url, got {spec!r}")
        alias, url = spec.split("=", 1)
        alias, url = alias.strip(), url.strip()
        if not alias or not url:
            raise ValueError(f"pool must be alias=url, got {spec!r}")
        pool[alias] = EndpointConfig(
            base_url=url,
            api_key=keys.get(alias, ""),
            timeout_s=timeout,
        )
    return pool


def _download_urls(urls: tuple[str, ...], out: Path, timeout: float, label: str) -> int:
    last_err: str | None = None
    for url in urls:
        try:
            req = urllib.request.Request(url, method="GET")
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                data = resp.read()
            if not data:
                last_err = "empty body"
                continue
            # Reject HTML error pages / parquet binary for mmlu helper
            if label == "mmlu" and (
                data[:4] == b"PAR1" or data.lstrip().startswith(b"<")
            ):
                last_err = "got non-JSONL payload; convert locally from HF dataset"
                continue
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(data)
            print(json.dumps({"downloaded": str(out), "bytes": len(data), "url": url}))
            return 0
        except Exception as e:
            last_err = str(e)
            continue
    print(
        f"skip: could not download {label} ({last_err}); see bench/corpus/README.md",
        file=sys.stderr,
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.cmd == "list-corpus":
            rows = [
                {
                    "name": "routing_tiny.json",
                    "path": str(_CORPUS_DIR / "routing_tiny.json"),
                    "kind": "routing",
                },
                {
                    "name": "research_tiny.jsonl",
                    "path": str(_CORPUS_DIR / "research_tiny.jsonl"),
                    "kind": "research",
                },
                {
                    "name": "mmlu_tiny.jsonl",
                    "path": str(_CORPUS_DIR / "mmlu_tiny.jsonl"),
                    "kind": "compare",
                },
            ]
            if args.json:
                print(json.dumps(rows, indent=2))
            else:
                for r in rows:
                    print(f"{r['kind']}\t{r['path']}")
            return 0

        if args.cmd == "download-draco":
            return _download_urls(_DRACO_URLS, Path(args.out), args.timeout, "DRACO test.jsonl")

        if args.cmd == "download-mmlu":
            return _download_urls(_MMLU_URLS, Path(args.out), args.timeout, "mmlu")

        keys = _parse_kv_list(args.api_key)
        pool = _build_pool(args.pool, keys, args.timeout)
        model_ids = _parse_kv_list(args.model_id)
        prices = load_prices(args.prices) if args.prices else load_prices(None)
        pick_map = _parse_kv_list(args.pick_map)

        # Default entrypoint when none given
        ep_args = list(args.entrypoint) if args.entrypoint else ["aria/semantic-auto"]
        routers = parse_router_args(
            args.router,
            entrypoint_args=ep_args,
            pick_header_args=args.pick_header,
            api_keys=keys,
            timeout_s=args.timeout,
        )

        if args.cmd == "routing":
            if not pool:
                print("error: provide at least one --pool alias=url", file=sys.stderr)
                return 2
            corpus = load_routing_corpus(args.corpus)
            report = run_routing(
                corpus=corpus,
                pool=pool,
                model_ids=model_ids,
                quality=args.quality,
                routers=routers,
                pick_map=pick_map,
                ref_model=args.ref_model,
                prices=prices,
                eps=args.eps,
                max_tokens=args.max_tokens,
                skip_probe=args.skip_probe,
                judge_url=args.judge_url,
                judge_model=args.judge_model,
                judge_api_key=args.judge_api_key or keys.get("judge", ""),
            )
        elif args.cmd == "research":
            if not pool and not routers:
                print("error: provide --pool and/or --router", file=sys.stderr)
                return 2
            corpus = load_research_corpus(args.corpus)
            report = run_research(
                corpus=corpus,
                pool=pool,
                model_ids=model_ids,
                quality=args.quality,
                routers=routers,
                max_tokens=args.max_tokens,
                skip_probe=args.skip_probe,
                judge_url=args.judge_url,
                judge_model=args.judge_model,
                judge_api_key=args.judge_api_key or keys.get("judge", ""),
            )
        elif args.cmd == "compare":
            if not pool and not routers:
                print("error: provide --pool and/or --router", file=sys.stderr)
                return 2
            corpus = load_compare_corpus(args.corpus)
            report = run_compare(
                corpus=corpus,
                pool=pool,
                model_ids=model_ids,
                routers=routers,
                prices=prices,
                max_tokens=args.max_tokens,
                skip_probe=args.skip_probe,
            )
        else:
            print(f"error: unknown command {args.cmd}", file=sys.stderr)
            return 2

        json_path = Path(args.report)
        md_path = Path(args.report_md) if args.report_md else None
        jp, mp = write_reports(report, json_path, md_path)
        print(
            json.dumps(
                {
                    "report": str(jp),
                    "report_md": str(mp),
                    "summary": report.get("summary"),
                    "ci_fail": report.get("ci_fail"),
                    "mode": report.get("mode"),
                },
                indent=2,
            )
        )
        return 0
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except FileNotFoundError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except Exception as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
