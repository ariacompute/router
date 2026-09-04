import { useEffect, useState } from 'react';
import {
  deleteJson,
  getJson,
  sendJson,
  type KeyCreated,
  type KeyPublic,
} from '../api';
import styles from './page.module.css';

export default function Keys() {
  const [keys, setKeys] = useState<KeyPublic[]>([]);
  const [name, setName] = useState('default');
  const [created, setCreated] = useState<KeyCreated | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function load() {
    setErr(null);
    getJson<{ keys: KeyPublic[] }>('/v1/router/keys')
      .then((p) => setKeys(p.keys))
      .catch((e: Error) => setErr(e.message));
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
      <h1 className={styles.h1}>Local (router Dashboard) — API keys</h1>
      <p>
        Issue local secrets here only (<code>sk-aria_…</code>). Use the same key for data-plane chat
        Bearer and for <code>aria-engine setup</code> [1/2] / <code>--router-api-key</code>. OAuth{' '}
        <code>bfvk-</code> keys belong on <a href="/account">Account</a>, not here. Plaintext is shown
        once; disk stores hashes only.
      </p>
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
    </>
  );
}
