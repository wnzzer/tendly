import type { Session } from "@tendly/protocol";

/**
 * A view preference is local UI state, never persisted into the workspace
 * layout. It is kept in this browser, though, so a reload does not quietly
 * switch a session you were following in its terminal back to chat.
 */
export type SessionView = "chat" | "native";
const KEY = "tendly.session-views";
/** Enough for every session anyone has open; the oldest choices fall away first. */
const LIMIT = 200;
const preferences = new Map<string, SessionView>(load());

function load(): Array<[string, SessionView]> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(parsed)
      ? parsed.filter(
          (entry): entry is [string, SessionView] =>
            Array.isArray(entry) && typeof entry[0] === "string" && (entry[1] === "chat" || entry[1] === "native"),
        )
      : [];
  } catch {
    return [];
  }
}
function save() {
  try {
    localStorage.setItem(KEY, JSON.stringify([...preferences].slice(-LIMIT)));
  } catch {
    /* This page only. */
  }
}

export function defaultSessionView(session: Pick<Session, "provider" | "resume_source_id">): SessionView {
  if (session.provider === "terminal") return "native";
  // An escape-hatch session exists only to be a terminal: it was opened so an
  // interactive command could run somewhere the structured pipe cannot reach.
  // Its conversation is already on screen in the session it reopened.
  return session.resume_source_id ? "native" : "chat";
}

export function sessionView(
  session: Pick<Session, "id" | "provider" | "interaction_mode" | "resume_source_id">,
): SessionView {
  if (session.provider === "terminal") return "native";
  // Structured mode is owned by the conversation surface. A stale native
  // preference must not disguise it as a PTY.
  if (session.interaction_mode === "structured") return "chat";
  if (session.resume_source_id) return "native";
  return preferences.get(session.id) ?? defaultSessionView(session);
}

export function setSessionView(session: Pick<Session, "id" | "provider">, view: SessionView): SessionView {
  const next = session.provider === "terminal" ? "native" : view;
  preferences.delete(session.id);
  preferences.set(session.id, next);
  save();
  return next;
}

/** Test isolation and hot-reload cleanup; intentionally not exported to app code. */
export function resetSessionViewsForTests(): void {
  preferences.clear();
}
