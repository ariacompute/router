#!/usr/bin/env bash
# Line-coverage gate for the aria-router workspace.
#
# Usage: scripts/coverage.sh [check|report|html|lcov|self-test]
#   check  (default) fail unless every instrumented line is covered
#   report           print a per-file summary (locates uncovered files)
#   html             write an HTML report to target/llvm-cov/html
#   lcov             write lcov.info at the repo root
#   self-test        run the gate helper against synthetic fixtures
#
# The predicate is fixed for every mode: --workspace --all-features with only
# build scripts exempted, so local runs and CI cannot drift apart.
#
# Why the gate parses lcov `DA:` records instead of using `llvm-cov`'s
# `--fail-under-lines`: a workspace build links a separate copy of every
# dependency crate into each test binary, and llvm-cov's summary sums line
# counts per *instantiation*. A line that is covered by `http`'s tests but not
# by `bin`'s (which links the same crate without exercising it) is therefore
# counted twice — once covered, once missed — so 100% is unreachable even when
# every instrumented line in the merged report is covered. The merged `DA:`
# records are per source line, which is what "line coverage" means here.
#
# The JSON export from `cargo llvm-cov report --json` is consulted only to skip
# lines that carry no executable code (a bare `}` closing an `if let` or a match
# arm that newer LLVM versions emit as `DA:<line>,0`). Real uncovered lines still
# fail. See docs/coverage-exclusions.md.
set -euo pipefail

cd "$(dirname "$0")/.."

# Build scripts run at compile time (npm/git), never in the shipped binary.
IGNORE_FILENAME_REGEX='(^|/)(build\.rs)$'
COV=(cargo llvm-cov --workspace --all-features
     --ignore-filename-regex "$IGNORE_FILENAME_REGEX")

command -v python3 >/dev/null 2>&1 || {
  echo "coverage.sh: python3 is required to filter no-code lines" >&2
  exit 2
}

# One instrumented test run, then both report formats from the same profile data
# so the gate and the per-file summary cannot disagree.
emit_reports() {
  "${COV[@]}" --lcov --output-path lcov.info
  cargo llvm-cov report --json --skip-functions --output-path coverage.json \
    --ignore-filename-regex "$IGNORE_FILENAME_REGEX"
}

gate() {
  python3 scripts/coverage_gate.py --lcov lcov.info --json coverage.json "$@"
}

mode="${1:-check}"
case "$mode" in
  check)
    # Keep lcov.info (and coverage.json) so a failing CI run can upload the gaps.
    emit_reports
    gate
    ;;
  report)
    emit_reports
    gate --report
    ;;
  html)
    "${COV[@]}" --html --output-dir target/llvm-cov/html
    echo "html report: target/llvm-cov/html/index.html"
    ;;
  lcov)
    emit_reports
    gate
    echo "lcov report: lcov.info"
    ;;
  self-test)
    python3 scripts/coverage_gate.py --self-test
    ;;
  *)
    echo "usage: $0 [check|report|html|lcov|self-test]" >&2
    exit 2
    ;;
esac
