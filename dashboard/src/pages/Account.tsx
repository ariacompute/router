import { useEffect, useRef, useState } from 'react';
import {
  getJson,
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

  // Auto-sync: poll the serve account so a key created/deleted/revoked on Aria
  // Compute surfaces on the dashboard without any manual paste.
  const acctRef = useRef<ServeAccount | null>(null);
  acctRef.current = acct;
  useEffect(() => {
    const id = setInterval(() => {
      if (!acctRef.current?.linked) return;
      syncServeAccount().then(setAcct).catch(() => {});
    }, 30000);
    return () => clearInterval(id);
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
      {acct?.api_key_deleted ? (
        <div
          style={{
            border: '1px solid #e0a800',
            background: '#fff8e1',
            color: '#7a5b00',
            padding: '0.75rem 1rem',
            borderRadius: '8px',
            marginBottom: '1rem',
          }}
        >
          This API key was deleted or revoked on Aria Compute. Generate a new key
          there (or re-link your Aria Compute account) and it will sync here
          automatically on the next refresh.
          {acct.api_key_prefix ? (
            <div className="muted" style={{ fontSize: '0.8rem', marginTop: '0.25rem' }}>
              Last known key: {acct.api_key_name ?? 'aria-router'} (
              {acct.api_key_prefix})
            </div>
          ) : null}
        </div>
      ) : null}
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
          {acct?.api_key_configured || acct?.api_key_deleted ? (
            <div className={styles.value}>
              <div>{acct.api_key_name ?? 'aria-router'}</div>
              <div className="muted" style={{ fontSize: '0.8rem' }}>
                {acct.api_key_prefix ?? '—'}
              </div>
              {acct.api_key_deleted ? (
                <span style={{ color: '#b00020', fontSize: '0.8rem' }}>
                  deleted on serve
                </span>
              ) : null}
            </div>
          ) : (
            <div className={styles.value}>—</div>
          )}
          <p className="muted" style={{ fontSize: '0.8rem', marginTop: '0.5rem' }}>
            {acct?.api_key_deleted
              ? 'The key above no longer exists on Aria Compute. It will sync again automatically once a valid key is present.'
              : 'Auto-synced from Aria Compute — no manual paste needed. Create or delete the key on Aria Compute and it reflects here automatically.'}
          </p>
          <button
            type="button"
            className="btn-ghost btn-sm"
            onClick={sync}
            disabled={busy || !acct?.linked}
            style={{ marginTop: '0.5rem' }}
          >
            {busy ? 'Syncing…' : 'Sync now'}
          </button>
        </div>
      </div>
    </>
  );
}
