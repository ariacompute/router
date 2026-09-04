import { useState } from 'react';
import { startAriaOAuth } from '../api';

export type AriaSite = 'com' | 'cn';

const SITE_STORAGE_KEY = 'aria_router_oauth_site';

const SITES: { id: AriaSite; label: string; url: string }[] = [
  { id: 'com', label: 'International Site', url: 'https://ariacompute.com' },
  { id: 'cn', label: 'China Site', url: 'https://ariacompute.cn' },
];

function initialSite(): AriaSite {
  return localStorage.getItem(SITE_STORAGE_KEY) === 'cn' ? 'cn' : 'com';
}

/// "Sign in with Aria Compute" button with an explicit China / International
/// site picker. The picked site is remembered in localStorage and forwarded to
/// the router OAuth start endpoint, which normalizes it to ariacompute.com
/// (com/intl) or ariacompute.cn (cn).
export default function OAuthSignIn({ onError }: { onError: (msg: string | null) => void }) {
  const [site, setSite] = useState<AriaSite>(initialSite);
  const [busy, setBusy] = useState(false);

  function choose(next: AriaSite) {
    setSite(next);
    localStorage.setItem(SITE_STORAGE_KEY, next);
  }

  async function signIn() {
    setBusy(true);
    onError(null);
    try {
      const { authorize_url } = await startAriaOAuth(site);
      window.location.href = authorize_url;
    } catch (ex) {
      onError((ex as Error).message);
      setBusy(false);
    }
  }

  return (
    <div className="stack" style={{ gap: '0.75rem' }}>
      <div
        role="radiogroup"
        aria-label="Aria Compute site"
        style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '0.5rem' }}
      >
        {SITES.map((s) => {
          const active = s.id === site;
          return (
            <button
              key={s.id}
              type="button"
              role="radio"
              aria-checked={active}
              onClick={() => choose(s.id)}
              style={{
                textAlign: 'left',
                padding: '0.6rem 0.75rem',
                borderRadius: 'var(--radius-sm)',
                border: `1px solid ${active ? 'var(--accent)' : 'var(--line)'}`,
                background: active ? 'var(--accent-wash)' : 'transparent',
                color: active ? 'var(--accent)' : 'var(--text-main)',
                cursor: 'pointer',
              }}
            >
              <div style={{ fontWeight: 600, fontSize: '0.9rem' }}>{s.label}</div>
              <div className="muted" style={{ fontSize: '0.72rem', marginTop: '0.15rem' }}>
                {s.url}
              </div>
            </button>
          );
        })}
      </div>
      <button
        type="button"
        className="btn-primary"
        onClick={signIn}
        disabled={busy}
        style={{ width: '100%' }}
      >
        {busy ? 'Redirecting…' : 'Sign in with Aria Compute'}
      </button>
    </div>
  );
}
