import { useEffect, useState } from 'react';
import { getJson, type CostReport } from '../api';
import styles from './page.module.css';

function money(n: number): string {
  return `$${n.toFixed(6)}`;
}

function bucketRows(m: Record<string, CostReport['by_model'][string]> | undefined) {
  if (!m) return [];
  return Object.entries(m).sort((a, b) => b[1].cost_usd - a[1].cost_usd);
}

export default function Cost() {
  const [data, setData] = useState<CostReport | null>(null);
  const [err, setErr] = useState<string | null>(null);

  function load() {
    setErr(null);
    getJson<CostReport>('/v1/router/cost?n=30')
      .then(setData)
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  if (err) return <p className={styles.err}>{err}</p>;
  if (!data) return <p>Loading…</p>;

  const t = data.totals;
  const f = data.factors;

  return (
    <>
      <h1 className={styles.h1}>Cost</h1>
      <p>
        Six-factor spend ≈ users × sessions/user × turns/session × requests/turn × tokens/request ×
        $/MTok. Pricing comes from YAML <code>providers.models[].pricing</code>.
      </p>
      <div className={styles.row}>
        <button type="button" onClick={load}>
          Refresh
        </button>
      </div>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Attributed cost</div>
          <div className={styles.value}>{money(t.cost_usd)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Requests</div>
          <div className={styles.value}>{t.requests}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Tokens</div>
          <div className={styles.value}>{t.tokens}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Priced fraction</div>
          <div className={styles.value}>{(t.priced_fraction * 100).toFixed(0)}%</div>
        </div>
      </div>

      <h2 className={styles.h1}>Six factors</h2>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Users</div>
          <div className={styles.value}>{f.users}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Sessions / user</div>
          <div className={styles.value}>{f.sessions_per_user.toFixed(2)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Turns / session</div>
          <div className={styles.value}>{f.turns_per_session.toFixed(2)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Requests / turn</div>
          <div className={styles.value}>{f.requests_per_turn.toFixed(2)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Tokens / request</div>
          <div className={styles.value}>{f.tokens_per_request.toFixed(1)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>$/MTok (avg)</div>
          <div className={styles.value}>{f.price_per_mtok.toFixed(4)}</div>
        </div>
      </div>
      <p className={styles.label}>
        product {money(f.product_usd)} · attributed {money(f.attributed_cost_usd)} · residual{' '}
        {money(f.residual_usd)}
      </p>

      <BucketTable title="By model" rows={bucketRows(data.by_model)} />
      <BucketTable title="By layer" rows={bucketRows(data.by_layer)} />
      <BucketTable title="By API key" rows={bucketRows(data.by_key)} />

      <h2 className={styles.h1}>Recent events</h2>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Time</th>
            <th>User / key</th>
            <th>Model</th>
            <th>Tokens</th>
            <th>Cost</th>
            <th>Source</th>
          </tr>
        </thead>
        <tbody>
          {data.recent.map((ev, i) => (
            <tr key={`${ev.ts}-${i}`}>
              <td>{ev.ts}</td>
              <td>
                {ev.user}
                {ev.key_name ? ` (${ev.key_name})` : ''}
              </td>
              <td>{ev.model}</td>
              <td>
                {ev.prompt_tokens}+{ev.completion_tokens}
              </td>
              <td>{ev.priced ? money(ev.cost_usd) : 'unpriced'}</td>
              <td>{ev.tokens_source}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

function BucketTable({
  title,
  rows,
}: {
  title: string;
  rows: [string, { requests: number; tokens: number; cost_usd: number }][];
}) {
  return (
    <>
      <h2 className={styles.h1}>{title}</h2>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Name</th>
            <th>Requests</th>
            <th>Tokens</th>
            <th>Cost</th>
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={4}>—</td>
            </tr>
          ) : (
            rows.map(([name, b]) => (
              <tr key={name}>
                <td>{name}</td>
                <td>{b.requests}</td>
                <td>{b.tokens}</td>
                <td>{money(b.cost_usd)}</td>
              </tr>
            ))
          )}
        </tbody>
      </table>
    </>
  );
}
