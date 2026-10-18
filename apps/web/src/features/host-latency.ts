/** How the host card reads its round-trip time, kept out of the view so it can
 * be tested on its own. */

export type HostLevel = "online" | "slow" | "offline";

/** Above this round-trip the host reads as sluggish rather than merely online. */
export const SLOW_LATENCY_MS = 150;

/**
 * Turn a measured round-trip (or its absence) into the label and level the card
 * shows. `online` is the connection the rest of the app already tracks; a stale
 * `ms` never overrides it, so a dropped host reads as offline at once.
 */
export function formatLatency(
  ms: number | undefined,
  online: boolean,
): { text: string; level: HostLevel } {
  if (!online) return { text: "Offline", level: "offline" };
  if (ms === undefined) return { text: "—", level: "online" };
  return { text: `${ms}ms`, level: ms > SLOW_LATENCY_MS ? "slow" : "online" };
}
