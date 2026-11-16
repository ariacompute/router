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

### Lines that hold no code

rustc 1.100 / LLVM 23 attributes each region to the line where the region
starts instead of inheriting the enclosing count, which is what LLVM 22 did.
That correction is what makes the report honest: the same tree grew from
"0 uncovered" to 127 zero-hit lines, 78 of which are real error/fallback
branches that the old toolchain had silently counted as covered. Pinning the
toolchain was therefore buying a green gate by hiding genuine misses.

The remaining 49 lines hold no executable code — a bare `}` closing an `if let`
whose implicit `else` never ran, the brace after a `match` arm that did not
match, … — so no test can cover them. The gate cross-checks `lcov.info` with
`coverage.json` (`cargo llvm-cov report --json` from the same run) and drops a
`DA:<line>,0` record only when **both** hold:

- the line's source text is nothing but delimiters (`}`, `});`, …), and
- no segment on that line recorded a hit in the JSON export.

Dropped lines are printed as `no-code lines ignored: N`, so the exclusion stays
visible. A zero-hit line that holds real code always fails the gate; the 78
branches uncovered by LLVM 23 were covered with tests, not exempted.
`scripts/coverage.sh self-test` exercises pass / real gap / executed brace /
missing input.

Do not replace the lcov line counts with the JSON `summary`/`totals`: those
count per test-binary instantiation (see above), so they cannot express the
merged per-line coverage the gate needs. Recomputing per-line counts from the
JSON segments does not work either — lcov's line attribution inherits outer
regions differently (`plugin/src/lib.rs:65` is `DA:65,1` in lcov while its only
segment is a zero-hit region entry).

### Failures that cannot be provoked in-process

A handful of error paths only trigger on a broken environment (no `$HOME` and
no passwd entry, a `reqwest` client build failure, an Argon2 failure, a zip
entry index that cannot be out of range). They are **not** excluded: the error
mapping sits in a small generic helper (`config::require_home`,
`config::embedded_template`, `http::resolve_path_or`, `http::oauth_step`,
`http::oauth_client`, `http::serve_http_client`, `http::password_hash_result`,
`provider::client_or_default`, `provider::body_or_upstream`,
`ffi::map_err_string`, `bindings::upstream`, `upgrade::zip_err`) and a unit test
drives it with an injected error, so the mapping line is genuinely executed and
asserted. Keep that shape when adding similar paths: no `unwrap`, no
`#[cfg]`-ed-out code, no per-line exclusion list.

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
scripts/coverage.sh            # 100% gate (writes lcov.info + coverage.json)
scripts/coverage.sh report     # per-file summary + no-code lines ignored
scripts/coverage.sh html       # target/llvm-cov/html/index.html
scripts/coverage.sh self-test  # gate fixtures: pass / real gap / executed brace / missing input
```

The gate needs `python3` (standard library only).
