import { useEffect, useState } from 'react';
import { getJson } from '../api';
import styles from './page.module.css';

type TierAgg = { requests: number; cost_usd: number; tokens: number };
type CompetenceCost = {
  by_tier: Record<string, TierAgg>;
  actual_cost_usd: number;
  notional_large_cost_usd: number;
  estimated_savings_usd: number;
};

function money(n: number): string {
  return `$${n.toFixed(6)}`;
}

export default function CostByCompetence() {
  const [data, setData] = useState<CompetenceCost | null>(null);
  const [err, setErr] = useState<string | null>(null);

  function load() {
    setErr(null);
    getJson<CompetenceCost>('/v1/router/cost/by-competence')
      .then(setData)
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  if (err) return <p className={styles.err}>{err}</p>;
  if (!data) return <p>Loading…</p>;

  const tiers = Object.entries(data.by_tier).sort(
    (a, b) => b[1].cost_usd - a[1].cost_usd,
  );

  return (
    <>
      <h1 className={styles.h1}>Cost by Competence</h1>
      <p>
        Spend attributed to each model competence tier. The estimated saving shows what
        competence-aware downgrades avoided versus always routing to the
        <code> large</code> tier.
      </p>
      <div className={styles.row}>
        <button type="button" className="btn-ghost" onClick={load}>
          Refresh
        </button>
      </div>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Actual cost</div>
          <div className={styles.value}>{money(data.actual_cost_usd)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Notional large cost</div>
          <div className={styles.value}>{money(data.notional_large_cost_usd)}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Est. savings</div>
          <div className={styles.value}>{money(data.estimated_savings_usd)}</div>
        </div>
      </div>

      <h2 className={styles.h2}>By tier</h2>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Tier</th>
            <th>Requests</th>
            <th>Tokens</th>
            <th>Cost</th>
          </tr>
        </thead>
        <tbody>
          {tiers.map(([tier, agg]) => (
            <tr key={tier}>
              <td>{tier}</td>
              <td>{agg.requests}</td>
              <td>{agg.tokens}</td>
              <td>{money(agg.cost_usd)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
