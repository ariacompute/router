import { useEffect, useState } from 'react';
import { getJson, sendJson } from '../api';
import styles from './page.module.css';

type Draft = {
  id: string;
  entrypoint: string;
  tier: string;
  action: string;
  confidence: number;
  sample_size: number;
  reasons: string[];
  published: boolean;
};

type Publication = {
  id: string;
  draft_id: string;
  version: number;
  tier: string;
  action: string;
  confidence: number;
};

type TrustStatus = {
  enabled: boolean;
  thresholds: {
    min_evidence: number;
    min_confidence: number;
    promote_at: number;
    demote_below: number;
    plan_version: string | null;
    rubric_version: string | null;
  };
  entrypoint: string;
  evidence_count: number;
  drafts: Draft[];
  publications: Publication[];
  published: Publication | null;
  online_config_unchanged: boolean;
};

type Suggestion = {
  status: string;
  usable: number;
  total: number;
  min_evidence: number;
  confidence: number | null;
  draft: Draft | null;
  online_routing_unchanged: boolean;
};

export default function Trust() {
  const [status, setStatus] = useState<TrustStatus | null>(null);
  const [suggestion, setSuggestion] = useState<Suggestion | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function load() {
    setErr(null);
    getJson<TrustStatus>('/v1/router/trust')
      .then(setStatus)
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  async function suggest() {
    setBusy(true);
    setErr(null);
    try {
      const { data } = await sendJson<Suggestion>('/v1/router/trust/suggest', 'POST', {});
      setSuggestion(data);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function publish(draftId: string) {
    setBusy(true);
    setErr(null);
    try {
      await sendJson('/v1/router/trust/publish', 'POST', {
        draft_id: draftId,
        confirm: true,
      });
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (err) return <p className={styles.err}>{err}</p>;
  if (!status) return <p>Loading…</p>;

  return (
    <>
      <h1 className={styles.h1}>Trust Tiers</h1>
      <p>
        Suggestions are computed from saved replay evidence only. Publishing is an
        explicit action; neither step rewrites the live router configuration.
      </p>
      {!status.enabled && (
        <p className="muted">
          Trust tiers are disabled. Enable <code>global.services.trust.enabled</code> to
          compute suggestions.
        </p>
      )}

      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Entrypoint</div>
          <div className={styles.value}>{status.entrypoint}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Evidence records</div>
          <div className={styles.value}>{status.evidence_count}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Minimum evidence</div>
          <div className={styles.value}>{status.thresholds.min_evidence}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Published tier</div>
          <div className={styles.value}>{status.published?.tier ?? 'Not published'}</div>
        </div>
      </div>

      <div className={styles.row}>
        <button type="button" className="btn-ghost" onClick={load}>
          Refresh
        </button>
        <button type="button" onClick={suggest} disabled={busy}>
          Estimate from replay
        </button>
      </div>

      {suggestion && (
        <div className={styles.card}>
          <div className={styles.label}>Last estimate</div>
          <p>
            {suggestion.status === 'ready'
              ? `Draft ${suggestion.draft?.id}: ${suggestion.draft?.tier} (${suggestion.draft?.action})`
              : `No suggestion: ${suggestion.status.replace(/_/g, ' ')}`}
          </p>
          <p className="muted">
            {suggestion.usable} of {suggestion.total} records usable; minimum is{' '}
            {suggestion.min_evidence}.
          </p>
        </div>
      )}

      <h2 className={styles.h2}>Drafts</h2>
      {status.drafts.length === 0 ? (
        <p className="muted">No drafts yet. Run an estimate after collecting evidence.</p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>ID</th>
              <th>Tier</th>
              <th>Action</th>
              <th>Confidence</th>
              <th>Samples</th>
              <th>State</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {status.drafts.map((d) => (
              <tr key={d.id}>
                <td>{d.id}</td>
                <td>{d.tier}</td>
                <td>{d.action}</td>
                <td>{d.confidence.toFixed(2)}</td>
                <td>{d.sample_size}</td>
                <td>{d.published ? 'Published' : 'Draft'}</td>
                <td>
                  {!d.published && (
                    <button
                      type="button"
                      className="btn-ghost"
                      disabled={busy}
                      onClick={() => publish(d.id)}
                    >
                      Publish
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h2 className={styles.h2}>Publications</h2>
      {status.publications.length === 0 ? (
        <p className="muted">Nothing published.</p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Version</th>
              <th>Draft</th>
              <th>Tier</th>
              <th>Action</th>
              <th>Confidence</th>
            </tr>
          </thead>
          <tbody>
            {status.publications.map((p) => (
              <tr key={p.id}>
                <td>{p.version}</td>
                <td>{p.draft_id}</td>
                <td>{p.tier}</td>
                <td>{p.action}</td>
                <td>{p.confidence.toFixed(2)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
