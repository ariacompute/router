import styles from './playground.module.css';

const PRIMARY: { key: string; label: string }[] = [
  { key: 'x-aria-router-decision', label: 'Decision' },
  { key: 'x-aria-router-algorithm', label: 'Algorithm' },
  { key: 'x-aria-router-model', label: 'Model' },
  { key: 'x-aria-router-latency-ms', label: 'Route ms' },
  { key: 'x-aria-usage-tokens', label: 'Tokens' },
];

const DETAILS: { key: string; label: string }[] = [
  { key: 'x-aria-router-layer', label: 'Layer' },
  { key: 'x-aria-router-reason', label: 'Reason' },
  { key: 'x-aria-router-confidence', label: 'Confidence' },
  { key: 'x-aria-router-bypass', label: 'Bypass' },
];

export default function RouteHeaders({ headers }: { headers: Record<string, string> }) {
  const primary = PRIMARY.filter((p) => headers[p.key]);
  const details = DETAILS.filter((p) => headers[p.key]);
  if (primary.length === 0 && details.length === 0) return null;

  return (
    <div className={styles.headers}>
      <div className={styles.headerPrimary}>
        {primary.map((p) => (
          <span key={p.key} className={styles.headerChip}>
            <span className={styles.headerLabel}>{p.label}</span>
            <span className={styles.headerValue}>{headers[p.key]}</span>
          </span>
        ))}
      </div>
      {details.length > 0 ? (
        <details className={styles.headerDetails}>
          <summary>Response details</summary>
          <div className={styles.headerDetailGrid}>
            {details.map((p) => (
              <div key={p.key} className={styles.headerDetailRow}>
                <span className={styles.headerLabel}>{p.label}</span>
                <span className={styles.headerValue}>{headers[p.key]}</span>
              </div>
            ))}
          </div>
        </details>
      ) : null}
    </div>
  );
}
