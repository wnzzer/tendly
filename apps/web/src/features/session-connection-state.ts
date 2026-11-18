import { reactive } from "vue";
import type { Session } from "@tendly/protocol";

export interface ConnectionEntry {
  state: "idle" | "opening" | "attached" | "ended" | "failed";
  epoch: number;
  explicit: boolean;
  generation: number;
  error?: string;
  pending?: Promise<Session | undefined>;
}

export const isLiveSession = (session: Session): boolean => ["running", "waiting", "starting"].includes(session.status);

export function createSessionConnections(connect: (session: Session) => Promise<Session>) {
  const entries = reactive(new Map<string, ConnectionEntry>());
  const get = (id: string): ConnectionEntry => {
    if (!entries.has(id)) entries.set(id, { state: "idle", epoch: 0, explicit: false, generation: 0 });
    return entries.get(id)!;
  };

  function requestOpen(id: string): void {
    const entry = get(id);
    // A second pane/open gesture joins the current attempt, not a deferred restart.
    if (entry.pending) return;
    entry.explicit = true;
    entry.epoch++;
    entry.state = "idle";
    entry.error = undefined;
  }

  function ended(id: string): void {
    const entry = get(id);
    entry.generation++;
    entry.state = "ended";
    entry.explicit = false;
    entry.error = undefined;
  }

  function ensure(session: Session): Promise<Session | undefined> {
    const entry = get(session.id);
    if (entry.pending) return entry.pending;
    const explicit = entry.explicit;
    entry.explicit = false;
    if (!explicit && (entry.state === "ended" || entry.state === "failed")) return Promise.resolve(undefined);
    if (!explicit && isLiveSession(session)) {
      entry.state = "attached";
      entry.error = undefined;
      return Promise.resolve(session);
    }
    if (!explicit) {
      // Restoring a layout is not consent to start even a newly-created stopped record.
      entry.state = session.status === "failed" ? "failed" : "ended";
      entry.error = session.status === "failed" ? (session.error ?? undefined) : undefined;
      return Promise.resolve(undefined);
    }
    entry.state = "opening";
    entry.error = undefined;
    const generation = ++entry.generation;
    // Explicit opens use the server's idempotent start to reconcile stale polling state.
    const operation = Promise.resolve()
      .then(() => (entry.generation === generation ? connect(session) : undefined))
      .then(result => {
        if (!result || entry.generation !== generation) return undefined;
        entry.state = isLiveSession(result) ? "attached" : result.status === "failed" ? "failed" : "ended";
        entry.error = result.error ?? undefined;
        return isLiveSession(result) ? result : undefined;
      })
      .catch(error => {
        if (entry.generation === generation) {
          entry.state = "failed";
          entry.error = error instanceof Error ? error.message : String(error);
        }
        return undefined;
      })
      .finally(() => {
        if (entry.pending === operation) entry.pending = undefined;
      });
    entry.pending = operation;
    return operation;
  }
  /** Attach a view to an already-live record without consuming an explicit
   * open intent or starting a stopped record. Used by the session view shell
   * when merely switching from chat to the native terminal view. */
  function attach(session: Session): Promise<Session | undefined> {
    const entry = get(session.id);
    if (entry.pending || entry.explicit) return Promise.resolve(undefined);
    if (isLiveSession(session)) {
      entry.state = "attached";
      entry.error = undefined;
      return Promise.resolve(session);
    }
    entry.state = session.status === "failed" ? "failed" : "ended";
    entry.error = session.status === "failed" ? (session.error ?? undefined) : undefined;
    return Promise.resolve(undefined);
  }
  return { get, requestOpen, ended, ensure, attach };
}
