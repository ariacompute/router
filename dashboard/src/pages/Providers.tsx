import { useEffect, useState } from 'react';
import { getJson, type ProviderRow } from '../api';
import styles from './page.module.css';

export default function Providers() {
  const [rows, setRows] = useState<ProviderRow[]>([]);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    getJson<{ models: ProviderRow[] }>('/v1/router/providers')
      .then((p) => setRows(p.models))
      .catch((e: Error) => setErr(e.message));
  }, []);

  if (err) return <p className={styles.err}>{err}</p>;

  return (
    <>
      <h1 className={styles.h1}>Providers</h1>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Name</th>
            <th>Upstream id</th>
            <th>Locality</th>
            <th>Endpoint</th>
            <th>Latency</th>
            <th>Failures</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.name}>
              <td>{r.name}</td>
              <td>{r.provider_model_id}</td>
              <td>{r.locality}</td>
              <td>{r.backend_refs.map((b) => b.endpoint).join(', ')}</td>
              <td>{r.latency_ms == null ? '—' : `${r.latency_ms.toFixed(1)} ms`}</td>
              <td>{r.failures}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
