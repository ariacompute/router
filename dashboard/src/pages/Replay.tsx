import { useEffect, useState } from 'react';
import { getJson, type RouteDecision } from '../api';
import styles from './page.module.css';

type ReplayRecord = {
  id: string;
  decision: RouteDecision;
  session?: string | null;
  retention?: {
    drop?: boolean | null;
    ttl_turns?: number | null;
    keep_current_model?: boolean | null;
  } | null;
  prompt_preview?: string | null;
};

export default function Replay() {
  const [items, setItems] = useState<RouteDecision[]>([]);
  const [records, setRecords] = useState<ReplayRecord[]>([]);
  const [err, setErr] = useState<string | null>(null);

  function load() {
    getJson<{ items: RouteDecision[]; records?: ReplayRecord[] }>('/v1/router/replay?n=50')
      .then((p) => {
        setItems(p.items ?? []);
        setRecords(p.records ?? []);
      })
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
            <th>Retention</th>
            <th>Reason</th>
          </tr>
        </thead>
        <tbody>
          {items.map((d, i) => {
            const rec = records[i];
            const ret = d.retention_keep_current_model
              ? `sticky ttl=${d.retention_ttl_turns ?? '—'}`
              : rec?.retention?.keep_current_model
                ? `sticky ttl=${rec.retention?.ttl_turns ?? '—'}`
                : '—';
            return (
              <tr key={`${d.model}-${i}`}>
                <td>{d.layer}</td>
                <td>{d.decision}</td>
                <td>{d.model}</td>
                <td>{ret}</td>
                <td>{d.reason}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}
