# impeccable-style UI audit — aria-router dashboard

Baseline for `T99 / U1`. This is a **report of what actually ran**; a mode that
could not run is recorded as `skipped`, never as a pass. Design contract:
[`PRODUCT.md`](../PRODUCT.md). Engine mapping:
[`dashboard/.impeccable/config.json`](../dashboard/.impeccable/config.json).

## Tooling

| Item | Value |
|------|-------|
| Target | `dashboard/` (React 18 + Vite 5 + TypeScript, `.tsx`) |
| Test config | `dashboard/vitest.config.ts` (separate from `vite.config.ts`, so production builds do not need the vitest devDependency) |
| Tool version | `IMPECCABLE_VERSION` default `4.8.0` |
| Runner | npm / `node_modules/.bin` (the playground's bun runner does **not** apply) |
| Audit script | `scripts/audit-ui.sh` (`--static` / `--dom` / `--url` / `--browser` / `--list-rules`) |
| Static guards | `dashboard/tests/style.spec.ts` (vitest, node fs) |
| DOM guards | `dashboard/tests/render-snapshot.spec.tsx` (vitest + jsdom + @testing-library/react) |
| CI | `.github/workflows/ci.yml` → `dashboard` + `bench` jobs (build/test required) |
| Output | `AUDIT_OUT_DIR` (default `detect-out/`, gitignored) |

## Detection modes

| Mode | Engine | Browser | Exit codes |
|------|--------|---------|------------|
| `--static` | CSS + `index.html` primary rules via style guard | no | `0` clean / `2` primary findings |
| `--dom` | jsdom render of **15 routes × light/dark** | no | `0` / `2` |
| `--url` | live `AUDIT_BASE_URL` + Chrome + impeccable binary | yes | `0` / `2`; **`skipped` (0) when unavailable** |
| `--browser` | same, but a missing browser is a hard error | yes | `1` + install hint |
| `--list-rules` | prints primary/secondary rule ids and the page matrix | no | `0` |

**TSX static parsing**: not used. impeccable's static engine is HTML/CSS
oriented (the playground only maps `.vue → html`), so `.tsx`/`.ts` are mapped
to `dom`. Element-level rules run on the rendered DOM (`--dom`) and, on a
machine with Chrome, on the live page (`--url`).

Environment: `IMPECCABLE_VERSION`, `IMPECCABLE_BIN`, `IMPECCABLE_BROWSER`,
`PREVIEW_PORT`, `AUDIT_BASE_URL`, `AUDIT_ROUTES`, `AUDIT_THEMES`,
`AUDIT_OUT_DIR`, `AUDIT_PAGE_TIMEOUT`. A busy `PREVIEW_PORT` is refused rather
than silently reusing an unknown server.

## Findings → fixes → residual

| Finding | Severity | Fix | Residual |
|---------|----------|-----|----------|
| External Google Fonts (`Chakra Petch` / `Inter` / `Newsreader`) + 2 preconnects in `index.html` | primary | Removed; local `--font-sans` / `--font-display` / `--font-label` system stacks; added `color-scheme` meta | none |
| `--radius-pill: 999px` broke the square-corner contract | primary | All `--radius*` tokens are `0` | none |
| `text-transform: uppercase` / `letter-spacing` on labels, chips, badges, headings | secondary | Removed from all 4 stylesheets | none |
| Body text below 12px (`0.68rem` / `0.7rem` / `0.72rem`) | primary | Raised to `0.8125rem`; `--font-size-min: 12px` guard | none |
| Transitions animated `background`, `border-color`, `color` | primary | All transitions now use only `transform` + `opacity`; `animation` is transform-only | none |
| `--text-soft` AA contrast failure on white (`#a1a1aa`) | primary | `#52525b` in light / `#a1a1aa` in dark; guard computes WCAG ratio ≥ 4.5 for all text tokens in both themes | none |
| No gradients / elevation | primary | Verified: `--shadow*: none`, no `*-gradient` in any stylesheet | none |
| `index.html` `<html lang="en">` | reviewed | Kept: UI copy is English, there is no i18n dimension; `lang` is present and valid | intentional, not a finding |
| CI had no dashboard job; `http/build.rs` only warns on `npm build` failure | primary | Explicit `dashboard` job: `npm ci` → `build` → `test` → `--static` → `--dom`, all required | none |
| `detect-out/` not ignored | secondary | Added to `.gitignore` | none |
| No Chrome in CI | info | `--url` writes `detect-out/url-status.json` with `status: skipped` | run `--url`/`--browser` locally and paste results into this file |
| impeccable 4.8.0 binary not installed by npm | info | `IMPECCABLE_BIN` / `node_modules/.bin/impeccable` used when present; otherwise `--url` records `skipped` | install the binary on the auditing machine; no success is fabricated |

## Baseline run (2026-11-05, this workspace)

- `npm --prefix dashboard ci && npm --prefix dashboard run build` → pass.
- `npm --prefix dashboard run test` → 9 tests pass (7 style + 2 render).
- `./scripts/audit-ui.sh --static` → `passed / 0 primary findings`.
- `./scripts/audit-ui.sh --dom` → `passed / jsdom render snapshots generated`
  (15 routes × light/dark, non-empty snapshots).
- `./scripts/audit-ui.sh --url` → `skipped / no Chrome/Chromium; not a pass`.
- `./scripts/audit-ui.sh --browser` → exit `1` with install hint.
- `./scripts/audit-ui.sh --bogus` → exit `2` with usage.

## Re-run

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
npm --prefix dashboard run test
./scripts/audit-ui.sh --list-rules
./scripts/audit-ui.sh --static
./scripts/audit-ui.sh --dom
# On a machine with Chrome and the impeccable binary:
IMPECCABLE_BROWSER=/usr/bin/google-chrome ./scripts/audit-ui.sh --url
```
