#!/usr/bin/env bash
# Line-coverage gate for the aria-router workspace.
#
# Usage: scripts/coverage.sh [check|report|html|lcov]
#   check  (default) fail unless every instrumented line is covered
#   report           print a per-file summary (locates uncovered files)
#   html             write an HTML report to target/llvm-cov/html
#   lcov             write lcov.info at the repo root
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
# See docs/coverage-exclusions.md for the full rationale.
set -euo pipefail

cd "$(dirname "$0")/.."

# Build scripts run at compile time (npm/git), never in the shipped binary.
IGNORE_FILENAME_REGEX='(^|/)(build\.rs)$'
COV=(cargo llvm-cov --workspace --all-features
     --ignore-filename-regex "$IGNORE_FILENAME_REGEX")

# Fail when any instrumented line has zero hits. Prints the merged totals and,
# on failure, the uncovered line numbers grouped by file.
enforce_gate() {
  awk -F'[:,]' '
    /^SF:/ { file = substr($0, 4) }
    /^DA:/ {
      total++
      if ($3 + 0 == 0) {
        miss++
        gaps[file] = gaps[file] " " $2
      }
    }
    END {
      covered = total - miss
      pct = (total > 0) ? (covered * 100 / total) : 100
      printf "lines: %d, covered: %d (%.2f%%)\n", total, covered, pct
      if (miss > 0) {
        printf "uncovered lines: %d\n", miss
        for (f in gaps) printf "%s:%s\n", f, gaps[f]
        exit 1
      }
    }
  ' lcov.info
}

mode="${1:-check}"
case "$mode" in
  check)
    # Emit lcov as well so a failing CI run can upload the exact gaps.
    "${COV[@]}" --lcov --output-path lcov.info
    enforce_gate
    ;;
  report)
    # One test run, then derive both reports from the same lcov data so the
    # numbers here always agree with the gate.
    "${COV[@]}" --lcov --output-path lcov.info
    awk -F'[:,]' '
      /^SF:/ { file = substr($0, 4) }
      /^DA:/ { t[file]++; if ($3 + 0 > 0) c[file]++ }
      END {
        for (f in t) printf "%7.2f%%  %6d/%6d  %s\n", c[f] * 100 / t[f], c[f], t[f], f
      }
    ' lcov.info | sort -n
    enforce_gate
    ;;
  html)
    "${COV[@]}" --html --output-dir target/llvm-cov/html
    echo "html report: target/llvm-cov/html/index.html"
    ;;
  lcov)
    "${COV[@]}" --lcov --output-path lcov.info
    enforce_gate
    echo "lcov report: lcov.info"
    ;;
  *)
    echo "usage: $0 [check|report|html|lcov]" >&2
    exit 2
    ;;
esac
