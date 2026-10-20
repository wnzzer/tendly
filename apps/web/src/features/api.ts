import { useI18n } from "../i18n";

export class ApiError extends Error {
  readonly status: number;
  constructor(message: string, status: number) { super(message); this.name = "ApiError"; this.status = status; }
}

export class ApiConnectionError extends ApiError {
  readonly outcomeUnknown: boolean;
  constructor(method: string) {
    const outcomeUnknown = !["GET", "HEAD", "OPTIONS"].includes(method.toUpperCase());
    super(useI18n().t(outcomeUnknown
      ? "Connection was lost while sending the request. It may have reached the server; refresh its state before retrying."
      : "Cannot reach Tendly from this page. Check that this page's service is running and the address is correct, then retry."), 0);
    this.name = "ApiConnectionError";
    this.outcomeUnknown = outcomeUnknown;
  }
}
function aborted(cause: unknown, signal?: AbortSignal | null) {
  return signal?.aborted || (cause instanceof Error && cause.name === "AbortError");
}

/** Same-origin transport; Vite proxies /api during local development. */
export async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const method = init.method ?? "GET";
  let response: Response;
  try {
    response = await fetch(`/api${path}`, {
      ...init,
      headers: { "X-Tendly-Client": "web", ...(init.body ? { "Content-Type": "application/json" } : {}), ...init.headers },
    });
  } catch (cause) {
    if (aborted(cause, init.signal)) throw cause;
    // Never automatically replay a mutation: it may already have committed.
    throw new ApiConnectionError(method);
  }
  if (!response.ok) {
    let message = `Request failed (${response.status})`;
    try {
      const raw = await response.text();
      try { const body = JSON.parse(raw); message = body.error ?? body.message ?? message; }
      catch { if (raw.length < 1000 && !raw.startsWith("<!")) message = raw || message; }
    } catch { /* Retain the HTTP error when a response body is unavailable. */ }
    throw new ApiError(String(message), response.status);
  }
  if (response.status === 204) return undefined as T;
  try { return await response.json() as T; }
  catch (cause) {
    if (aborted(cause, init.signal)) throw cause;
    if (cause instanceof TypeError) throw new ApiConnectionError(method);
    throw new ApiError(useI18n().t("The backend returned an unreadable response. Check that the page and backend belong to the same Tendly instance."), response.status);
  }
}

export const json = (method: string, value?: unknown): RequestInit => ({ method, ...(value === undefined ? {} : { body: JSON.stringify(value) }) });
export const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);
export const workspacePath = (id: string) => `/workspaces/${encodeURIComponent(id)}`;
export const sessionPath = (id: string) => `/sessions/${encodeURIComponent(id)}`;
/** A workspace URL in one of its checkouts: without one, the workspace directory; with one, the worktree a session works in. */
export const inCheckout = (url: string, checkout?: string | null) => checkout ? `${url}${url.includes("?") ? "&" : "?"}checkout=${encodeURIComponent(checkout)}` : url;
export const assetUrl = (id: string, path: string, checkout?: string | null) => inCheckout(`/api${workspacePath(id)}/asset?path=${encodeURIComponent(path)}`, checkout);
export { providerLabel } from "./clients";
export const formatBytes = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1024 ** 2 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 ** 2).toFixed(1)} MB`;
