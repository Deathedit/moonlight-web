// Typed client for the bridge's REST API (`/api/*`).

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    let message = text || res.statusText;
    try {
      message = JSON.parse(text).error ?? message;
    } catch {
      // not JSON (e.g. proxy error page) — keep raw text
    }
    throw new ApiError(res.status, message);
  }
  return res.status === 204 ? (undefined as T) : res.json();
}

export interface Health {
  status: string;
  version: string;
  stages: number;
}

export interface Host {
  id: string;
  name: string;
  address: string;
  port: number;
  paired: boolean;
  online: boolean;
  busy: boolean;
  /** 0 when idle, otherwise the running app's id. */
  currentGame: number;
  appVersion: string | null;
}

export interface DiscoveredHost {
  name: string;
  address: string;
  port: number;
  /** Set when already in the host list. */
  knownId: string | null;
}

export interface App {
  id: number;
  title: string;
  hdr: boolean;
  /** Currently running on the host (can be resumed). */
  running: boolean;
}

export type PairState =
  | { pin: string; status: 'pending' }
  | { pin: string; status: 'paired' }
  | { pin: string; status: 'failed'; error: string };

const enc = encodeURIComponent;

export const api = {
  health: () => request<Health>('GET', '/health'),
  listHosts: () => request<Host[]>('GET', '/hosts'),
  getHost: (id: string) => request<Host>('GET', `/hosts/${enc(id)}`),
  addHost: (address: string) => request<Host>('POST', '/hosts', { address }),
  deleteHost: (id: string) => request<void>('DELETE', `/hosts/${enc(id)}`),
  startPair: (id: string) => request<PairState>('POST', `/hosts/${enc(id)}/pair`),
  pairStatus: (id: string) => request<PairState>('GET', `/hosts/${enc(id)}/pair`),
  cancelPair: (id: string) => request<void>('DELETE', `/hosts/${enc(id)}/pair`),
  unpair: (id: string) => request<void>('POST', `/hosts/${enc(id)}/unpair`),
  discover: () => request<DiscoveredHost[]>('GET', '/discover'),
  listApps: (id: string) => request<App[]>('GET', `/hosts/${enc(id)}/apps`),
  quitApp: (id: string) => request<void>('POST', `/hosts/${enc(id)}/quit`),
  artUrl: (id: string, appId: number) => `/api/hosts/${enc(id)}/apps/${appId}/art`,
};
