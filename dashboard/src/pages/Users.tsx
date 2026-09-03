import { useEffect, useState } from 'react';
import { getJson, sendJson, type LocalUser, type Overview } from '../api';
import styles from './page.module.css';

export default function Users() {
  const [users, setUsers] = useState<LocalUser[]>([]);
  const [allowRegister, setAllowRegister] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function load() {
    setErr(null);
    Promise.all([
      getJson<{ users: LocalUser[] }>('/v1/router/users'),
      getJson<Overview>('/v1/router/overview'),
    ])
      .then(([u, ov]) => {
        setUsers(u.users);
        setAllowRegister(!!ov.allow_register);
      })
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  async function toggleDisabled(u: LocalUser) {
    setBusy(true);
    setErr(null);
    try {
      await sendJson(`/v1/router/users/${encodeURIComponent(u.id)}/disabled`, 'PUT', {
        disabled: !u.disabled,
      });
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function toggleAllow() {
    setBusy(true);
    setErr(null);
    try {
      await sendJson('/v1/router/settings/allow_register', 'PUT', {
        allow_register: !allowRegister,
      });
      setAllowRegister(!allowRegister);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h1 className={styles.h1}>Local (router Dashboard) — Users</h1>
      <p>Admin-only. Self-register creates <code>user</code> role accounts.</p>
      <div className={styles.row}>
        <button type="button" onClick={toggleAllow} disabled={busy}>
          Self-registration: {allowRegister ? 'ON' : 'OFF'}
        </button>
        <button type="button" onClick={load} disabled={busy}>
          Refresh
        </button>
      </div>
      {err ? <p className={styles.err}>{err}</p> : null}
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Username</th>
            <th>Role</th>
            <th>Created</th>
            <th>Status</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {users.map((u) => (
            <tr key={u.id}>
              <td>{u.username}</td>
              <td>{u.role}</td>
              <td>{u.created_at}</td>
              <td>{u.disabled ? 'disabled' : 'active'}</td>
              <td>
                <button type="button" onClick={() => toggleDisabled(u)} disabled={busy}>
                  {u.disabled ? 'Enable' : 'Disable'}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
