"""Write router bench JSON + Markdown reports."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


def write_json(report: dict[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def default_md_path(json_path: Path) -> Path:
    if json_path.suffix.lower() == ".json":
        return json_path.with_suffix(".md")
    return json_path.parent / (json_path.name + ".md")


def _fmt(v: Any, digits: int = 3) -> str:
    if isinstance(v, float):
        return f"{v:.{digits}f}"
    if v is None:
        return "-"
    return str(v)


def render_markdown(report: dict[str, Any]) -> str:
    mode = report.get("mode") or "unknown"
    lines: list[str] = []
    lines.append(f"# Aria Router Bench Report ({mode})")
    lines.append("")
    lines.append(f"- mode: `{mode}`")
    lines.append(f"- ci_fail: `{report.get('ci_fail')}`")
    meta = report.get("meta") or {}
    if meta:
        for k in (
            "generated_at",
            "quality",
            "corpus",
            "entrypoint",
            "eps",
            "ref_model",
            "routers",
        ):
            if k in meta and meta[k] is not None:
                lines.append(f"- {k}: `{meta[k]}`")
    lines.append("")

    summary = report.get("summary") or {}
    if summary:
        lines.append("## Summary")
        lines.append("")
        lines.append("| metric | value |")
        lines.append("|--------|-------|")
        for k, v in summary.items():
            lines.append(f"| {k} | {_fmt(v)} |")
        lines.append("")

    ladder = report.get("ladder") or []
    if ladder:
        lines.append("## Ladder")
        lines.append("")
        lines.append(
            "| policy | mean_quality | mean_cost_usd | q_per_dollar | "
            "pct_of_oracle_quality | pct_of_oracle_qd |"
        )
        lines.append(
            "|--------|--------------|---------------|--------------|"
            "---------------------|-----------------|"
        )
        for row in ladder:
            lines.append(
                "| {policy} | {mq} | {mc} | {qd} | {pq} | {pqd} |".format(
                    policy=row.get("policy"),
                    mq=_fmt(row.get("mean_quality")),
                    mc=_fmt(row.get("mean_cost_usd"), 6),
                    qd=_fmt(row.get("q_per_dollar")),
                    pq=_fmt(row.get("pct_of_oracle_quality")),
                    pqd=_fmt(row.get("pct_of_oracle_qd")),
                )
            )
        lines.append("")

    systems = report.get("systems") or []
    if systems and mode == "router_compare":
        lines.append("## Systems (accuracy / latency / tokens)")
        lines.append("")
        lines.append(
            "| system | accuracy | mean_ms | p50_ms | p95_ms | "
            "avg_completion_tokens | avg_total_tokens | Δacc vs always |"
        )
        lines.append(
            "|--------|----------|---------|--------|--------|"
            "----------------------|-----------------|----------------|"
        )
        for s in systems:
            lat = s.get("latency_ms") or {}
            lines.append(
                "| {sys} | {acc} | {mean} | {p50} | {p95} | {ct} | {tt} | {d} |".format(
                    sys=s.get("system"),
                    acc=_fmt(s.get("accuracy"), 4),
                    mean=_fmt(lat.get("mean_ms"), 1),
                    p50=_fmt(lat.get("p50_ms"), 1),
                    p95=_fmt(lat.get("p95_ms"), 1),
                    ct=_fmt(s.get("avg_completion_tokens"), 1),
                    tt=_fmt(s.get("avg_total_tokens"), 1),
                    d=_fmt(s.get("accuracy_delta_vs_best_always"), 4),
                )
            )
        lines.append("")
    elif systems:
        lines.append("## Systems")
        lines.append("")
        lines.append(
            "| system | mean_score | factual | breadth | presentation | citation | n |"
        )
        lines.append(
            "|--------|------------|---------|---------|--------------|----------|---|"
        )
        for s in systems:
            axes = s.get("axes") or {}
            lines.append(
                "| {sys} | {ms} | {fa} | {bd} | {pr} | {ci} | {n} |".format(
                    sys=s.get("system"),
                    ms=_fmt(s.get("mean_score")),
                    fa=_fmt(axes.get("factual-accuracy")),
                    bd=_fmt(axes.get("breadth-and-depth")),
                    pr=_fmt(axes.get("presentation")),
                    ci=_fmt(axes.get("citation")),
                    n=s.get("n", "-"),
                )
            )
        lines.append("")

    notes = report.get("notes") or []
    lines.append("## Notes")
    lines.append("")
    lines.append("- Report-only: missing backends or HTTP errors do not fail CI (`ci_fail: false`).")
    for n in notes:
        lines.append(f"- {n}")
    lines.append("")
    return "\n".join(lines)


def write_markdown(report: dict[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_markdown(report), encoding="utf-8")


def write_reports(
    report: dict[str, Any],
    json_path: Path,
    md_path: Path | None = None,
) -> tuple[Path, Path]:
    write_json(report, json_path)
    md = md_path or default_md_path(json_path)
    write_markdown(report, md)
    return json_path, md
