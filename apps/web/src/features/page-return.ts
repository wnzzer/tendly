/**
 * The page coming back to the person: a tab shown again, a phone unlocked, a
 * page restored from the back-forward cache, or the network returning.
 *
 * `stale` says a socket that still looks open should not be trusted. A phone
 * that slept usually leaves its WebSockets half-dead: no close event, no data,
 * until the OS gives up on the connection minutes later. Brief tab switches
 * keep their sockets; a longer absence, a cache restore or an offline spell
 * reconnects even what claims to be connected.
 */
export const STALE_AFTER_MS = 30_000;

export interface PageEvents {
  hidden(): boolean;
  now(): number;
  on(target: "document" | "window", type: string, listener: (event: Event) => void): () => void;
}

const browserEvents: PageEvents = {
  hidden: () => document.hidden,
  now: () => Date.now(),
  on(target, type, listener) {
    const where = target === "document" ? document : window;
    where.addEventListener(type, listener);
    return () => where.removeEventListener(type, listener);
  },
};

export function onPageReturn(callback: (stale: boolean) => void, events: PageEvents = browserEvents): () => void {
  let hiddenAt = events.hidden() ? events.now() : undefined;
  const stops = [
    events.on("document", "visibilitychange", () => {
      if (events.hidden()) {
        hiddenAt ??= events.now();
        return;
      }
      const away = hiddenAt === undefined ? 0 : events.now() - hiddenAt;
      hiddenAt = undefined;
      callback(away >= STALE_AFTER_MS);
    }),
    events.on("window", "pageshow", event => {
      if ((event as PageTransitionEvent).persisted) callback(true);
    }),
    events.on("window", "online", () => {
      if (!events.hidden()) callback(true);
    }),
  ];
  return () => {
    for (const stop of stops) stop();
  };
}
