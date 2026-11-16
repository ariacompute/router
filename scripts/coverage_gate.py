#!/usr/bin/env python3
"""Line-coverage gate for the aria-router workspace.

Two reports produced by the same `cargo llvm-cov` run feed this gate:

* ``lcov.info`` from ``cargo llvm-cov --lcov``. Its ``DA:`` records merge hit
  counts per source line; the console summary instead counts lines per
  test-binary instantiation, so 100% is unreachable there.
* ``coverage.json`` from ``cargo llvm-cov report --json``. Only the JSON export
  exposes per-line segments, which prove that nothing on a line executed.

rustc 1.100 / LLVM 23 attributes each region to the line where the region
starts instead of inheriting the enclosing count (the LLVM 22 behaviour). That
correction is what makes the report honest, but it also emits ``DA:<line>,0``
for lines that carry no executable code at all — the closing brace of an ``if
let`` whose implicit ``else`` never ran, the closing brace of a ``match`` arm
that never matched, and so on. Such a line cannot be covered by any test.

A zero-hit line is therefore dropped from the gate only when both hold:

* its source text holds nothing but delimiters (``}``, ``});``, ...), i.e. there
  is no expression to execute, and
* no segment on that line in ``coverage.json`` recorded a hit.

Dropped lines are counted and printed as ``no-code lines ignored: N`` so the
exclusion is visible. Every line that holds real code still fails when it is
not executed.

A source path that lcov names but this process cannot read (a widened
``--ignore-filename-regex`` leaking registry/rustc paths, a file removed
mid-run) is never a reason to abort: its zero-hit lines are treated as code, so
they surface as a real gap with a diagnostic on stderr. Exit codes: 0 = every
executable line is covered; 1 = real uncovered lines; 2 = missing/unparseable
input (fail closed, never a silent pass).
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import sys
import tempfile
from pathlib import Path

# A line whose stripped text is a subset of these characters holds no
# expression: only the delimiters that close the surrounding block or call.
DELIMITERS = frozenset("{}()[];,")


def read_lcov(path: Path) -> list[tuple[str, int, int]]:
    """Return ``(file, line, hit_count)`` for every lcov ``DA:`` record."""
    records: list[tuple[str, int, int]] = []
    current: str | None = None
    with path.open(encoding="utf-8") as handle:
        for raw in handle:
            line = raw.rstrip("\n")
            if line.startswith("SF:"):
                current = line[3:]
            elif line.startswith("DA:"):
                if current is None:
                    raise ValueError("lcov DA record before any SF record")
                fields = line[3:].split(",")
                if len(fields) < 2:
                    raise ValueError(f"malformed lcov DA record: {line!r}")
                records.append((current, int(fields[0]), int(fields[1])))
    return records


def read_segments(path: Path) -> tuple[dict[str, dict[int, list[int]]], set[str]]:
    """Map each file/line to the hit counts of its segments.

    The segment layout is ``[line, col, count, hasCount, isRegionEntry,
    isGapRegion]``; only the line and its count are needed here, because a line
    that no segment executed is the precondition for treating it as no-code.
    """
    with path.open(encoding="utf-8") as handle:
        document = json.load(handle)
    entries = document.get("data")
    if not isinstance(entries, list) or not entries:
        raise ValueError("coverage.json has no data entries")

    by_file: dict[str, dict[int, list[int]]] = {}
    for entry in entries:
        for item in entry.get("files") or []:
            name = item.get("filename")
            if not name:
                raise ValueError("coverage.json file entry without a filename")
            by_line: dict[int, list[int]] = {}
            for segment in item.get("segments") or []:
                if len(segment) < 5:
                    raise ValueError(f"malformed coverage segment: {segment!r}")
                by_line.setdefault(int(segment[0]), []).append(int(segment[2]))
            by_file[name] = by_line
    if not by_file:
        raise ValueError("coverage.json describes no files")
    return by_file, set(by_file)


def is_no_code(text: str) -> bool:
    """True when the line holds only delimiters, so no test could execute it."""
    body = text.split("//", 1)[0].strip()
    return bool(body) and set(body) <= DELIMITERS


def read_source(name: str) -> list[str] | None:
    """Return the source lines of ``name``, or ``None`` when it is unreadable.

    A path the gate cannot read says nothing about whether the line holds code,
    so the caller must not treat it as a no-code line: failing closed on a
    missing file is what keeps a widened filename filter from silently passing.
    """
    try:
        return Path(name).read_text(encoding="utf-8").split("\n")
    except (OSError, UnicodeDecodeError):
        return None


def no_code_lines(
    records: list[tuple[str, int, int]], by_file: dict[str, dict[int, list[int]]]
) -> set[tuple[str, int]]:
    """Collect the zero-hit ``(file, line)`` pairs that hold no executable code."""
    sources: dict[str, list[str] | None] = {}
    unreadable: set[str] = set()
    ignored: set[tuple[str, int]] = set()
    for name, line, count in records:
        if count != 0 or (name, line) in ignored:
            continue
        lines = by_file.get(name)
        if lines is None:
            raise ValueError(f"{name} is missing from coverage.json")
        segments = lines.get(line)
        if segments is None:
            continue
        if any(count > 0 for count in segments):
            continue
        if name not in sources:
            # A negative cache: an unreadable file is probed once, not per line.
            sources[name] = read_source(name)
            if sources[name] is None:
                unreadable.add(name)
        source = sources[name]
        if source is None:
            continue
        if line - 1 < len(source) and is_no_code(source[line - 1]):
            ignored.add((name, line))
    for name in sorted(unreadable):
        print(
            f"coverage_gate: cannot read {name}; treating its zero-hit lines as code",
            file=sys.stderr,
        )
    return ignored


def summarise(
    records: list[tuple[str, int, int]], ignored: set[tuple[str, int]]
) -> tuple[int, int, int, dict[str, list[int]]]:
    """Fold lcov records into totals, skipping the lines that hold no code."""
    total = covered = skipped = 0
    misses: dict[str, list[int]] = {}
    for name, line, count in records:
        if (name, line) in ignored:
            skipped += 1
            continue
        total += 1
        if count > 0:
            covered += 1
        else:
            misses.setdefault(name, []).append(line)
    return total, covered, skipped, misses


def emit_report(records: list[tuple[str, int, int]], ignored: set[tuple[str, int]]) -> None:
    """Print a per-file summary using the same predicate as the gate."""
    per_file: dict[str, list[int]] = {}
    for name, line, count in records:
        if (name, line) in ignored:
            continue
        stats = per_file.setdefault(name, [0, 0])
        stats[0] += 1
        if count > 0:
            stats[1] += 1
    for name in sorted(per_file):
        total, covered = per_file[name]
        print(f"{covered * 100 / total:7.2f}%  {covered:6d}/{total:6d}  {name}")


def emit_summary(
    total: int, covered: int, skipped: int, misses: dict[str, list[int]]
) -> int:
    """Print totals and, on failure, the uncovered lines grouped by file."""
    pct = covered * 100 / total if total else 0.0
    print(f"lines: {total}, covered: {covered} ({pct:.2f}%)")
    print(f"no-code lines ignored: {skipped}")
    if not misses:
        return 0
    print(f"uncovered lines: {sum(len(lines) for lines in misses.values())}")
    for name in sorted(misses):
        line_list = " ".join(str(n) for n in sorted(misses[name]))
        print(f"{name}: {line_list}")
    return 1


def run_gate(lcov: Path, json_path: Path, show_report: bool) -> int:
    """Evaluate the gate. Raises on bad input so callers can fail closed."""
    records = read_lcov(lcov)
    if not records:
        raise ValueError("no instrumented lines found in lcov")
    by_file, json_files = read_segments(json_path)
    lcov_files = {name for name, _, _ in records}
    if not lcov_files & json_files:
        raise ValueError("lcov and coverage.json describe disjoint file sets")
    missing = lcov_files - json_files
    if missing:
        raise ValueError(f"coverage.json is missing {len(missing)} lcov file(s)")
    ignored = no_code_lines(records, by_file)
    if show_report:
        emit_report(records, ignored)
    total, covered, skipped, misses = summarise(records, ignored)
    if total == 0:
        raise ValueError("no instrumented lines left after filtering")
    return emit_summary(total, covered, skipped, misses)


def self_test() -> int:
    """Exercise the meaningful outcomes against synthetic fixtures."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        source = root / "lib.rs"
        lcov = root / "lcov.info"
        coverage = root / "coverage.json"

        # Line 2 is a bare closing brace: no code. Lines 3 and 4 hold code.
        source.write_text("fn f() {\n}\nlet a = g();\nlet b = h();\n", encoding="utf-8")

        def write_lcov(*counts: int) -> None:
            body = "".join(f"DA:{n},{c}\n" for n, c in enumerate(counts, start=1))
            lcov.write_text(f"SF:{source}\n{body}end_of_record\n", encoding="utf-8")

        def write_json(*counts: int) -> None:
            segments = [
                [n, 1, c, True, True, False] for n, c in enumerate(counts, start=1)
            ]
            coverage.write_text(
                json.dumps({"data": [{"files": [{"filename": str(source), "segments": segments}]}]}),
                encoding="utf-8",
            )

        outcomes: list[tuple[str, int]] = []

        # Line 2 is a no-code brace, lines 3 and 4 are uncovered -> fail.
        write_lcov(1, 0, 0, 0)
        write_json(1, 0, 0, 0)
        with contextlib.redirect_stdout(io.StringIO()):
            outcomes.append(("real-gap", run_gate(lcov, coverage, False)))

        # Everything covered except the no-code brace -> pass, brace counted.
        write_lcov(1, 0, 1, 1)
        write_json(1, 0, 1, 1)
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            outcomes.append(("all-covered", run_gate(lcov, coverage, False)))
        counted = "no-code lines ignored: 1" in buffer.getvalue()

        # A bare brace that *did* record a hit is never treated as no-code.
        write_lcov(1, 1, 0, 0)
        write_json(1, 1, 0, 0)
        with contextlib.redirect_stdout(io.StringIO()):
            outcomes.append(("executed-brace", run_gate(lcov, coverage, False)))

        # Missing JSON -> fail closed.
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                run_gate(lcov, root / "missing.json", False)
            missing_code = 0
        except (OSError, ValueError, json.JSONDecodeError):
            missing_code = 2
        outcomes.append(("missing-input", missing_code))

        # An unreadable source path is *not* a no-code line: the zero-hit line
        # is reported as a real gap (exit 1) with a diagnostic, never as bad
        # input (exit 2) and never silently ignored.
        unreadable = root / "missing.rs"
        lcov.write_text(f"SF:{unreadable}\nDA:1,0\nend_of_record\n", encoding="utf-8")
        coverage.write_text(
            json.dumps(
                {
                    "data": [
                        {
                            "files": [
                                {
                                    "filename": str(unreadable),
                                    "segments": [[1, 1, 0, True, True, False]],
                                }
                            ]
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        errors = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(errors):
            outcomes.append(("unreadable-source", run_gate(lcov, coverage, False)))
        warned = str(unreadable) in errors.getvalue()

        expected = [
            ("real-gap", 1),
            ("all-covered", 0),
            ("executed-brace", 1),
            ("missing-input", 2),
            ("unreadable-source", 1),
        ]
        ok = outcomes == expected and counted and warned
        print(
            "self-test:",
            "ok" if ok else f"FAILED outcomes={outcomes!r} counted={counted} warned={warned}",
        )
        return 0 if ok else 2


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lcov", type=Path, help="lcov tracefile")
    parser.add_argument("--json", type=Path, dest="json_path", help="llvm-cov JSON export")
    parser.add_argument(
        "--report",
        dest="show_report",
        action="store_true",
        help="print a per-file summary before the totals",
    )
    parser.add_argument("--self-test", action="store_true", help="run synthetic fixtures")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    if not args.lcov or not args.json_path:
        parser.error("--lcov and --json are required unless --self-test is used")
    try:
        return run_gate(args.lcov, args.json_path, args.show_report)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"coverage_gate: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
