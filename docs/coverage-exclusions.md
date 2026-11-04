# Coverage exclusions — aria-router

Target: **100% line coverage** for the whole workspace (`--workspace --all-features`),
measured with `cargo llvm-cov` (see `scripts/coverage.sh`).

Every exclusion below is file-level and applied with
`--ignore-filename-regex '(^|/)(build\.rs)$'` in one place only
(`scripts/coverage.sh`, reused verbatim by CI). Anything not listed here must be
covered by tests; PRs that add an exclusion must add a row **and** justify why the
code cannot be exercised by tests.

## Measurement: why lcov `DA:` records, not `--fail-under-lines`

A workspace build links a separate copy of every dependency crate into each test
binary (`http`'s tests link `config`, `bin` links `http`, …). `llvm-cov`'s summary
counts line coverage **per instantiation**, so a line that is exercised by one
crate's tests but only linked (never called) by another crate's tests is counted
twice: once covered, once missed. That makes a 100% summary unreachable even when
every line in the merged report is covered — the remaining "missed" lines cannot be
attributed to any source line in the annotated report (`llvm-cov show`).

`scripts/coverage.sh` therefore enforces the gate on the merged lcov `DA:` records:
one record per source line, `count == 0` means uncovered. This matches what
`llvm-cov show` annotates and what a reviewer can act on. The script prints the
merged totals (`lines: N, covered: N (100.00%)`) so the percentage is still visible.

## Exempt

| File | Kind | Reason | Testable? |
| --- | --- | --- | --- |
| `http/build.rs` | build script | Runs only at compile time: bundles the dashboard SPA / emits cargo directives. Never part of the shipped binary, so its lines are not runtime behavior. | Not in principle (compile-time only) |
| `bin/build.rs` | build script | Same as above: injects the git/version stamp for `aria-router`. | Not in principle (compile-time only) |

## Not exempt

- `http/src/serve_account.rs` is a re-export only (no executable lines); it will
  report as 0/0 lines rather than uncovered lines, so it needs no entry here.
- Network paths (provider / afm-d / agent / oauth / upgrade downloads) are covered
  by loopback fake upstreams on `127.0.0.1:0`, not excluded.
- Platform-specific branches must be restructured or `#[cfg]`-gated so that the
  compiled target's lines stay covered, instead of being excluded. Example:
  `bin/src/upgrade.rs` resolves the installed binary name through a
  `#[cfg(windows)]` / `#[cfg(not(windows))]` const pair, so only the target's
  branch exists in the binary.
- Detached/never-completing futures (test upstreams) are structured so that the
  instrumented line is a tail expression (`axum::serve(...).await`) rather than an
  `async` block's closing brace, otherwise the unreachable brace would be reported.

## How to check

```bash
scripts/coverage.sh          # 100% gate (writes lcov.info)
scripts/coverage.sh report   # per-file summary + target/llvm-cov-report.txt
scripts/coverage.sh html     # target/llvm-cov/html/index.html
```
