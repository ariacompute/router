import { useEffect, useState } from 'react';
import { getJson, type LocalUser, type ServeAccount } from '../api';
import styles from './page.module.css';

export default function Account() {
  const [me, setMe] = useState<LocalUser | null>(null);
  const [acct, setAcct] = useState<ServeAccount | null>(null);
  const [err, setErr] = useState<string | null>(null);

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

  return (
    <>
      <h1 className={styles.h1}>Account</h1>
      <p>
        Your router dashboard identity. Sign in with <strong>Aria Compute</strong> from the login
        page to connect your serve account for LLM proxying.
      </p>
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
          <div className={styles.value}>{acct?.api_key_configured ? 'configured' : '—'}</div>
        </div>
      </div>
    </>
  );
}
