import { useEffect, useState } from 'react';
import {
  createUser,
  getJson,
  resetUserPassword,
  sendJson,
  setUserEmail,
  type LocalUser,
  type Overview,
} from '../api';
import styles from './page.module.css';

export default function Users() {
  const [users, setUsers] = useState<LocalUser[]>([]);
  const [allowRegister, setAllowRegister] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [emailEdits, setEmailEdits] = useState<Record<string, string>>({});
  const [pwdEdits, setPwdEdits] = useState<Record<string, string>>({});
  const [newUser, setNewUser] = useState<{
    username: string;
    password: string;
    role: 'admin' | 'user';
  }>({ username: '', password: '', role: 'user' });

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

  async function saveUserEmail(u: LocalUser) {
    setBusy(true);
    setErr(null);
    try {
      const v = (emailEdits[u.id] ?? u.email ?? '').trim();
      await setUserEmail(u.id, v === '' ? null : v);
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

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

  async function create() {
    setBusy(true);
    setErr(null);
    try {
      await createUser(newUser.username.trim(), newUser.password, newUser.role);
      setNewUser({ username: '', password: '', role: 'user' });
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function resetPassword(u: LocalUser) {
    const next = (pwdEdits[u.id] ?? '').trim();
    if (next.length < 8) {
      setErr('password must be at least 8 characters');
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await resetUserPassword(u.id, next);
      setPwdEdits((m) => ({ ...m, [u.id]: '' }));
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
      <h1 className={styles.h1}>Users</h1>
      <div className={styles.row}>
        <button type="button" className="btn-ghost" onClick={toggleAllow} disabled={busy}>
          Self-registration: {allowRegister ? 'ON' : 'OFF'}
        </button>
        <button type="button" className="btn-ghost" onClick={load} disabled={busy}>
          Refresh
        </button>
      </div>
      {err ? <p className={styles.err}>{err}</p> : null}
      <div className={styles.row} style={{ marginBottom: '1rem' }}>
        <input
          className="input-field"
          style={{ maxWidth: '12rem' }}
          value={newUser.username}
          onChange={(e) => setNewUser({ ...newUser, username: e.target.value })}
          placeholder="username"
          disabled={busy}
        />
        <input
          className="input-field"
          style={{ maxWidth: '12rem' }}
          type="password"
          value={newUser.password}
          onChange={(e) => setNewUser({ ...newUser, password: e.target.value })}
          placeholder="password (min 8)"
          disabled={busy}
        />
        <select
          className="input-field"
          style={{ maxWidth: '8rem' }}
          value={newUser.role}
          onChange={(e) =>
            setNewUser({ ...newUser, role: e.target.value === 'admin' ? 'admin' : 'user' })
          }
          disabled={busy}
        >
          <option value="user">user</option>
          <option value="admin">admin</option>
        </select>
        <button
          type="button"
          className="btn-primary"
          onClick={create}
          disabled={busy || !newUser.username.trim() || newUser.password.length < 8}
        >
          Create user
        </button>
      </div>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Username</th>
            <th>Email</th>
            <th>Role</th>
            <th>Created</th>
            <th>Status</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          {users.map((u) => (
            <tr key={u.id}>
              <td>{u.username}</td>
              <td>
                <div className={styles.row}>
                  <input
                    className="input-field"
                    value={emailEdits[u.id] ?? u.email ?? ''}
                    placeholder="—"
                    onChange={(e) =>
                      setEmailEdits((m) => ({ ...m, [u.id]: e.target.value }))
                    }
                    disabled={busy}
                  />
                  <button
                    type="button"
                    className="btn-ghost btn-sm"
                    onClick={() => saveUserEmail(u)}
                    disabled={busy}
                  >
                    Save
                  </button>
                </div>
              </td>
              <td>
                <span className="badge badge-accent">{u.role}</span>
              </td>
              <td className="muted">{u.created_at}</td>
              <td>
                {u.disabled ? (
                  <span className="badge badge-err">disabled</span>
                ) : (
                  <span className="badge badge-ok">active</span>
                )}
              </td>
              <td>
                <div className={styles.row}>
                  <button
                    type="button"
                    className={u.disabled ? 'btn-ghost btn-sm' : 'btn-danger btn-sm'}
                    onClick={() => toggleDisabled(u)}
                    disabled={busy}
                  >
                    {u.disabled ? 'Enable' : 'Disable'}
                  </button>
                  <input
                    className="input-field"
                    style={{ maxWidth: '10rem' }}
                    type="password"
                    value={pwdEdits[u.id] ?? ''}
                    placeholder="new password"
                    onChange={(e) => setPwdEdits((m) => ({ ...m, [u.id]: e.target.value }))}
                    disabled={busy}
                  />
                  <button
                    type="button"
                    className="btn-ghost btn-sm"
                    onClick={() => resetPassword(u)}
                    disabled={busy}
                  >
                    Reset
                  </button>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
