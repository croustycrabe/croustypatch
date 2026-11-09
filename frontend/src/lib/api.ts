export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message);
  }
}

type Unauthorized = () => void;
let onUnauthorized: Unauthorized | null = null;
export function setUnauthorizedHandler(fn: Unauthorized | null) {
  onUnauthorized = fn;
}

/** JSON request to the CroustyPatch API. The session lives in an HttpOnly cookie, never in JS. */
export async function api<T = void>(method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api/v1${path}`, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    /* non-JSON error page from a proxy */
  }
  if (!res.ok) {
    const d = (data ?? {}) as { error?: string; message?: string };
    // 401 on a data call means the session ended; the login endpoint reports its own 401.
    if (res.status === 401 && !path.startsWith('/auth/login')) onUnauthorized?.();
    throw new ApiError(res.status, d.error ?? 'error', d.message ?? res.statusText);
  }
  return data as T;
}

/** `module:action` pattern matching, mirroring the server. */
export function allows(perms: string[], need: string): boolean {
  const [nm, na] = need.split(':');
  return perms.some((p) => {
    if (p === '*' || p === need) return true;
    const [hm, ha] = p.split(':');
    return hm !== undefined && ha !== undefined && (hm === '*' || hm === nm) && (ha === '*' || ha === na);
  });
}
