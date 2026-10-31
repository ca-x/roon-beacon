export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

export async function request<Result>(path: string, body?: unknown, signal?: AbortSignal): Promise<Result> {
  const timeout = AbortSignal.timeout(15000);
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    credentials: 'same-origin',
    cache: 'no-store',
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout
  });
  const raw = await response.text();
  let data: unknown;
  try { data = raw ? JSON.parse(raw) : {}; } catch {
    if (response.ok) throw new Error('Beacon returned an invalid response.');
  }
  if (!response.ok) {
    if (response.status === 401 && path !== '/api/login') window.dispatchEvent(new Event('beacon-auth-expired'));
    const detail = data && typeof data === 'object' && 'error' in data ? String(data.error) : '';
    const validation = response.status === 422 && !raw.trimStart().startsWith('<') ? raw.slice(0, 500) : '';
    throw new ApiError(detail || validation || `Request failed (${response.status}).`, response.status);
  }
  return data as Result;
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error || '');
}

export function clockTime(value: number | undefined): string {
  const total = Math.max(0, Math.floor(value || 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  return hours ? `${hours}:${String(minutes).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}` : `${minutes}:${String(total % 60).padStart(2, '0')}`;
}
