# aria-router dashboard

Ops SPA served from the management listener (`--mgmt-bind`). Pages: Overview, Config, Topology, Providers, Replay, Playground.

**Playground** (aligned with vLLM SR core chat UX): multi-turn bubbles, model dropdown (`GET /v1/router/models`), SSE streaming via `POST /v1/router/chat`, routing `x-aria-router-*` header panel, markdown replies, localStorage conversation sidebar. No MCP / Claw / web search / attachments.

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

Does not embed Grafana, ML setup, or security policy.
