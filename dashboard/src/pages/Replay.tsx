import { useEffect, useState } from 'react';
import { getJson, type RouteDecision } from '../api';
import styles from './page.module.css';

export default function Replay() {
  const [items, setItems] = useState<RouteDecision[]>([]);
  const [err, setErr] = useState<string | null>(null);

  function load() {
    getJson<{ items: RouteDecision[] }>('/v1/router/replay?n=50')
      .then((p) => setItems(p.items ?? []))
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  if (err) return <p className={styles.err}>{err}</p>;

  return (
    <>
      <h1 className={styles.h1}>Replay</h1>
      <div className={styles.row}>
        <button type="button" className="btn-ghost" onClick={load}>
          Refresh
        </button>
        <span className="muted">{items.length} decisions</span>
      </div>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Layer</th>
            <th>Decision</th>
            <th>Model</th>
            <th>Reason</th>
          </tr>
        </thead>
        <tbody>
          {items.map((d, i) => (
            <tr key={`${d.model}-${i}`}>
              <td>{d.layer}</td>
              <td>{d.decision}</td>
              <td>{d.model}</td>
              <td>{d.reason}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
