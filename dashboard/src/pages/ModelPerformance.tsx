import { useEffect, useState } from 'react';
import { getJson } from '../api';
import styles from './page.module.css';

type Row = {
  name: string;
  requests: number;
  share: number;
  tokens: number;
  cost_usd: number;
  notional_cost_usd: number;
  stage_score: number | null;
  scored: number;
};

type ModelPerformance = {
  by_model: Row[];
  by_tier: Row[];
  requests: number;
  evidence_count: number;
  scored_count: number;
  has_evidence: boolean;
  estimate_note: string;
};

function money(n: number): string {
  return `$${n.toFixed(6)}`;
}

function pct(n: number): string {
  return `${(n * 100).toFixed(1)}%`;
}

function PerformanceTable({
  title,
  rows,
  hasEvidence,
}: {
  title: string;
  rows: Row[];
  hasEvidence: boolean;
}) {
  return (
    <>
      <h2 className={styles.h2}>{title}</h2>
      {rows.length === 0 ? (
        <p className={styles.muted ?? 'muted'}>No data yet. Send a request to populate this view.</p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Name</th>
              <th>Requests</th>
              <th>Share</th>
              <th>Tokens</th>
              <th>Cost</th>
              <th>Notional cost (estimate)</th>
              <th>Stage score</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.name}>
                <td>{r.name}</td>
                <td>{r.requests}</td>
                <td>{pct(r.share)}</td>
                <td>{r.tokens}</td>
                <td>{money(r.cost_usd)}</td>
                <td>{money(r.notional_cost_usd)}</td>
                <td>{hasEvidence && r.stage_score !== null ? r.stage_score.toFixed(2) : '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}

export default function ModelPerformance() {
  const [data, setData] = useState<ModelPerformance | null>(null);
  const [err, setErr] = useState<string | null>(null);

  function load() {
    setErr(null);
    getJson<ModelPerformance>('/v1/router/cost/model-performance')
      .then(setData)
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  if (err) return <p className={styles.err}>{err}</p>;
  if (!data) return <p>Loading…</p>;

  return (
    <>
      <h1 className={styles.h1}>Model Performance</h1>
      <p>
        Per-model and per-tier traffic share, spend and stage scores. Stage scores come
        from saved execution evidence only — no score is inferred when none exists.
      </p>
      <div className={styles.row}>
        <button type="button" className="btn-ghost" onClick={load}>
          Refresh
        </button>
      </div>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Requests recorded</div>
          <div className={styles.value}>{data.requests}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Evidence records</div>
          <div className={styles.value}>{data.evidence_count}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Stage scores</div>
          <div className={styles.value}>{data.has_evidence ? data.scored_count : 'No data'}</div>
        </div>
      </div>
      <p className="muted">{data.estimate_note}</p>

      <PerformanceTable
        title="By model"
        rows={data.by_model}
        hasEvidence={data.has_evidence}
      />
      <PerformanceTable
        title="By competence tier"
        rows={data.by_tier}
        hasEvidence={data.has_evidence}
      />
    </>
  );
}
