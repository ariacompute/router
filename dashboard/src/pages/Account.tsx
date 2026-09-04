import { useEffect, useState } from 'react';
import {
  getJson,
  setServeApiKey,
  syncServeAccount,
  type LocalUser,
  type ServeAccount,
} from '../api';
import styles from './page.module.css';

export default function Account() {
  const [me, setMe] = useState<LocalUser | null>(null);
  const [acct, setAcct] = useState<ServeAccount | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [keyInput, setKeyInput] = useState('');

  useEffect(() => {
    Promise.all([
      getJson<{ user: LocalUser }>('/v1/router/auth/me').then((r) => r.user),
      getJson<ServeAccount>('/v1/router/serve/account'),
    ])
      .then(([m, a]) => {
        setMe(m);
        setAcct(a);
      })
      .catch((e: Error) => setErr(e.message));

    const q = new URLSearchParams(window.location.search);
    if (q.get('error')) {
      setErr(q.get('error'));
      window.history.replaceState({}, '', '/account');
    }
  }, []);

  async function sync() {
    setBusy(true);
    setErr(null);
    try {
      setAcct(await syncServeAccount());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function saveKey() {
    const v = keyInput.trim();
    if (!v) return;
    setBusy(true);
    setErr(null);
    try {
      setAcct(await setServeApiKey(v));
      setKeyInput('');
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h1 className={styles.h1}>Account</h1>
      {err ? <p className={styles.err}>{err}</p> : null}

      {me ? (
        <>
          <h2 className={styles.h2}>Your account</h2>
          <div className={styles.grid}>
            <div className={styles.card}>
              <div className={styles.label}>Username</div>
              <div className={styles.value}>{me.username}</div>
            </div>
            <div className={styles.card}>
              <div className={styles.label}>Name</div>
              <div className={styles.value}>{me.name ?? '—'}</div>
            </div>
            <div className={styles.card}>
              <div className={styles.label}>Email</div>
              <div className={styles.value}>{me.email ?? '—'}</div>
            </div>
            <div className={styles.card}>
              <div className={styles.label}>Role</div>
              <div className={styles.value}>{me.role}</div>
            </div>
            <div className={styles.card}>
              <div className={styles.label}>Created</div>
              <div className={styles.value} style={{ fontSize: '1rem' }}>
                {new Date(me.created_at).toLocaleString()}
              </div>
            </div>
            <div className={styles.card}>
              <div className={styles.label}>Status</div>
              <div className={styles.value}>{me.disabled ? 'Disabled' : 'Active'}</div>
            </div>
          </div>
        </>
      ) : null}

      <h2 className={styles.h2}>Aria Compute connection</h2>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Status</div>
          <div className={styles.value}>{acct?.linked ? 'Connected' : 'Not connected'}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Site</div>
          <div className={styles.value}>{acct?.site_url ?? acct?.site ?? '—'}</div>
        </div>
        {acct?.linked ? (
          <div className={styles.card}>
            <div className={styles.label}>Serve account</div>
            <div className={styles.value}>{acct.user?.email ?? '—'}</div>
          </div>
        ) : null}
        <div className={styles.card}>
          <div className={styles.label}>Serve API key</div>
          {acct?.api_key_configured ? (
            <div className={styles.value}>
              <div>{acct.api_key_name ?? 'aria-router'}</div>
              <div className="muted" style={{ fontSize: '0.8rem' }}>
                {acct.api_key_prefix ?? '—'}
              </div>
            </div>
          ) : (
            <div className={styles.value}>—</div>
          )}
          <p className="muted" style={{ fontSize: '0.8rem', marginTop: '0.5rem' }}>
            Create this key yourself on Aria Compute, then paste it here once. Its
            name/prefix are auto-synced from serve.
          </p>
          <div style={{ display: 'flex', gap: '0.5rem', marginTop: '0.5rem' }}>
            <input
              type="password"
              className="input-field"
              placeholder="sk-bf-…"
              value={keyInput}
              onChange={(e) => setKeyInput(e.target.value)}
              style={{ flex: 1 }}
            />
            <button
              type="button"
              className="btn-ghost btn-sm"
              onClick={saveKey}
              disabled={busy || !acct?.linked || !keyInput.trim()}
            >
              {busy ? 'Saving…' : 'Save key'}
            </button>
          </div>
          <button
            type="button"
            className="btn-ghost btn-sm"
            onClick={sync}
            disabled={busy || !acct?.linked}
            style={{ marginTop: '0.5rem' }}
          >
            {busy ? 'Syncing…' : 'Auto-update'}
          </button>
        </div>
      </div>
    </>
  );
}
