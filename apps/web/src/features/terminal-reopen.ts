import type { Session } from "@tendly/protocol";
import { json, request } from "./api";

/**
 * Open a structured session's conversation in a real terminal client.
 *
 * Structured mode drives the same client over a JSON pipe, which is exactly what
 * makes an interactive command such as `/config` unusable: the pipe carries one
 * turn at a time, and a full-screen TUI has nowhere to run. Rather than trying
 * to imitate that TUI over the pipe, this asks the server for a genuine terminal
 * session on the same conversation — same client, same account, same
 * configuration home, same native session id.
 *
 * It returns a *new* session rather than converting the current one, so the
 * conversation pane is left untouched and its history survives closing the
 * terminal.
 */
export async function terminalReopen(session: Pick<Session, "id">): Promise<Session> {
  const opened = await request<Session>(`/sessions/${encodeURIComponent(session.id)}/terminal`, json("POST"));
  // Creating a session launches nothing, here as everywhere else. Starting it
  // is a separate step through the ordinary route, which already reports a
  // client that will not run; the terminal is then live when its pane opens.
  return await request<Session>(`/sessions/${encodeURIComponent(opened.id)}/start`, json("POST"));
}

/**
 * Whether the escape hatch is worth offering.
 *
 * A conversation only exists once the client has reported a session id, so
 * before the first turn there is nothing to resume and the server would refuse.
 * Offering the action anyway would be a button whose only outcome is an error.
 */
export function canReopenInTerminal(session: Pick<Session, "provider" | "interaction_mode" | "provider_session_id">, capabilities: { sessionTerminalEscape?: boolean } = {}): boolean {
  if (capabilities.sessionTerminalEscape !== true) return false;
  if (session.provider === "terminal") return false;
  if (session.interaction_mode !== "structured") return false;
  return !!session.provider_session_id;
}

/**
 * The conversation an escape hatch is a detour from, or nothing for any other
 * session. Leaving the terminal (its client exits, its tab closes) returns
 * there: the terminal was only ever a way into that conversation.
 */
export function escapeHatchSource(session: Pick<Session, "provider" | "resume_source_id">): string | undefined {
  return session.provider !== "terminal" && session.resume_source_id ? session.resume_source_id : undefined;
}

/**
 * The escape hatch open on `source`'s conversation right now, if any. While it
 * runs, two clients hold one conversation: a message sent from the
 * conversation view meanwhile is not part of what the terminal's client knows.
 */
export function liveEscapeHatch(source: Pick<Session, "id">, sessions: readonly Session[]): Session | undefined {
  return sessions.find(session => escapeHatchSource(session) === source.id && ["starting", "running", "waiting"].includes(session.status));
}
