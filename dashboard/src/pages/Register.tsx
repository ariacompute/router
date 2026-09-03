import { FormEvent, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import {
  getJson,
  sendJson,
  setSessionToken,
  type LocalUser,
  type RegisterStatus,
} from '../api';
import styles from './page.module.css';

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
      <>
        <h1 className={styles.h1}>Local (router Dashboard) — Register</h1>
        <p className={styles.err}>
          No local users yet. Run <code>aria-router setup</code> to create the first admin.
        </p>
        <p>
          <Link to="/login">Login</Link>
        </p>
      </>
    );
  }

  if (status && !status.allow_register) {
    return (
      <>
        <h1 className={styles.h1}>Local (router Dashboard) — Register</h1>
        <p>Self-registration is disabled. Ask an admin or use Login.</p>
        <p>
          <Link to="/login">Login</Link>
        </p>
      </>
    );
  }

  return (
    <>
      <h1 className={styles.h1}>Local (router Dashboard) — Register</h1>
      <p>Creates a local <code>user</code> account (not admin). Password is for Dashboard only.</p>
      <form onSubmit={onSubmit}>
        <div className={styles.row}>
          <input
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="username"
            autoComplete="username"
            disabled={busy}
          />
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="password (min 8)"
            autoComplete="new-password"
            disabled={busy}
          />
          <button type="submit" disabled={busy || !username || password.length < 8}>
            Register
          </button>
        </div>
      </form>
      {err ? <p className={styles.err}>{err}</p> : null}
      <p>
        Already have an account? <Link to="/login">Login</Link>
      </p>
    </>
  );
}
