/**
 * Live file changes, pushed by the host.
 *
 * One socket per workspace on the canvas reports which paths changed on disk,
 * so the tree and open files follow what an agent writes without anyone
 * pressing refresh. `null` means "anything may have changed": the host lost
 * track, or this view reconnected after missing events.
 */
export type ChangedPaths = string[] | null;
type Listener = (workspaceId: string, paths: ChangedPaths) => void;

const listeners = new Set<Listener>();
/** Subscribe to changes in any workspace; returns the unsubscribe. */
export function onFilesChanged(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function announceFilesChanged(workspaceId: string, paths: ChangedPaths) {
  for (const listener of [...listeners]) listener(workspaceId, paths);
}

function parent(path: string) {
  const cut = path.lastIndexOf("/");
  return cut === -1 ? "" : path.slice(0, cut);
}
/**
 * The loaded directories a change touches: the folder an entry appeared in,
 * disappeared from or was renamed within, and a changed folder itself. `null`
 * asks for a full refresh.
 */
export function directoriesToReload(paths: ChangedPaths, loaded: Iterable<string>): string[] | null {
  if (!paths) return null;
  const known = new Set(loaded),
    result = new Set<string>();
  for (const path of paths)
    for (const candidate of [parent(path), path]) if (known.has(candidate)) result.add(candidate);
  return [...result];
}
export function touchesFile(paths: ChangedPaths, path: string): boolean {
  return !paths || paths.includes(path);
}

export interface FileWatchMessage {
  type: "ready" | "changed" | "unavailable";
  paths?: ChangedPaths;
  git?: boolean;
  message?: string;
}
interface FileWatchOptions {
  createSocket(url: string): Pick<WebSocket, "onopen" | "onmessage" | "onclose" | "onerror" | "close">;
  url(workspaceId: string): string;
  onChange(workspaceId: string, paths: ChangedPaths, git: boolean): void;
  setTimer?(callback: () => void, delay: number): unknown;
  clearTimer?(timer: unknown): void;
}
const RETRY_MIN_MS = 1_000,
  RETRY_MAX_MS = 30_000;

/**
 * Keeps one socket open for each workspace passed to `sync`, reconnecting with
 * backoff. A host that cannot watch says so once and is left to the polling
 * that runs anyway.
 */
export function createFileWatch(options: FileWatchOptions) {
  const schedule = options.setTimer ?? ((callback, delay) => setTimeout(callback, delay));
  const cancel = options.clearTimer ?? (handle => clearTimeout(handle as ReturnType<typeof setTimeout>));
  interface Watch {
    socket?: ReturnType<FileWatchOptions["createSocket"]>;
    timer?: unknown;
    delay: number;
    connectedBefore: boolean;
    stopped: boolean;
  }
  const watches = new Map<string, Watch>();
  const unavailable = new Set<string>();

  function connect(id: string, watch: Watch) {
    if (watch.stopped) return;
    let socket: ReturnType<FileWatchOptions["createSocket"]>;
    try {
      socket = options.createSocket(options.url(id));
    } catch {
      retry(id, watch);
      return;
    }
    watch.socket = socket;
    socket.onmessage = event => {
      if (watch.stopped || watch.socket !== socket) return;
      let message: FileWatchMessage;
      try {
        message = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (message.type === "ready") {
        watch.delay = RETRY_MIN_MS;
        // Whatever changed while this view was away was never reported.
        if (watch.connectedBefore) options.onChange(id, null, true);
        watch.connectedBefore = true;
      } else if (message.type === "changed") {
        options.onChange(
          id,
          Array.isArray(message.paths) ? message.paths.filter(path => typeof path === "string") : null,
          !!message.git,
        );
      } else if (message.type === "unavailable") {
        unavailable.add(id);
        watch.stopped = true;
      }
    };
    socket.onerror = () => {};
    socket.onclose = () => {
      if (watch.socket === socket) {
        watch.socket = undefined;
        retry(id, watch);
      }
    };
  }
  function retry(id: string, watch: Watch) {
    if (watch.stopped) return;
    watch.timer = schedule(() => {
      watch.timer = undefined;
      connect(id, watch);
    }, watch.delay);
    watch.delay = Math.min(watch.delay * 2, RETRY_MAX_MS);
  }
  function stop(id: string) {
    const watch = watches.get(id);
    if (!watch) return;
    watch.stopped = true;
    watches.delete(id);
    if (watch.timer !== undefined) cancel(watch.timer);
    const socket = watch.socket;
    watch.socket = undefined;
    if (socket) {
      socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
      try {
        socket.close();
      } catch {
        /* already gone */
      }
    }
  }
  /** Watch exactly these workspaces. */
  function sync(ids: Iterable<string>) {
    const wanted = new Set([...ids].filter(id => id && !unavailable.has(id)));
    for (const id of [...watches.keys()]) if (!wanted.has(id)) stop(id);
    for (const id of wanted)
      if (!watches.has(id)) {
        const watch: Watch = { delay: RETRY_MIN_MS, connectedBefore: false, stopped: false };
        watches.set(id, watch);
        connect(id, watch);
      }
  }
  /**
   * Reconnect now rather than at the end of a backoff, e.g. when the tab
   * returns. `stale` also replaces sockets that still look open, which a
   * phone that slept leaves behind; the new `ready` reports a full resync.
   */
  function wake(stale = false) {
    for (const [id, watch] of watches) {
      if (stale && watch.socket) {
        const socket = watch.socket;
        watch.socket = undefined;
        socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
        try {
          socket.close();
        } catch {
          /* already gone */
        }
      } else if (watch.socket || watch.timer === undefined) continue;
      if (watch.timer !== undefined) {
        cancel(watch.timer);
        watch.timer = undefined;
      }
      watch.delay = RETRY_MIN_MS;
      connect(id, watch);
    }
  }
  function dispose() {
    for (const id of [...watches.keys()]) stop(id);
  }
  return { sync, wake, dispose };
}
