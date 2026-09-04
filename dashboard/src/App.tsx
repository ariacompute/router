import { useEffect, useState } from 'react';
import { NavLink, Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import styles from './App.module.css';
import {
  getJson,
  sendJson,
  setSessionToken,
  type BuildVersion,
  type LocalUser,
  type RegisterStatus,
} from './api';
import ThemeSwitch from './components/ThemeSwitch';
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
import VersionBadge from './components/VersionBadge';

type NavItem = { to: string; label: string; end?: boolean; admin?: boolean };
type NavGroup = { title: string; items: NavItem[] };

const groups: NavGroup[] = [
  {
    title: 'Monitor',
    items: [
      { to: '/', label: 'Overview', end: true },
      { to: '/cost', label: 'Cost' },
      { to: '/account', label: 'Account' },
    ],
  },
  {
    title: 'Manage',
    items: [
      { to: '/keys', label: 'API keys' },
      { to: '/users', label: 'Users', admin: true },
    ],
  },
  {
    title: 'Routing',
    items: [
      { to: '/config', label: 'Config' },
      { to: '/topology', label: 'Topology' },
      { to: '/providers', label: 'Providers' },
      { to: '/replay', label: 'Replay' },
      { to: '/playground', label: 'Playground' },
    ],
  },
];

function Sidebar({ user }: { user: LocalUser }) {
  const nav = useNavigate();
  const [build, setBuild] = useState<BuildVersion | null>(null);

  useEffect(() => {
    let cancelled = false;
    getJson<BuildVersion>('/v1/router/version')
      .then((v) => {
        if (!cancelled) setBuild(v);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  async function logout() {
    try {
      await sendJson('/v1/router/auth/logout', 'POST', {});
    } catch {
      /* ignore */
    }
    setSessionToken(null);
    nav('/login');
  }
  return (
    <nav className={styles.nav}>
      <div className={styles.brand}>
        <span className={styles.brandMark}>A</span>
        <span className={styles.brandText}>Aria Router</span>
      </div>
      {groups.map((g) => (
        <div key={g.title}>
          <div className={styles.section}>{g.title}</div>
          {g.items
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
            ))}
        </div>
      ))}
      <div className={styles.spacer} />
      <div className={styles.footer}>
        <ThemeSwitch />
        <span className={styles.user}>{user.username}</span>
        <button type="button" className={styles.logout} onClick={logout}>
          Logout
        </button>
        {build && (
          <span
            className={styles.version}
            title={`Aria Router v${build.version} @ ${build.commit}`}
          >
            v{build.version}@{build.commit}
          </span>
        )}
      </div>
    </nav>
  );
}

export default function App() {
  const loc = useLocation();
  const [boot, setBoot] = useState(true);
  const [needsSetup, setNeedsSetup] = useState(false);
  const [user, setUser] = useState<LocalUser | null>(null);
  const publicAuth = loc.pathname === '/login' || loc.pathname === '/register';

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

  const routes = (
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
  );

  if (boot) {
    return <div className={styles.main}>Loading…</div>;
  }

  if (needsSetup && !publicAuth) {
    return (
      <div className={styles.shell}>
        <nav className={styles.nav}>
          <div className={styles.brand}>
            <span className={styles.brandMark}>A</span>
            <span className={styles.brandText}>Aria Router</span>
          </div>
        </nav>
        <main className={styles.main}>
          <div className="page">
            <h1 className="h1">Local setup required</h1>
            <p className="muted">
              Run <code>aria-router setup</code> to create the first local admin, then open Login.
            </p>
            <p style={{ marginTop: '0.75rem' }}>
              <NavLink to="/login" style={{ color: 'var(--accent)' }}>
                Login
              </NavLink>
            </p>
          </div>
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

  if (publicAuth) {
    return (
      <div className={styles.authShell}>
        <div className={styles.brandCenter}>
          <span className={styles.brandMark}>A</span>
          <span className={styles.brandText}>Aria Router</span>
        </div>
        {routes}
        <VersionBadge className={styles.authVersion} />
      </div>
    );
  }

  return (
    <div className={styles.shell}>
      <Sidebar user={user!} />
      <main className={styles.main}>
        <div className="page">{routes}</div>
      </main>
    </div>
  );
}
