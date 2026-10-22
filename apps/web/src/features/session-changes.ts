import { ref, watch, type Ref } from "vue";
import type { LayoutDocument, PaneNode, Session } from "@tendly/protocol";
import { flattenPanes } from "../layout/layout-engine";
import { ApiConnectionError, ApiError, json, request } from "./api";
import { backendCapabilities } from "./backend-capabilities";
import { conversationView, type ChatEvent } from "./chat-model";
import { conversationCache } from "./conversation-cache";
import { paneSession, paneString, renameSessionPanes } from "./pane-context";
import { sessionConnections } from "./session-connection";
import { isEphemeralSession } from "./session-list";
import { escapeHatchSource } from "./terminal-reopen";
import { showToast, type ToastAction } from "./toasts";

/**
 * What the shell does to sessions on the user's word: archive and restore,
 * delete, keep a temporary one, rename, discard a temporary window with its
 * tab, and switch a session to the structured chat. Each keeps the shared
 * session list and the open tabs in step with the server's answer.
 */

/** Which sessions start the moment their pane is placed. A structured chat
 * and a terminal are both asked for by the gesture that creates or opens
 * them, and a terminal that shows "ended" on arrival only ever wanted one
 * more tap; a legacy PTY shown in the conversation shell still waits for an
 * explicit reopen. */
export const opensOnArrival = (session: Session) => session.interaction_mode === "structured" || session.provider === "terminal";

/** A delete that found nothing to delete: the wanted end state, as long as the server, not the network, said so. */
export function isAlreadyGone(cause: unknown): boolean { return cause instanceof ApiError && cause.status === 404 && !(cause instanceof ApiConnectionError); }

/** The sessions a delete may go ahead with: the server deletes only temporary and archived ones, and one already being deleted is not asked twice. */
export function deletableSessions(list: readonly Session[], busyIds: readonly string[]): Session[] {
  return list.filter(session => !busyIds.includes(session.id) && (isEphemeralSession(session) || !!session.archived_at));
}

/** Closing a temporary window asks first only for a conversation in the middle of a turn. A terminal's process is
 * "running" for as long as it is open, which says nothing about work under way -- nor does a conversation's: its
 * client stays up between turns. What its view last held (`conversation`) tells whether a turn or a question is open. */
export function asksBeforeDiscard(session: Session, conversation?: readonly ChatEvent[]): boolean {
  if (session.interaction_mode !== "structured") return false;
  if (session.status === "starting" || session.status === "waiting") return true;
  if (session.status !== "running") return false;
  if (!conversation) return true;
  const view = conversationView(conversation, true);
  return view.turn === "running" || view.awaitingApproval;
}

/** The list with the server's copy of one session in place of its own. */
export function replaceSession(list: readonly Session[], updated: Session): Session[] { return list.map(item => item.id === updated.id ? updated : item); }

export interface SessionChangesOptions {
  sessions: Ref<Session[]>;
  layout: Ref<LayoutDocument>;
  canvas: Readonly<Ref<{ closePane(id: string): unknown } | undefined>>;
  report: (cause: unknown) => void;
  t: (key: string, values?: Record<string, string | number>) => string;
  /** The list is about to change here: a poll already under way must not overwrite it with what it fetched before. */
  touch: () => void;
  /** Show the conversation an escape-hatch terminal came from, once its tab has closed. */
  returnTo?: (sourceId: string) => void;
}

export function useSessionChanges({ sessions, layout, canvas, report, t, touch, returnTo }: SessionChangesOptions) {
  const archiveBusyIds = ref<string[]>([]), keepBusyIds = ref<string[]>([]), deleteBusyIds = ref<string[]>([]);
  const discardPrompt = ref<{ paneId: string; session: Session }>();
  const sessionNotice = ref(""), sessionNoticeValues = ref<Record<string, number>>();
  const enablingChat = new Set<string>(), discarding = new Set<string>();
  // A prompt outlives neither its pane nor its record: a poll that already removed
  // the session, or a pane closed elsewhere, must not leave a stale question.
  watch([sessions, layout], () => {
    const prompt = discardPrompt.value;
    if (prompt && (!sessions.value.some(item => item.id === prompt.session.id) || !flattenPanes(layout.value.root).some(pane => pane.id === prompt.paneId))) discardPrompt.value = undefined;
  }, { deep: true });
  /** What just happened, as a toast that goes away by itself; `sessionNotice` keeps the last one for the record. */
  function notice(message: string, values?: Record<string, number>, action?: ToastAction) {
    sessionNotice.value = message; sessionNoticeValues.value = values;
    if (message) showToast(t(message, values), { action });
  }
  /** The session is gone from the list, and any open intent retired with it so nothing can reconnect it later. */
  function forget(ids: readonly string[]) {
    const gone = new Set(ids);
    sessions.value = sessions.value.filter(item => !gone.has(item.id));
    for (const id of ids) sessionConnections.ended(id);
  }
  async function archiveSession(session: Session, archived: boolean, announce = true): Promise<boolean> {
    if (!backendCapabilities.sessionArchive || archiveBusyIds.value.includes(session.id)) return false;
    archiveBusyIds.value = [...archiveBusyIds.value, session.id];
    touch();
    if (announce) notice("");
    try {
      const updated = await request<Session>(`/sessions/${encodeURIComponent(session.id)}/archive`, json("PATCH", { archived }));
      touch();
      sessions.value = replaceSession(sessions.value, updated);
      if (announce) notice(archived ? "Session archived. History and running agents are unchanged." : "Session restored to its workspace list.", undefined, { label: t("Undo"), run: () => void archiveSession(updated, !archived) });
      return true;
    } catch (cause) {
      report(cause); return false;
    } finally {
      archiveBusyIds.value = archiveBusyIds.value.filter(id => id !== session.id);
    }
  }
  async function archiveSessions(list: Session[], archived: boolean) {
    if (list.length === 1) { await archiveSession(list[0]!, archived); return; }
    notice("");
    const done = (await Promise.all(list.map(session => archiveSession(session, archived, false)))).filter(Boolean).length;
    if (done) notice(archived ? "{count} sessions archived. History and running agents are unchanged." : "{count} sessions restored to their workspace lists.", { count: done }, { label: t("Undo"), run: () => void archiveSessions(list, !archived) });
  }
  /**
   * Delete sessions for good. The server accepts only temporary sessions and
   * archived ones, and stops anything still running first. Views bound to a
   * deleted session close with it, since there is nothing left to show.
   */
  async function deleteSessions(list: Session[]) {
    const targets = deletableSessions(list, deleteBusyIds.value);
    if (!targets.length) return;
    deleteBusyIds.value = [...deleteBusyIds.value, ...targets.map(session => session.id)];
    touch();
    notice("");
    const deleted: string[] = [];
    await Promise.all(targets.map(async session => {
      try { await request<{ id: string }>(`/sessions/${encodeURIComponent(session.id)}`, json("DELETE")); deleted.push(session.id); }
      catch (cause) { if (isAlreadyGone(cause)) deleted.push(session.id); else report(cause); }
    }));
    deleteBusyIds.value = deleteBusyIds.value.filter(id => !targets.some(session => session.id === id));
    if (!deleted.length) return;
    touch();
    forget(deleted);
    // Removed from `sessions` first, so the close guard has no record left to discard again.
    const gone = new Set(deleted);
    for (const pane of flattenPanes(layout.value.root).filter(pane => gone.has(paneString(pane, "session_id") ?? ""))) await canvas.value?.closePane(pane.id);
    notice(deleted.length === 1 ? "Session deleted." : "{count} sessions deleted.", { count: deleted.length });
  }
  /** The one place where closing a view ends a process — allowed only because the
   * user chose "temporary window" when the session was created. */
  async function discardEphemeral(session: Session): Promise<boolean> {
    try {
      await request<{ id: string }>(`/sessions/${encodeURIComponent(session.id)}`, json("DELETE"));
      touch(); forget([session.id]);
      return true;
    } catch (cause) {
      // Already gone (a restart discards temporary sessions) is the wanted end
      // state, so the view may close; anything else keeps the pane and reports.
      if (isAlreadyGone(cause)) { touch(); forget([session.id]); return true; }
      report(cause); return false;
    }
  }
  /** Close guard. A non-ephemeral pane keeps its existing view-only behaviour and
   * issues no request at all. A temporary window goes with its tab, unless it is
   * still working (`asksBeforeDiscard`): then the bar asks first. */
  async function confirmClosePane(pane: PaneNode): Promise<boolean> {
    const session = paneSession(pane, sessions.value);
    if (!session) return true;
    let closing = true;
    if (isEphemeralSession(session)) {
      if (asksBeforeDiscard(session, conversationCache.get(session.id)?.events)) { discardPrompt.value = { paneId: pane.id, session }; return false; }
      // A second close while the first is still discarding leaves the tab to
      // that one: an escape hatch left by its button exits under the DELETE,
      // and its exit asks to close the same tab again.
      if (discarding.has(session.id)) return false;
      discarding.add(session.id);
      try { closing = await discardEphemeral(session); } finally { discarding.delete(session.id); }
    }
    // An escape hatch is a detour from a conversation, so closing it goes back
    // there. After the canvas has settled what the close selects, not before.
    const source = escapeHatchSource(session);
    if (closing && source) setTimeout(() => returnTo?.(source));
    return closing;
  }
  async function confirmDiscard() {
    const prompt = discardPrompt.value;
    if (!prompt) return;
    discardPrompt.value = undefined;
    // The record is gone from `sessions` on success, so the guard now lets the
    // same pane close instead of prompting a second time.
    if (await discardEphemeral(prompt.session)) await canvas.value?.closePane(prompt.paneId);
  }
  async function keepSession(session: Session) {
    if (!isEphemeralSession(session) || keepBusyIds.value.includes(session.id)) return;
    keepBusyIds.value = [...keepBusyIds.value, session.id];
    touch();
    notice("");
    try {
      const updated = await request<Session>(`/sessions/${encodeURIComponent(session.id)}/keep`, json("POST"));
      touch();
      sessions.value = replaceSession(sessions.value, updated);
      if (discardPrompt.value?.session.id === updated.id) discardPrompt.value = undefined;
      notice("Session kept. It stays in the session list and closing its window no longer discards it.");
    } catch (cause) { report(cause); }
    finally { keepBusyIds.value = keepBusyIds.value.filter(id => id !== session.id); }
  }
  function keepSessionById(id: string) {
    const session = sessions.value.find(item => item.id === id);
    if (session) void keepSession(session);
  }
  async function renameSession(session: Session, title: string): Promise<boolean> {
    const nextTitle = title.trim();
    if (!nextTitle || nextTitle === session.title) return false;
    try {
      const updated = await request<Session>(`/sessions/${encodeURIComponent(session.id)}`, json("PATCH", { title: nextTitle }));
      sessions.value = replaceSession(sessions.value, updated);
      const nextRoot = renameSessionPanes(layout.value.root, updated.id, updated.title);
      if (nextRoot !== layout.value.root) layout.value = { ...layout.value, root: nextRoot };
      notice("Session renamed. Its ID, history and running process are unchanged.");
      return true;
    } catch (cause) { report(cause); return false; }
  }
  /** A legacy session asked to become a structured chat; a second ask while the first is under way is dropped. */
  async function enableChat(id: string) {
    if (enablingChat.has(id)) return; enablingChat.add(id);
    try { const updated = await request<Session>(`/sessions/${encodeURIComponent(id)}/conversation/open`, json("POST", {})); sessions.value = sessions.value.map(session => session.id === id ? updated : session); }
    catch (cause) { report(cause); } finally { enablingChat.delete(id); }
  }

  return { archiveBusyIds, keepBusyIds, deleteBusyIds, discardPrompt, sessionNotice, sessionNoticeValues, archiveSession, archiveSessions, deleteSessions, confirmClosePane, confirmDiscard, keepSession, keepSessionById, renameSession, enableChat };
}
