import { useState, CSSProperties } from 'react';
import { getTheme, setTheme, Theme } from '../theme';

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
  const [theme, setThemeState] = useState<Theme>(getTheme());

  function select(next: Theme) {
    setTheme(next);
    setThemeState(next);
  }

  function buttonStyle(active: boolean): CSSProperties {
    return {
      flex: 1,
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      gap: '0.4rem',
      padding: '0.45rem 0.5rem',
      borderRadius: 'var(--radius-pill)',
      border: `1px solid ${active ? 'var(--accent)' : 'var(--line)'}`,
      background: active ? 'var(--accent-wash)' : 'transparent',
      color: active ? 'var(--accent)' : 'var(--text-muted)',
      fontSize: '0.8rem',
      cursor: 'pointer',
      transition: 'all 0.18s ease',
    };
  }

  return (
    <div
      role="group"
      aria-label="Theme"
      style={{ display: 'flex', gap: '0.4rem', width: '100%' }}
    >
      <button
        type="button"
        onClick={() => select('light')}
        title="Light mode"
        aria-pressed={theme === 'light'}
        style={buttonStyle(theme === 'light')}
      >
        <SunIcon />
        <span>Light</span>
      </button>
      <button
        type="button"
        onClick={() => select('dark')}
        title="Dark mode"
        aria-pressed={theme === 'dark'}
        style={buttonStyle(theme === 'dark')}
      >
        <MoonIcon />
        <span>Dark</span>
      </button>
    </div>
  );
}
