export async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path);
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
    headers: { 'content-type': 'application/json' },
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
    headers: { 'content-type': contentType },
    body,
  });
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
}

async function errorMessage(res: Response): Promise<string> {
  const text = await res.text();
  try {
    const v = JSON.parse(text) as { error?: { message?: string } };
    return v.error?.message ?? text ?? res.statusText;
  } catch {
    return text || res.statusText;
  }
}

export type Overview = {
  status: string;
  entrypoints: number;
  recipes: number;
  providers: number;
  last_route: RouteDecision | null;
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
