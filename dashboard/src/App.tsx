import { NavLink, Route, Routes } from 'react-router-dom';
import styles from './App.module.css';
import Overview from './pages/Overview';
import Config from './pages/Config';
import Topology from './pages/Topology';
import Providers from './pages/Providers';
import Replay from './pages/Replay';
import Playground from './pages/Playground';
import Cost from './pages/Cost';
import Keys from './pages/Keys';

const links = [
  { to: '/', label: 'Overview', end: true },
  { to: '/cost', label: 'Cost' },
  { to: '/keys', label: 'API keys' },
  { to: '/config', label: 'Config' },
  { to: '/topology', label: 'Topology' },
  { to: '/providers', label: 'Providers' },
  { to: '/replay', label: 'Replay' },
  { to: '/playground', label: 'Playground' },
];

export default function App() {
  return (
    <div className={styles.shell}>
      <nav className={styles.nav}>
        <div className={styles.brand}>ariarouter</div>
        {links.map((l) => (
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
      </nav>
      <main className={styles.main}>
        <Routes>
          <Route path="/" element={<Overview />} />
          <Route path="/cost" element={<Cost />} />
          <Route path="/keys" element={<Keys />} />
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
