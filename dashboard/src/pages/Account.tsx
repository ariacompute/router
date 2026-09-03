import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  deleteJson,
  getJson,
  sendJson,
  type ServeAccount,
} from '../api';
import styles from './page.module.css';

export default function Account() {
  const [acct, setAcct] = useState<ServeAccount | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [reveal, setReveal] = useState<string | null>(null);
  const [pasteKey, setPasteKey] = useState('');
  const [pasteName, setPasteName] = useState('dashboard');
  const [site, setSite] = useState('com');

  function load() {
    setErr(null);
    getJson<ServeAccount>('/v1/router/serve/account')
      .then(setAcct)
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(() => {
    load();
    const q = new URLSearchParams(window.location.search);
    if (q.get('linked') === '1') {
      window.history.replaceState({}, '', '/account');
    }
    if (q.get('error')) {
      setErr(q.get('error'));
      window.history.replaceState({}, '', '/account');
    }
  }, []);

  async function startOAuth() {
    setBusy(true);
    setErr(null);
    try {
      const { data } = await sendJson<{ authorize_url: string }>(
        '/v1/router/serve/link/start',
        'POST',
        { site },
      );
      window.location.href = data.authorize_url;
    } catch (e) {
      setErr((e as Error).message);
      setBusy(false);
    }
  }

  async function unlink() {
    if (!window.confirm('Unlink OAuth account and clear stored serve API key?')) return;
    setBusy(true);
    setErr(null);
    try {
      await deleteJson('/v1/router/serve/account');
      setReveal(null);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function paste() {
    setBusy(true);
    setErr(null);
    try {
      await sendJson('/v1/router/serve/api-key', 'PUT', {
        api_key: pasteKey.trim(),
        name: pasteName.trim() || 'dashboard',
        site,
      });
      setPasteKey('');
      setReveal(null);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function createAttach() {
    setBusy(true);
    setErr(null);
    try {
      await sendJson('/v1/router/serve/api-keys', 'POST', {
        name: pasteName.trim() || 'aria-router',
      });
      setReveal(null);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function doReveal() {
    if (!window.confirm('Reveal full serve API key on this management plane?')) return;
    setBusy(true);
    setErr(null);
    try {
      const data = await getJson<{ api_key?: string | null }>(
        '/v1/router/serve/account/secret',
      );
      setReveal(data.api_key ?? null);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (err && !acct) return <p className={styles.err}>{err}</p>;
  if (!acct) return <p>Loading…</p>;

  const email = acct.user?.email ?? null;
  const uid = acct.user?.id != null ? String(acct.user.id) : null;

  return (
    <>
      <h1 className={styles.h1}>OAuth (Aria Compute)</h1>
      <p>
        Cloud account on ariacompute.com / .cn. Serve API keys use <code>bfvk-…</code> (not{' '}
        <code>sk-aria_</code>). Local keys: <Link to="/keys">Local (router Dashboard) → Keys</Link>.
      </p>
      {err ? <p className={styles.err}>{err}</p> : null}

      <h2 className={styles.h1}>Link status</h2>
      <div className={styles.grid}>
        <div className={styles.card}>
          <div className={styles.label}>Status</div>
          <div className={styles.value}>{acct.status}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Site</div>
          <div className={styles.value}>{acct.site_url ?? acct.site ?? '—'}</div>
        </div>
        <div className={styles.card}>
          <div className={styles.label}>Linked at</div>
          <div className={styles.value} style={{ fontSize: '1rem' }}>
            {acct.linked_at ?? '—'}
          </div>
        </div>
      </div>

      <h2 className={styles.h1}>Serve user</h2>
      {acct.linked ? (
        <div className={styles.card} style={{ marginBottom: '1rem' }}>
          <div className={styles.label}>email</div>
          <div>{email ?? '—'}</div>
          <div className={styles.label}>user id</div>
          <div>
            <code>{uid ?? '—'}</code>
          </div>
          <div className={styles.label}>role</div>
          <div>{acct.user?.role ?? '—'}</div>
        </div>
      ) : (
        <p>
          {acct.api_key_configured
            ? 'key only, not linked — paste/CLI key without OAuth profile'
            : 'Not linked'}
        </p>
      )}

      <h2 className={styles.h1}>Serve API key</h2>
      <div className={styles.card} style={{ marginBottom: '1rem' }}>
        <div className={styles.label}>configured</div>
        <div>{acct.api_key_configured ? 'true' : 'false'}</div>
        <div className={styles.label}>name</div>
        <div>{acct.api_key_name ?? '—'}</div>
        <div className={styles.label}>prefix</div>
        <div>
          <code>{acct.api_key_prefix ?? '—'}</code>
        </div>
        {reveal ? (
          <pre className={styles.mono}>{reveal}</pre>
        ) : null}
        <div className={styles.row}>
          <button type="button" onClick={doReveal} disabled={busy || !acct.api_key_configured}>
            Reveal
          </button>
          <button
            type="button"
            disabled={busy || !reveal}
            onClick={() => reveal && navigator.clipboard.writeText(reveal)}
          >
            Copy
          </button>
        </div>
      </div>

      <h2 className={styles.h1}>Actions</h2>
      <div className={styles.row}>
        <select value={site} onChange={(e) => setSite(e.target.value)} disabled={busy}>
          <option value="com">ariacompute.com</option>
          <option value="cn">ariacompute.cn</option>
        </select>
        <button type="button" onClick={startOAuth} disabled={busy}>
          Link with OAuth
        </button>
        <button type="button" onClick={unlink} disabled={busy}>
          Unlink
        </button>
        <button type="button" onClick={createAttach} disabled={busy || !acct.linked}>
          Create &amp; attach
        </button>
        <button type="button" onClick={load} disabled={busy}>
          Refresh
        </button>
      </div>
      <div className={styles.row}>
        <input
          value={pasteName}
          onChange={(e) => setPasteName(e.target.value)}
          placeholder="key name"
          disabled={busy}
        />
        <input
          value={pasteKey}
          onChange={(e) => setPasteKey(e.target.value)}
          placeholder="Paste bfvk-… key"
          disabled={busy}
          style={{ minWidth: '16rem' }}
        />
        <button type="button" onClick={paste} disabled={busy || !pasteKey.trim()}>
          Paste key
        </button>
      </div>
    </>
  );
}
