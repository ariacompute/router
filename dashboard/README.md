# aria-router dashboard

Ops SPA served from the management listener (`--mgmt-bind`). Pages: Overview, Config, Topology, Providers, Replay, Playground.

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
