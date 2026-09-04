import { useState } from 'react';
import { getTheme, toggleTheme } from '../theme';

function SunIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </svg>
  );
}

export default function ThemeSwitch() {
  const [isDark, setIsDark] = useState(getTheme() === 'dark');

  function onClick() {
    const next = toggleTheme();
    setIsDark(next === 'dark');
  }

  return (
    <button
      type="button"
      onClick={onClick}
      title={isDark ? 'Switch to light mode' : 'Switch to dark mode'}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: '0.5rem',
        padding: '0.45rem 0.7rem',
        borderRadius: '8px',
        border: '1px solid var(--line)',
        background: 'transparent',
        color: 'var(--text-muted)',
        fontSize: '0.8rem',
        cursor: 'pointer',
        transition: 'all 0.18s ease',
        width: '100%',
        justifyContent: 'flex-start',
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.color = 'var(--text-main)';
        e.currentTarget.style.background = 'var(--panel-bg-soft)';
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.color = 'var(--text-muted)';
        e.currentTarget.style.background = 'transparent';
      }}
    >
      {isDark ? <MoonIcon /> : <SunIcon />}
      <span>{isDark ? 'Dark' : 'Light'}</span>
    </button>
  );
}
