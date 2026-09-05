const THEME_KEY = 'aria_router_theme';

export type Theme = 'light' | 'dark';

export function getTheme(): Theme {
  if (typeof window === 'undefined') return 'light';
  const stored = localStorage.getItem(THEME_KEY);
  if (stored === 'light' || stored === 'dark') return stored;
  const attr = document.documentElement.getAttribute('data-theme');
  if (attr === 'light' || attr === 'dark') return attr;
  if (document.documentElement.classList.contains('dark')) return 'dark';
  if (window.matchMedia('(prefers-color-scheme: dark)').matches) return 'dark';
  return 'light';
}

export function setTheme(theme: Theme): void {
  if (typeof window === 'undefined') return;
  localStorage.setItem(THEME_KEY, theme);
  applyTheme(theme);
}

export function toggleTheme(): Theme {
  const next: Theme = getTheme() === 'dark' ? 'light' : 'dark';
  setTheme(next);
  return next;
}

export function applyTheme(theme: Theme): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  // `data-theme` is the source of truth (matches serve/frontend). The legacy
  // `.dark` class is kept in sync so any markup still keying off it works.
  root.setAttribute('data-theme', theme);
  root.classList.toggle('dark', theme === 'dark');
}

export function initTheme(): void {
  applyTheme(getTheme());
}
