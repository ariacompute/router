import { FormEvent, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import {
  getJson,
  sendJson,
  setSessionToken,
  type LocalUser,
  type RegisterStatus,
} from '../api';

export default function Register() {
  const nav = useNavigate();
  const [status, setStatus] = useState<RegisterStatus | null>(null);
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    getJson<RegisterStatus>('/v1/router/auth/register-status')
      .then(setStatus)
      .catch((e: Error) => setErr(e.message));
  }, []);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      const { data } = await sendJson<{ user: LocalUser; token: string }>(
        '/v1/router/auth/register',
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

  if (status?.needs_setup) {
    return (
      <div className="glass-card" style={{ width: '100%', maxWidth: '26rem', padding: '2rem' }}>
        <h1 className="h1" style={{ marginBottom: '0.5rem' }}>
          Register
        </h1>
        <p className="alert alert-err">
          No local users yet. Run <code>aria-router setup</code> to create the first admin.
        </p>
        <p className="muted" style={{ marginTop: '1.25rem', textAlign: 'center' }}>
          <Link to="/login" style={{ color: 'var(--accent)', fontWeight: 600 }}>
            Login
          </Link>
        </p>
      </div>
    );
  }

  if (status && !status.allow_register) {
    return (
      <div className="glass-card" style={{ width: '100%', maxWidth: '26rem', padding: '2rem' }}>
        <h1 className="h1" style={{ marginBottom: '0.5rem' }}>
          Register
        </h1>
        <p>Self-registration is disabled. Ask an admin or use Login.</p>
        <p className="muted" style={{ marginTop: '1.25rem', textAlign: 'center' }}>
          <Link to="/login" style={{ color: 'var(--accent)', fontWeight: 600 }}>
            Login
          </Link>
        </p>
      </div>
    );
  }

  return (
    <div className="glass-card" style={{ width: '100%', maxWidth: '26rem', padding: '2rem' }}>
      <h1 className="h1" style={{ marginBottom: '0.4rem' }}>
        Create account
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
            placeholder="password (min 8)"
            autoComplete="new-password"
            disabled={busy}
          />
        </div>
        <button
          type="submit"
          className="btn-primary"
          disabled={busy || !username || password.length < 8}
          style={{ width: '100%' }}
        >
          Register
        </button>
      </form>
      {err ? (
        <p className="alert alert-err" style={{ marginTop: '1rem' }}>
          {err}
        </p>
      ) : null}
      <p className="muted" style={{ marginTop: '1.5rem', textAlign: 'center' }}>
        Already have an account?{' '}
        <Link to="/login" style={{ color: 'var(--accent)', fontWeight: 600 }}>
          Login
        </Link>
      </p>
    </div>
  );
}
