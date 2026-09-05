import { useEffect, useState } from 'react';
import {
  deleteJson,
  getJson,
  sendJson,
  type KeyCreated,
  type KeyPublic,
  type ServeAccount,
} from '../api';
import styles from './page.module.css';

export default function Keys() {
  const [keys, setKeys] = useState<KeyPublic[]>([]);
  const [serve, setServe] = useState<ServeAccount | null>(null);
  const [name, setName] = useState('default');
  const [created, setCreated] = useState<KeyCreated | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function load() {
    setErr(null);
    getJson<{ keys: KeyPublic[] }>('/v1/router/keys')
      .then((p) => setKeys(p.keys))
      .catch((e: Error) => setErr(e.message));
    getJson<ServeAccount>('/v1/router/serve/account')
      .then(setServe)
      .catch(() => setServe(null));
  }

  useEffect(load, []);

  async function create() {
    setBusy(true);
    setErr(null);
    setCreated(null);
    try {
      const { data } = await sendJson<KeyCreated>('/v1/router/keys', 'POST', {
        name: name.trim() || 'default',
      });
      setCreated(data);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function revoke(id: string) {
    if (!window.confirm(`Revoke key ${id}?`)) return;
    setBusy(true);
    setErr(null);
    try {
      await deleteJson(`/v1/router/keys/${encodeURIComponent(id)}`);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h1 className={styles.h1}>API keys</h1>
      <div className={styles.row}>
        <input
          className="input-field"
          style={{ maxWidth: '16rem' }}
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="key name"
          disabled={busy}
        />
        <button type="button" className="btn-primary" onClick={create} disabled={busy}>
          Generate
        </button>
        <button type="button" className="btn-ghost" onClick={load} disabled={busy}>
          Refresh
        </button>
      </div>
      {err ? <p className={styles.err}>{err}</p> : null}
      {created ? (
        <div className={styles.card} style={{ marginBottom: '1rem' }}>
          <div className={styles.label}>Secret (copy now — not shown again)</div>
          <pre className={styles.mono}>{created.secret}</pre>
          <div className={styles.label}>
            id {created.id} · name {created.name} · prefix {created.prefix}
          </div>
        </div>
      ) : null}
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Name</th>
            <th>Prefix</th>
            <th>Created</th>
            <th>Last used</th>
            <th>Status</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {keys.map((k) => (
            <tr key={k.id}>
              <td>{k.name}</td>
              <td>
                <code>{k.prefix}</code>
              </td>
              <td>{k.created_at}</td>
              <td>{k.last_used_at ?? '—'}</td>
              <td>
                {k.revoked ? (
                  <span className="badge badge-err">revoked</span>
                ) : (
                  <span className="badge badge-ok">active</span>
                )}
              </td>
              <td>
                {!k.revoked ? (
                  <button
                    type="button"
                    className="btn-danger btn-sm"
                    onClick={() => revoke(k.id)}
                    disabled={busy}
                  >
                    Revoke
                  </button>
                ) : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {serve?.linked ? (
        <div style={{ marginTop: '2rem' }}>
          <h2 className={styles.h2}>Serve (Aria Compute) API key</h2>
          <p className="muted" style={{ marginBottom: '0.75rem' }}>
            Auto-synced from {serve.site_url ?? serve.site ?? 'Aria Compute'}.
          </p>
          {serve.api_key_deleted ? (
            <div
              style={{
                border: '1px solid var(--chip-amber-fg)',
                background: 'var(--chip-amber-bg)',
                color: 'var(--chip-amber-fg)',
                padding: '0.75rem 1rem',
                borderRadius: 'var(--radius-field)',
                marginBottom: '0.75rem',
              }}
            >
              This key was deleted or revoked on Aria Compute. Generate a new key
              there (or re-link your Aria Compute account) and it will sync here
              automatically. See{' '}
              <a href="/account">Account</a> for status.
            </div>
          ) : null}
          <div className={styles.card} style={{ maxWidth: '40rem' }}>
            <div className={styles.label}>Name</div>
            <div className={styles.value}>{serve.api_key_name ?? '—'}</div>
            <div className={styles.label} style={{ marginTop: '0.5rem' }}>
              Prefix
            </div>
            <div className={styles.value}>
              <code>{serve.api_key_prefix ?? '—'}</code>
            </div>
            <div className={styles.label} style={{ marginTop: '0.5rem' }}>
              Status
            </div>
            <div className={styles.value}>
              {serve.api_key_deleted ? (
                <span style={{ color: 'var(--danger)' }}>deleted on serve</span>
              ) : serve.api_key_configured ? (
                <span style={{ color: 'var(--accent-green)' }}>active</span>
              ) : (
                'not configured'
              )}
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}
