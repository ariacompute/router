const TOKEN_KEY = 'aria_router_session_token';

export function getSessionToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

export function setSessionToken(token: string | null) {
  if (token) localStorage.setItem(TOKEN_KEY, token);
  else localStorage.removeItem(TOKEN_KEY);
}

function authHeaders(extra?: HeadersInit): Headers {
  const h = new Headers(extra);
  const tok = getSessionToken();
  if (tok && !h.has('authorization')) {
    h.set('authorization', `Bearer ${tok}`);
  }
  return h;
}

async function errorMessage(res: Response): Promise<string> {
  const text = await res.text();
  try {
    const v = JSON.parse(text) as { error?: { message?: string } | string };
    if (typeof v.error === 'string') return v.error;
    return v.error?.message ?? text ?? res.statusText;
  } catch {
    return text || res.statusText;
  }
}

export async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path, {
    credentials: 'include',
    headers: authHeaders(),
  });
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
  return res.json() as Promise<T>;
}

export async function sendJson<T>(
  path: string,
  method: string,
  body: unknown,
): Promise<{ data: T; headers: Headers }> {
  const res = await fetch(path, {
    method,
    credentials: 'include',
    headers: authHeaders({ 'content-type': 'application/json' }),
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
  const data = (await res.json()) as T;
  return { data, headers: res.headers };
}

export async function putText(path: string, body: string, contentType: string): Promise<void> {
  const res = await fetch(path, {
    method: 'PUT',
    credentials: 'include',
    headers: authHeaders({ 'content-type': contentType }),
    body,
  });
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
}

export async function deleteJson(path: string): Promise<void> {
  const res = await fetch(path, {
    method: 'DELETE',
    credentials: 'include',
    headers: authHeaders(),
  });
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
}

export type LocalUser = {
  id: string;
  username: string;
  role: 'admin' | 'user';
  created_at: string;
  disabled: boolean;
};

export type RegisterStatus = {
  allow_register: boolean;
  needs_setup: boolean;
};

export type ServeAccount = {
  linked: boolean;
  site?: string | null;
  site_url?: string | null;
  gateway_url?: string | null;
  user?: {
    id: unknown;
    email?: string | null;
    role?: string | null;
  } | null;
  linked_at?: string | null;
  api_key_name?: string | null;
  api_key_prefix?: string | null;
  api_key_configured: boolean;
  status: string;
};

export type Overview = {
  status: string;
  entrypoints: number;
  recipes: number;
  providers: number;
  last_route: RouteDecision | null;
  cost?: {
    cost_usd: number;
    requests: number;
    distinct_users: number;
    avg_tokens_per_request: number;
  };
  api_keys?: { active: number; revoked: number };
  local_users?: { admin: number; user: number };
  serve_account?: ServeAccount;
  allow_register?: boolean;
};

export type CostBucket = {
  requests: number;
  prompt_tokens: number;
  completion_tokens: number;
  tokens: number;
  cost_usd: number;
  priced_requests: number;
};

export type CostEvent = {
  ts: string;
  user: string;
  key_id?: string | null;
  key_name?: string | null;
  session: string;
  entrypoint: string;
  layer: string;
  decision: string;
  model: string;
  bypass: boolean;
  prompt_tokens: number;
  completion_tokens: number;
  cost_usd: number;
  tokens_source: string;
  priced: boolean;
};

export type CostReport = {
  totals: {
    requests: number;
    distinct_users: number;
    sessions: number;
    turns: number;
    prompt_tokens: number;
    completion_tokens: number;
    tokens: number;
    cost_usd: number;
    priced_fraction: number;
  };
  factors: {
    users: number;
    sessions_per_user: number;
    turns_per_session: number;
    requests_per_turn: number;
    tokens_per_request: number;
    price_per_mtok: number;
    product_usd: number;
    attributed_cost_usd: number;
    residual_usd: number;
  };
  by_model: Record<string, CostBucket>;
  by_layer: Record<string, CostBucket>;
  by_entrypoint: Record<string, CostBucket>;
  by_key: Record<string, CostBucket>;
  by_local_user?: Record<string, CostBucket>;
  by_serve_user?: Record<string, CostBucket>;
  recent: CostEvent[];
};

export type KeyPublic = {
  id: string;
  name: string;
  prefix: string;
  created_at: string;
  last_used_at?: string | null;
  revoked: boolean;
  owner_user_id?: string | null;
};

export type KeyCreated = {
  id: string;
  name: string;
  prefix: string;
  secret: string;
  created_at: string;
};

export type RouteDecision = {
  model: string;
  algorithm?: string | null;
  reason: string;
  confidence: number;
  layer: string;
  decision: string;
  bypass: boolean;
};

export type ConfigPayload = {
  document: unknown;
  yaml: string;
};

export type ProviderRow = {
  name: string;
  provider_model_id: string;
  locality: string;
  modality: string;
  backend_refs: { name: string; endpoint: string; weight: number }[];
  latency_ms: number | null;
  failures: number;
};

export type Topology = {
  nodes: { id: string; kind: string; label: string; router?: string; signal?: string }[];
  edges: { from: string; to: string }[];
};
