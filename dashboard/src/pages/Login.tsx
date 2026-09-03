import { FormEvent, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { sendJson, setSessionToken, type LocalUser } from '../api';
import styles from './page.module.css';

export default function Login() {
  const nav = useNavigate();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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

  return (
    <>
      <h1 className={styles.h1}>Local (router Dashboard) — Login</h1>
      <p>Username/password for this router instance. Not OAuth / Aria Compute.</p>
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
            placeholder="password"
            autoComplete="current-password"
            disabled={busy}
          />
          <button type="submit" disabled={busy || !username || !password}>
            Login
          </button>
        </div>
      </form>
      {err ? <p className={styles.err}>{err}</p> : null}
      <p>
        No account? <Link to="/register">Register</Link>
      </p>
    </>
  );
}
