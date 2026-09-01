import { useEffect, useState } from 'react';
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

  return (
    <>
      <h1 className={styles.h1}>Overview</h1>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Health</div>
          <div className={styles.value}>{data.status}</div>
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
      </div>
      <h2 className={styles.h1}>Last route</h2>
      <pre className={styles.mono}>
        {data.last_route ? JSON.stringify(data.last_route, null, 2) : 'none'}
      </pre>
    </>
  );
}
