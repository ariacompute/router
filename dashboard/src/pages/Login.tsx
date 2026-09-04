import { FormEvent, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { sendJson, setSessionToken, startAriaOAuth, type LocalUser } from '../api';

export default function Login() {
  const nav = useNavigate();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [oauthBusy, setOauthBusy] = useState(false);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      const { data } = await sendJson<{ user: LocalUser; token: string }>(
        '/v1/router/auth/login',
        'POST',
        { username, password },
      );
      setSessionToken(data.token);
      nav('/');
    } catch (ex) {
      setErr((ex as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function onOAuth() {
    setOauthBusy(true);
    setErr(null);
    try {
      const { authorize_url } = await startAriaOAuth();
      window.location.href = authorize_url;
    } catch (ex) {
      setErr((ex as Error).message);
      setOauthBusy(false);
    }
  }

  return (
    <div className="glass-card" style={{ width: '100%', maxWidth: '26rem', padding: '2rem' }}>
      <h1 className="h1" style={{ marginBottom: '0.4rem' }}>
        Sign in
      </h1>
      <form onSubmit={onSubmit} className="stack" style={{ gap: '1rem' }}>
        <div className="stack" style={{ gap: '0.4rem' }}>
          <label className="stat-label" htmlFor="username">
            Username
          </label>
          <input
            id="username"
            className="input-field"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="username"
            autoComplete="username"
            disabled={busy}
          />
        </div>
        <div className="stack" style={{ gap: '0.4rem' }}>
          <label className="stat-label" htmlFor="password">
            Password
          </label>
          <input
            type="password"
            id="password"
            className="input-field"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="password"
            autoComplete="current-password"
            disabled={busy}
          />
        </div>
        <button
          type="submit"
          className="btn-primary"
          disabled={busy || !username || !password}
          style={{ width: '100%' }}
        >
          Sign in
        </button>
      </form>

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '0.75rem',
          margin: '1.5rem 0',
        }}
      >
        <div style={{ flex: 1, height: 1, background: 'var(--line)' }} />
        <span className="muted" style={{ fontSize: '0.8rem' }}>
          or
        </span>
        <div style={{ flex: 1, height: 1, background: 'var(--line)' }} />
      </div>

      <button
        type="button"
        className="btn-primary"
        onClick={onOAuth}
        disabled={oauthBusy}
        style={{ width: '100%' }}
      >
        {oauthBusy ? 'Redirecting…' : 'Sign in with Aria Compute'}
      </button>

      {err ? (
        <p className="alert alert-err" style={{ marginTop: '1rem' }}>
          {err}
        </p>
      ) : null}
      <p className="muted" style={{ marginTop: '1.5rem', textAlign: 'center' }}>
        No account?{' '}
        <Link to="/register" style={{ color: 'var(--accent)', fontWeight: 600 }}>
          Register
        </Link>
      </p>
    </div>
  );
}
