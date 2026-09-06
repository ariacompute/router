import { useEffect, useMemo, useState } from 'react';
import { getJson, type Topology as TopologyT } from '../api';
import styles from './page.module.css';

const ORDER = ['entrypoint', 'recipe', 'signal', 'decision', 'algorithm', 'plugin', 'builtin', 'model'];

export default function Topology() {
  const [data, setData] = useState<TopologyT | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    getJson<TopologyT>('/v1/router/topology')
      .then(setData)
      .catch((e: Error) => setErr(e.message));
  }, []);

  const groups = useMemo(() => {
    const g = new Map<string, TopologyT['nodes']>();
    for (const kind of ORDER) g.set(kind, []);
    for (const n of data?.nodes ?? []) {
      const list = g.get(n.kind) ?? [];
      list.push(n);
      g.set(n.kind, list);
    }
    return ORDER.filter((k) => (g.get(k) ?? []).length > 0).map((k) => [k, g.get(k) ?? []] as const);
  }, [data]);

  if (err) return <p className={styles.err}>{err}</p>;
  if (!data) return <p>Loading…</p>;

  return (
    <>
      <h1 className={styles.h1}>Topology</h1>
      <p>Entrypoint → recipe → signals / decisions / algorithm / plugins or builtin agent → models.</p>
      <div className={styles.flow}>
        {groups.map(([kind, nodes]) => (
          <div key={kind} className={styles.col}>
            <div className={styles.kind}>{kind}</div>
            {nodes.map((n) => (
              <span key={n.id} className={styles.chip} title={n.id}>
                {n.label}
              </span>
            ))}
          </div>
        ))}
      </div>
      <h2 className={styles.h2}>Edges</h2>
      <pre className={styles.mono}>
        {data.edges.map((e) => `${e.from} → ${e.to}`).join('\n')}
      </pre>
    </>
  );
}
