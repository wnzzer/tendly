import { json, request } from "./api";
import type { BackendHealth } from "./backend-capabilities";

/** GET/POST /api/update (crates/server/src/update.rs). */
export interface UpdateStatus {
  current: string;
  latest: string | null;
  available: boolean;
  /** Put on disk by an update, waiting for a restart. */
  installed: string | null;
  method: "npm" | "manual";
  writable: boolean;
  command: string;
  restart: "daemon" | "supervisor" | "manual";
  running_sessions: number;
  check_error: string | null;
}

/**
 * The one thing the panel should offer. Ordered by what a person can act on:
 * a waiting restart beats a newer release, and an install the server cannot
 * touch is shown as a command before any version comparison.
 */
export type UpdateStage = "restart" | "manual" | "needs-permission" | "check-failed" | "available" | "current";
export function updateStage(status: UpdateStatus): UpdateStage {
  if (status.installed) return "restart";
  if (status.method !== "npm") return "manual";
  if (status.check_error) return "check-failed";
  if (!status.available) return "current";
  return status.writable ? "available" : "needs-permission";
}

export const readUpdate = (refresh = false) => request<UpdateStatus>(`/update${refresh ? "?refresh=true" : ""}`);
export const installUpdate = () => request<UpdateStatus>("/update", json("POST"));
export const restartForUpdate = () => request<{ restart: UpdateStatus["restart"] }>("/update/restart", json("POST"));

/**
 * Wait for the gateway to come back running `version`.
 *
 * Unanswered checks are the restart itself and say nothing. An answer from the
 * old version is the old process not yet gone, which a daemon restart shows
 * for a moment. Only the new version ends the wait; anything else until the
 * deadline is a restart that did not happen.
 */
export async function waitForVersion(
  version: string,
  health: () => Promise<BackendHealth>,
  {
    timeoutMs = 90_000,
    intervalMs = 1_000,
    sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms)),
    now = () => Date.now(),
  } = {},
): Promise<boolean> {
  const deadline = now() + timeoutMs;
  while (now() < deadline) {
    await sleep(intervalMs);
    try {
      if ((await health()).version === version) return true;
    } catch {
      /* Down while restarting. */
    }
  }
  return false;
}
