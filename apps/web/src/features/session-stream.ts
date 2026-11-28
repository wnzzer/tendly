export type SessionStreamState = "connecting" | "connected" | "disconnected" | "exited" | "error";
export type SessionStreamSocket = Pick<
  WebSocket,
  "binaryType" | "readyState" | "onopen" | "onmessage" | "onerror" | "onclose" | "send" | "close"
>;
export const SESSION_STREAM_TIMEOUT_MS = 8_000;
export const SESSION_STREAM_TIMEOUT =
  "Connection timed out. The process may still be running; reconnect the view to try again.";
export const SESSION_STREAM_ERROR =
  "Unable to connect the session view. The process may still be running; reconnect the view to try again.";
export const SESSION_STREAM_CLOSED =
  "The session view disconnected. The process may still be running; reconnect the view to try again.";
export const SESSION_STREAM_RENDER_ERROR =
  "Unable to display the session stream. The process may still be running; reconnect the view to try again.";
export const TERMINAL_VIEW_ERROR =
  "Unable to open the terminal view. The process may still be running; reconnect the view to try again.";

/** A quiet stream is probed this often while the page is in front. */
export const HEARTBEAT_MS = 15_000;
/** How long a probe on the page's return may go unanswered before the socket is replaced. */
export const PROBE_TIMEOUT_MS = 4_000;
/** Retries back off from the first delay to the cap, and give up after this many in a row. */
export const RETRY_DELAYS_MS = [300, 1_000, 2_000, 4_000, 8_000, 10_000, 10_000, 10_000];
export const PING_MESSAGE = '{"type":"ping"}';
export const PONG_MESSAGE = '{"type":"pong"}';

interface SessionStreamOptions {
  createSocket(url: string): SessionStreamSocket;
  onState(state: SessionStreamState, notice: string): void;
  onMessage(data: unknown): void;
  onOpen?(): void;
  onPong?(): void;
  timeoutMs?: number;
  setTimer?(callback: () => void, delay: number): unknown;
  clearTimer?(timer: unknown): void;
  /**
   * Keep the view attached on its own: a dropped or silent socket is replaced
   * with backoff, and a quiet one is probed with a ping the server answers.
   * The view is the only thing retried -- nothing typed is queued or resent.
   */
  reconnect?: boolean;
  /** Whether the page is out of sight; retries and probes wait for its return. */
  hidden?(): boolean;
  now?(): number;
}

/**
 * View transport only: there are no process APIs and no queued or replayed
 * inputs. With `reconnect` it re-attaches itself; panes also call `wake` when
 * the page returns (page-return.ts).
 */
export function createSessionStream(options: SessionStreamOptions) {
  let socket: SessionStreamSocket | undefined,
    timer: unknown,
    retryTimer: unknown,
    beatTimer: unknown,
    probeTimer: unknown;
  let generation = 0,
    disposed = false,
    attempts = 0,
    lastInbound = 0;
  let state: SessionStreamState = "disconnected";
  let target: string | (() => string) | undefined;
  /** Retrying is wanted: set by `connect`, cleared by every deliberate stop. */
  let retrying = false;
  const schedule = options.setTimer ?? ((callback, delay) => setTimeout(callback, delay));
  const cancel = options.clearTimer ?? (handle => clearTimeout(handle as ReturnType<typeof setTimeout>));
  const now = options.now ?? (() => Date.now());
  const hidden = () => {
    try {
      return options.hidden?.() === true;
    } catch {
      return false;
    }
  };
  function clearTimer(handle: unknown) {
    if (handle !== undefined) cancel(handle);
  }
  function clearHandshake() {
    clearTimer(timer);
    timer = undefined;
  }
  function clearLiveness() {
    clearTimer(beatTimer);
    beatTimer = undefined;
    clearTimer(probeTimer);
    probeTimer = undefined;
  }
  function clearRetry() {
    clearTimer(retryTimer);
    retryTimer = undefined;
  }
  function retire(close = true) {
    generation++;
    clearHandshake();
    clearLiveness();
    const previous = socket;
    socket = undefined;
    if (!previous) return;
    previous.onopen = previous.onmessage = previous.onerror = previous.onclose = null;
    if (close)
      try {
        previous.close();
      } catch {
        /* A retired view must not affect the replacement. */
      }
  }
  function publish(next: SessionStreamState, notice = "") {
    state = next;
    options.onState(next, notice);
  }
  function stop() {
    retrying = false;
    clearRetry();
  }
  function fail(notice = SESSION_STREAM_ERROR) {
    if (!disposed) {
      stop();
      retire();
      publish("error", notice);
    }
  }
  /**
   * The transport failed on its own. Without `reconnect` that is the end of
   * the view; with it the view says it is connecting and tries again, and only
   * a run of failures in a row is reported.
   */
  function lost(state: "error" | "disconnected", notice: string, close = true) {
    if (disposed) return;
    retire(close);
    if (!options.reconnect || !retrying || attempts >= RETRY_DELAYS_MS.length) {
      stop();
      publish(state, notice);
      return;
    }
    publish("connecting");
    // Out of sight there is nothing to show and a phone may be asleep; the
    // page's return wakes the view instead of a timer firing into the void.
    if (hidden()) return;
    const delay = RETRY_DELAYS_MS[attempts++]!;
    clearRetry();
    retryTimer = schedule(() => {
      retryTimer = undefined;
      open(false);
    }, delay);
  }
  function heartbeat() {
    clearTimer(beatTimer);
    beatTimer = schedule(() => {
      beatTimer = undefined;
      if (!socket || state !== "connected") return;
      if (!hidden()) {
        // Two beats without a word -- not even the answer to the last ping --
        // is a socket the network has silently dropped.
        if (now() - lastInbound > HEARTBEAT_MS * 2) {
          lost("disconnected", SESSION_STREAM_CLOSED);
          return;
        }
        try {
          socket.send(PING_MESSAGE);
        } catch {
          lost("error", SESSION_STREAM_ERROR);
          return;
        }
      }
      heartbeat();
    }, HEARTBEAT_MS);
  }
  function open(announce = true) {
    if (disposed || target === undefined) return;
    retire();
    if (announce || state !== "connecting") publish("connecting");
    const own = generation;
    try {
      const current = options.createSocket(typeof target === "function" ? target() : target);
      socket = current;
      current.binaryType = "arraybuffer";
      const isCurrent = () => !disposed && generation === own && socket === current;
      current.onopen = () => {
        if (!isCurrent()) return;
        clearHandshake();
        attempts = 0;
        lastInbound = now();
        publish("connected");
        if (options.reconnect) heartbeat();
        try {
          options.onOpen?.();
        } catch {
          fail(SESSION_STREAM_RENDER_ERROR);
        }
      };
      current.onmessage = event => {
        if (!isCurrent()) return;
        // Anything at all proves the socket alive, not only the answer to a probe.
        lastInbound = now();
        clearTimer(probeTimer);
        probeTimer = undefined;
        if (event.data === PONG_MESSAGE) {
          options.onPong?.();
          return;
        }
        try {
          options.onMessage(event.data);
        } catch {
          fail(SESSION_STREAM_RENDER_ERROR);
        }
      };
      current.onerror = () => {
        if (isCurrent()) lost("error", SESSION_STREAM_ERROR);
      };
      current.onclose = () => {
        if (isCurrent()) lost("disconnected", SESSION_STREAM_CLOSED, false);
      };
      timer = schedule(() => {
        if (isCurrent() && state === "connecting") lost("error", SESSION_STREAM_TIMEOUT);
      }, options.timeoutMs ?? SESSION_STREAM_TIMEOUT_MS);
    } catch {
      lost("error", SESSION_STREAM_ERROR);
    }
  }
  /** Attach to `url`; a function is asked again on each retry, so it can say where to resume. */
  function connect(url: string | (() => string)) {
    if (disposed) return;
    target = url;
    retrying = true;
    attempts = 0;
    clearRetry();
    open();
  }
  /**
   * The page is back. A view waiting to retry, or one that gave up, tries now;
   * a socket that claims to be open is replaced when `stale`, and otherwise
   * asked to prove it is alive within a few seconds.
   */
  function wake(stale = false) {
    if (disposed || target === undefined || state === "exited") return;
    if (!socket || state === "error" || state === "disconnected" || retryTimer !== undefined) {
      if (state === "connecting" && socket) return;
      retrying = true;
      attempts = 0;
      clearRetry();
      open();
      return;
    }
    if (state !== "connected") return;
    if (stale) {
      retrying = true;
      attempts = 0;
      open();
      return;
    }
    if (!options.reconnect || probeTimer !== undefined) return;
    try {
      socket.send(PING_MESSAGE);
    } catch {
      lost("error", SESSION_STREAM_ERROR);
      return;
    }
    probeTimer = schedule(() => {
      probeTimer = undefined;
      lost("disconnected", SESSION_STREAM_CLOSED);
    }, PROBE_TIMEOUT_MS);
  }
  function send(data: Parameters<WebSocket["send"]>[0]): boolean {
    if (disposed || !socket || socket.readyState !== 1 || state !== "connected") return false;
    try {
      socket.send(data);
      return true;
    } catch {
      lost("error", SESSION_STREAM_ERROR);
      return false;
    }
  }
  function disconnect() {
    if (!disposed) {
      stop();
      retire();
      publish("disconnected", SESSION_STREAM_CLOSED);
    }
  }
  function finish(notice: string) {
    if (!disposed) {
      stop();
      retire();
      publish("exited", notice);
    }
  }
  function dispose() {
    if (!disposed) {
      stop();
      disposed = true;
      retire();
    }
  }
  return { connect, wake, send, disconnect, finish, fail, dispose };
}
