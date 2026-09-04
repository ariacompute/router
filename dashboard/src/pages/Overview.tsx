import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { getJson, type Overview as OverviewT } from '../api';
import styles from './page.module.css';

export default function Overview() {
  const [data, setData] = useState<OverviewT | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    getJson<OverviewT>('/v1/router/overview')
      .then(setData)
      .catch((e: Error) => setErr(e.message));
  }, []);

  if (err) return <p className={styles.err}>{err}</p>;
  if (!data) return <p>Loading…</p>;

  const serve = data.serve_account;

  return (
    <>
      <h1 className={styles.h1}>Overview</h1>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Health</div>
          <span className="badge badge-accent" style={{ marginTop: '0.4rem' }}>
            {data.status}
          </span>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Entrypoints</div>
          <div className={styles.value}>{data.entrypoints}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Recipes</div>
          <div className={styles.value}>{data.recipes}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Providers</div>
          <div className={styles.value}>{data.providers}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Cost (USD)</div>
          <div className={styles.value}>
            {data.cost ? `$${data.cost.cost_usd.toFixed(4)}` : '—'}
          </div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Local API keys</div>
          <div className={styles.value}>
            {data.api_keys
              ? `${data.api_keys.active} active / ${data.api_keys.revoked} revoked`
              : '—'}
          </div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>OAuth (Aria Compute)</div>
          <div className={styles.value} style={{ fontSize: '1rem' }}>
            {serve ? (
              <>
                {serve.linked ? 'linked' : serve.api_key_configured ? 'key only' : 'not linked'}
                {serve.user?.email ? ` · ${serve.user.email}` : ''}
                {serve.api_key_prefix ? ` · ${serve.api_key_prefix}` : ''}
                {' · '}
                <Link to="/account">Account</Link>
              </>
            ) : (
              '—'
            )}
          </div>
        </div>
      </div>
      <h2 className={styles.h2}>Last route</h2>
      <pre className={styles.mono}>
        {data.last_route ? JSON.stringify(data.last_route, null, 2) : 'none'}
      </pre>
    </>
  );
}
