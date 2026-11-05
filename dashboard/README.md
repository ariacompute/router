# aria-router dashboard

Ops SPA served from the management listener (`--mgmt-bind`). Pages: Overview, Config, Topology, Providers, Replay, Playground, Cost, Cost by Competence, Model Performance, Trust Tiers, Keys, Users, Account, Login, Register.

**Design contract**: [`../PRODUCT.md`](../PRODUCT.md) — monochrome, square, flat, ≥12px, AA contrast, transform/opacity-only motion, local fonts, no web-font CDN. Routing headers now include `x-aria-router-redecision` (explicit cross-request boundary).

**Playground** (aligned with vLLM SR core chat UX): multi-turn bubbles, model dropdown (`GET /v1/router/models`), SSE streaming via `POST /v1/router/chat`, routing header chips (`decision` / `algorithm` / `model` / **Route ms** from `x-aria-router-latency-ms` / **Tokens** from SSE `usage` when present), markdown replies, localStorage conversation sidebar. No MCP / Claw / web search / attachments.

**Capability surface vs vLLM SR keyword baseline**

| Capability | aria-router | vLLM SR (bench baseline) |
|------------|-------------|---------------------------|
| No Envoy ExtProc | yes (native HTTP) | Envoy data plane |
| Builtin agent entrypoint | `agent-gateway` | not in frozen config |
| Projection-conditioned decisions | `type: projection` | keyword-only |
| Model pricing + Cost ledger | yes | not in frozen config |
| Route latency header | `x-aria-router-latency-ms` | — |
| Replay / Topology | dashboard | upstream dashboard differs |

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --mgmt-bind 127.0.0.1:8090
```

Dev (proxy to a running serve):

```bash
npm --prefix dashboard run dev
```

UI regression guards and impeccable-style audit (see [`../docs/impeccable-audit.md`](../docs/impeccable-audit.md)):

```bash
npm --prefix dashboard run test        # style tokens + 15 routes x light/dark jsdom snapshots
npm --prefix dashboard run audit       # --static
npm --prefix dashboard run audit:dom   # --dom
npm --prefix dashboard run audit:url   # --url (records skipped without Chrome)
npm --prefix dashboard run audit:rules # rule list + page matrix
```

Does not embed Grafana, ML setup, or security policy.
