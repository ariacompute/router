/** jsdom render snapshots: every route in light + dark (no browser needed). */
import { cleanup, render, waitFor } from '@testing-library/react';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../src/App';

const config = JSON.parse(
  readFileSync(resolve(process.cwd(), '.impeccable/config.json'), 'utf-8'),
) as { dom: { routes: string[]; themes: string[] } };

const ROUTES = config.dom.routes;
const THEMES = config.dom.themes;

type JsonResponse = {
  ok: boolean;
  status: number;
  headers: Headers;
  json: () => Promise<unknown>;
  text: () => Promise<string>;
};

function jsonResponse(data: unknown): JsonResponse {
  return {
    ok: true,
    status: 200,
    headers: new Headers(),
    json: async () => data,
    text: async () => JSON.stringify(data),
  };
}

function payload(url: string): unknown {
  if (url.includes('/auth/register-status')) return { needs_setup: false };
  if (url.includes('/auth/me')) {
    return { user: { username: 'admin', role: 'admin', email: 'admin@local' } };
  }
  if (url.includes('/version')) return { version: '0.1.0', commit: 'test' };
  if (url.includes('/cost/by-competence')) {
    return {
      by_tier: {},
      actual_cost_usd: 0,
      notional_large_cost_usd: 0,
      estimated_savings_usd: 0,
    };
  }
  if (url.includes('/cost/model-performance')) {
    return {
      by_model: [],
      by_tier: [],
      requests: 0,
      evidence_count: 0,
      scored_count: 0,
      has_evidence: false,
      estimate_note: 'heuristic',
    };
  }
  if (url.includes('/trust')) {
    return {
      enabled: false,
      thresholds: {
        min_evidence: 5,
        min_confidence: 0.6,
        promote_at: 0.8,
        demote_below: 0.4,
        plan_version: null,
        rubric_version: null,
      },
      entrypoint: 'ep',
      evidence_count: 0,
      drafts: [],
      publications: [],
      published: null,
      online_config_unchanged: true,
    };
  }
  if (url.includes('/config')) return { yaml: 'version: v0.3', document: {} };
  if (url.includes('/replay')) return { items: [], records: [] };
  if (url.includes('/providers')) return { models: [] };
  if (url.includes('/users')) return { users: [] };
  if (url.includes('/keys')) return { keys: [] };
  if (url.includes('/topology')) return { nodes: [], edges: [] };
  if (url.includes('/overview')) {
    return { status: 'ok', entrypoints: 1, recipes: 1, providers: 1 };
  }
  if (url.includes('/cost')) {
    return {
      totals: {
        requests: 0,
        distinct_users: 0,
        sessions: 0,
        turns: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        tokens: 0,
        cost_usd: 0,
        priced_fraction: 0,
      },
      factors: {
        users: 0,
        sessions_per_user: 0,
        turns_per_session: 0,
        requests_per_turn: 0,
        tokens_per_request: 0,
        price_per_mtok: 0,
        product_usd: 0,
        attributed_cost_usd: 0,
        residual_usd: 0,
      },
      by_key: {},
      by_model: {},
      by_layer: {},
      by_entrypoint: {},
      recent: [],
    };
  }
  return {};
}

// Public auth routes render stably only when there is no authenticated user;
// otherwise App immediately <Navigate>s away and empties the container.
let authenticated = true;

beforeEach(() => {
  localStorage.clear();
  authenticated = true;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes('/auth/me') && !authenticated) {
        return { ...jsonResponse({}), ok: false, status: 401 };
      }
      return jsonResponse(payload(url));
    }),
  );
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('route render snapshots', () => {
  it('generates a non-empty snapshot for every route x theme', async () => {
    const snapshots: Array<Record<string, unknown>> = [];
    for (const theme of THEMES) {
      document.documentElement.setAttribute('data-theme', theme);
      for (const route of ROUTES) {
        const publicAuth = route === '/login' || route === '/register';
        authenticated = !publicAuth;
        cleanup();
        const { container } = render(
          <MemoryRouter initialEntries={[route]}>
            <App />
          </MemoryRouter>,
        );
        // Wait for the boot gate (and the page's own loading state) to settle.
        await waitFor(() => {
          expect(container.textContent ?? '').not.toContain('Loading…');
          expect((container.textContent ?? '').trim().length).toBeGreaterThan(0);
        });
        const text = (container.textContent ?? '').replace(/\s+/g, ' ').trim();
        const elements = container.querySelectorAll('*').length;
        expect(text.length, `${theme}:${route}`).toBeGreaterThan(0);
        expect(elements, `${theme}:${route} text=${text.slice(0, 80)}`).toBeGreaterThan(0);
        expect(document.documentElement.getAttribute('data-theme')).toBe(theme);
        snapshots.push({ route, theme, text, elements });
      }
    }
    expect(snapshots.length).toBe(ROUTES.length * THEMES.length);
    expect(new Set(snapshots.map((s) => `${s.theme}:${s.route}`)).size).toBe(snapshots.length);

    if (process.env.AUDIT_OUT_DIR) {
      const out = resolve(process.cwd(), '..', 'detect-out');
      mkdirSync(out, { recursive: true });
      writeFileSync(
        resolve(out, 'render-snapshots.json'),
        JSON.stringify(snapshots, null, 2) + '\n',
        'utf-8',
      );
    }
  });

  it('covers the admin-only route and keeps both themes distinct', () => {
    expect(ROUTES).toContain('/users');
    expect(new Set(THEMES)).toEqual(new Set(['light', 'dark']));
  });
});
