import { useEffect, useState } from 'react';
import { NavLink, Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import styles from './App.module.css';
import {
  getJson,
  sendJson,
  setSessionToken,
  type LocalUser,
  type RegisterStatus,
} from './api';
import Overview from './pages/Overview';
import Config from './pages/Config';
import Topology from './pages/Topology';
import Providers from './pages/Providers';
import Replay from './pages/Replay';
import Playground from './pages/Playground';
import Cost from './pages/Cost';
import Keys from './pages/Keys';
import Account from './pages/Account';
import Users from './pages/Users';
import Login from './pages/Login';
import Register from './pages/Register';

const links = [
  { to: '/', label: 'Overview', end: true },
  { to: '/account', label: 'Account' },
  { to: '/cost', label: 'Cost' },
  { to: '/keys', label: 'API keys' },
  { to: '/users', label: 'Users', admin: true },
  { to: '/config', label: 'Config' },
  { to: '/topology', label: 'Topology' },
  { to: '/providers', label: 'Providers' },
  { to: '/replay', label: 'Replay' },
  { to: '/playground', label: 'Playground' },
];

export default function App() {
  const loc = useLocation();
  const nav = useNavigate();
  const [boot, setBoot] = useState(true);
  const [needsSetup, setNeedsSetup] = useState(false);
  const [user, setUser] = useState<LocalUser | null>(null);
  const publicAuth =
    loc.pathname === '/login' || loc.pathname === '/register';

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const st = await getJson<RegisterStatus>('/v1/router/auth/register-status');
        if (cancelled) return;
        setNeedsSetup(st.needs_setup);
        if (st.needs_setup) {
          setUser(null);
          setBoot(false);
          return;
        }
        try {
          const me = await getJson<{ user: LocalUser }>('/v1/router/auth/me');
          if (!cancelled) setUser(me.user);
        } catch {
          if (!cancelled) setUser(null);
        }
      } catch {
        if (!cancelled) setUser(null);
      } finally {
        if (!cancelled) setBoot(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [loc.pathname]);

  async function logout() {
    try {
      await sendJson('/v1/router/auth/logout', 'POST', {});
    } catch {
      /* ignore */
    }
    setSessionToken(null);
    setUser(null);
    nav('/login');
  }

  if (boot) {
    return <div className={styles.main}>Loading…</div>;
  }

  if (needsSetup && !publicAuth) {
    return (
      <div className={styles.shell}>
        <nav className={styles.nav}>
          <div className={styles.brand}>aria-router</div>
        </nav>
        <main className={styles.main}>
          <h1>Local setup required</h1>
          <p>
            Run <code>aria-router setup</code> to create the first local admin, then open Login.
          </p>
          <p>
            <NavLink to="/login">Login</NavLink>
          </p>
        </main>
      </div>
    );
  }

  if (!needsSetup && !user && !publicAuth) {
    return <Navigate to="/login" replace />;
  }

  if (user && publicAuth) {
    return <Navigate to="/" replace />;
  }

  return (
    <div className={styles.shell}>
      <nav className={styles.nav}>
        <div className={styles.brand}>aria-router</div>
        {user
          ? links
              .filter((l) => !l.admin || user.role === 'admin')
              .map((l) => (
                <NavLink
                  key={l.to}
                  to={l.to}
                  end={l.end}
                  className={({ isActive }) =>
                    isActive ? `${styles.link} ${styles.active}` : styles.link
                  }
                >
                  {l.label}
                </NavLink>
              ))
          : null}
        {user ? (
          <button type="button" className={styles.link} onClick={logout} style={{ marginTop: '1rem', textAlign: 'start', background: 'none', border: 0, cursor: 'pointer' }}>
            Logout ({user.username})
          </button>
        ) : null}
      </nav>
      <main className={styles.main}>
        <Routes>
          <Route path="/login" element={<Login />} />
          <Route path="/register" element={<Register />} />
          <Route path="/" element={<Overview />} />
          <Route path="/account" element={<Account />} />
          <Route path="/cost" element={<Cost />} />
          <Route path="/keys" element={<Keys />} />
          <Route path="/users" element={<Users />} />
          <Route path="/config" element={<Config />} />
          <Route path="/topology" element={<Topology />} />
          <Route path="/providers" element={<Providers />} />
          <Route path="/replay" element={<Replay />} />
          <Route path="/playground" element={<Playground />} />
        </Routes>
      </main>
    </div>
  );
}
