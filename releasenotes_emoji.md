# 🚀 aria-router release notes

📦 These notes cover the `T90–T99` feature set (C1–C9 / U1). The runtime contract
(hard-constraint pruning + fail-closed + the three-router isolation of semantic /
agent / afm-d) is unchanged — every addition below is opt-in and never bypasses it.

## ✨ Added

- 🔁 **Re-decision boundaries (C1)** — `core::RedecisionTracker` classifies the
  cross-request boundary (`new-user-message` / `context-compaction` /
  `tool-continuation` / `same-turn` / `unknown`). A discontinuous history
  (`unknown`) fails closed instead of reusing a frozen decision. The chosen
  boundary is observable on every response via the new `x-aria-router-redecision`
  header. Configurable under `global.services.redecision` (off by default).
- 🧠 **afm-d three-question template (C2)** — `afm-d.question_template:
  single|three`. With `three`, stage-competence / difficulty / continuation are
  asked alongside `route` and normalised into one typed `RouteDecision`; a `hard`
  difficulty promotes within the hard-filtered eligible pool, and the answers also
  fold into execution evidence.
- 🛡️ **Low-score protection (C3)** — `routing.competence.protection` reorders the
  already hard-pruned eligible pool when fresh, complete, same-category,
  same plan+rubric evidence shows a low score. State machine: trigger → resolve →
  release; old low scores do not re-trigger; `unscored != 0`; protection can never
  override location / auth / modality pruning.
- 🤝 **Trust tiers (C4)** — `global.services.trust` gates suggestions behind a
  minimum evidence count, confidence, and promote/demote thresholds (with plan /
  rubric version). New endpoints: `GET /v1/router/trust` (list drafts),
  `POST /v1/router/trust/suggest` (next suggested change; insufficient /
  low-confidence evidence → no suggestion), `POST /v1/router/trust/publish`
  (applies a draft **only** when `confirm: true`; an offline recompute that does
  **not** call `PUT /v1/router/config`). A `?estimate=true` query returns a
  heuristic offline estimate. Driven by the new ✨ **Trust Tiers** dashboard page.
- 📊 **Model performance (C5)** — `GET /v1/router/cost/model-performance` returns
  per-model / per-tier cost share, request counts, and stage competence. With no
  evidence it reports `has_evidence: false` (the dashboard shows "No data" rather
  than 0). Surfaced on the new 📊 **Model Performance** dashboard page.
- 🧾 **Execution evidence on production paths (C6)** — the production routing paths
  now emit the task_category / stage_competence / accepted_output / tool_outcomes
  / plan+rubric versions / continuation / entrypoint / redecision_boundary chain.
  JSONL is backward-compatible; when replay is off there is zero extra evidence
  computation.
- 💳 **API-key soft quota (C7)** — `global.services.quota` (default off) keeps a
  persistent cumulative spend per key and shared `group`; each request reserves,
  then settles to actual traffic (including on SSE interruption). No limit is
  enforced unless a key declares `spend_limit_usd`; the accounting never blocks.
  Exhaustion returns `QuotaExceeded` (429), distinct from rate limiting.
  `GET /v1/router/quota` exposes pending reservations and any settlement errors
  (settlement failures are recorded, never swallowed).
- 🔀 **Backend failover (C8)** — `global.services.backend_failover` (default off)
  builds a weight-ordered candidate chain from each model's `backend_refs` and
  switches to the next candidate on failure (both SSE and non-SSE paths); all
  candidates failing aggregates the error without leaking keys.
- 🧪 **Offline policy-consistency bench (C9)** — `python -m bench
  policy-consistency` recomputes four deterministic policies (cheapest /
  strongest / expected-label / by-config-expected) from saved decision records —
  no live router required. The same summary is embedded in live reports.
  Report-only, `ci_fail: false`.
- 🎨 **Dashboard redesign & audit (U1)** — new 📊 **Model Performance** and 🤝
  **Trust Tiers** pages. The dashboard is now monochrome / flat / square / AA /
  ≥12px with transform-or-opacity-only motion and a local font stack (no web-font
  CDN); design contract in [`PRODUCT.md`](PRODUCT.md). `scripts/audit-ui.sh` runs
  the impeccable-style audit in five modes (`--static` / `--dom` / `--url` /
  `--browser` / `--list-rules`); a mode that cannot run is reported as `skipped`,
  never as a pass. See the "Local rendering check (re-run)" section in
  [README.md](README.md).

## 🔧 Changed

- 💰 `GET /v1/router/cost` already reported six-factor spend; it now pairs with the
  new Model Performance endpoint and the Cost by Competence page.
- 📨 Response headers now also include `x-aria-router-redecision`.
- ⚙️ CI adds mandatory `dashboard` (build + UI guards + `--static` + `--dom`) and
  `bench` jobs. A 100% line-coverage gate (`scripts/coverage.sh`) is enforced.

## 📝 Notes

Local rendering check / re-run:

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
