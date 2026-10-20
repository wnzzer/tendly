import type { InjectionKey } from "vue";
import type { Session } from "@tendly/protocol";

/**
 * The session actions App owns (open, rename, environment, keep, run information),
 * offered to places that show a session without holding it -- a tab's
 * right-click menu -- so they list the same actions as the sidebar and the
 * session's own menu instead of sending the person to another menu first.
 */
export interface SessionActions {
  session(id: string): Session | undefined;
  /** Every session App knows of, and bringing one onto the canvas. */
  all(): Session[];
  open(session: Session): void;
  keepBusy(id: string): boolean;
  rename(id: string): void;
  environment(id: string): void;
  keep(id: string): void;
  info(id: string): void;
}
export const SESSION_ACTIONS: InjectionKey<SessionActions> = Symbol("session-actions");
