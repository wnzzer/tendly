import type { LayoutDocument, LayoutNode, PaneNode, Session, Workspace } from "@tendly/protocol";
import { closePane, flattenPanes, restoreCollapsed, updatePane, validateLayout } from "../layout/layout-engine";
import { isEphemeralSession } from "./session-list";

export function paneString(pane: PaneNode, key: string): string | undefined {
  const value = pane.metadata?.[key];
  return typeof value === "string" && value ? value : undefined;
}
export function sessionPane(session: Session): PaneNode {
  return { type: "pane", id: `session-${session.id}`, kind: session.provider === "terminal" ? "terminal" : "agent_chat", title: session.title,
    metadata: { workspace_id: session.workspace_id, session_id: session.id, provider: session.provider } };
}
/** Update every canvas tab bound to a session without changing its pane ID. */
export function renameSessionPanes(root: LayoutNode, sessionId: string, title: string): LayoutNode {
  if (root.type === "pane") return paneString(root, "session_id") === sessionId && root.title !== title ? { ...root, title } : root;
  if (root.type === "stack") {
    let changed = false;
    const panes = root.panes.map(pane => {
      const next = renameSessionPanes(pane, sessionId, title) as PaneNode;
      changed ||= next !== pane;
      return next;
    });
    return changed ? { ...root, panes } : root;
  }
  const first = renameSessionPanes(root.first, sessionId, title);
  const second = renameSessionPanes(root.second, sessionId, title);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}
/**
 * The view a session is actually bound to. A layout restored from an older
 * document can carry a pane ID that predates `session-<id>`, so locating must
 * follow the binding rather than assume the canonical ID exists.
 */
export function findSessionPaneId(root: LayoutNode, sessionId: string): string | undefined {
  if (root.type === "pane") return paneString(root, "session_id") === sessionId ? root.id : undefined;
  if (root.type === "stack") {
    for (const pane of root.panes) {
      const id = findSessionPaneId(pane, sessionId);
      if (id) return id;
    }
    return undefined;
  }
  return findSessionPaneId(root.first, sessionId) ?? findSessionPaneId(root.second, sessionId);
}
/** A worktree's views are their own panes: the same path in another checkout is another file. */
const inTree = (checkout?: string | null) => checkout ? `@${encodeURIComponent(checkout)}` : "";
export function filePane(workspaceId: string, path: string, checkout?: string | null): PaneNode {
  return { type: "pane", id: `file-${encodeURIComponent(workspaceId)}${inTree(checkout)}:${encodeURIComponent(path)}`,
    kind: /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif|mp4|webm|mov|m4v|ogv|mp3|wav|ogg|m4a|flac|pdf)$/i.test(path) ? "file_preview" : "editor",
    title: path.split("/").pop(), metadata: { workspace_id: workspaceId, path, ...(checkout ? { checkout } : {}) } };
}
export function changesPane(workspaceId: string, checkout?: string | null): PaneNode {
  return { type: "pane", id: `changes-${encodeURIComponent(workspaceId)}${inTree(checkout)}`, kind: "git_diff", title: "Changes", metadata: { workspace_id: workspaceId, ...(checkout ? { checkout } : {}) } };
}
/** The checkout a pane looks at: its session's worktree, or the one a file or Git pane was opened in; `null` for the workspace directory. */
export function paneCheckout(pane: PaneNode, sessions: Session[]): string | null {
  const session = paneSession(pane, sessions);
  if (session) return session.checkout_path ?? null;
  return paneString(pane, "checkout") ?? null;
}
export function paneSession(pane: PaneNode, sessions: Session[]): Session | undefined {
  if (pane.kind !== "agent_chat" && pane.kind !== "terminal") return undefined;
  const session = sessions.find(session => session.id === paneString(pane, "session_id"));
  if (!session || paneString(pane, "workspace_id") !== session.workspace_id) return undefined;
  if ((pane.kind === "terminal") !== (session.provider === "terminal")) return undefined;
  return session;
}
/** No fallback to whichever workspace is currently selected in the sidebar. */
export function paneWorkspace(pane: PaneNode, workspaces: Workspace[], sessions: Session[]): Workspace | undefined {
  const bound = paneString(pane, "workspace_id");
  if (!bound) return undefined;
  if (paneString(pane, "session_id") && !paneSession(pane, sessions)) return undefined;
  return workspaces.find(workspace => workspace.id === bound);
}
export function ephemeralSessionIds(sessions: readonly Session[]): string[] {
  return sessions.filter(isEphemeralSession).map(session => session.id);
}
/**
 * The persisted copy of a layout, with each temporary window's pane marked.
 *
 * A temporary window stays open across a reload: its session lives on until
 * its tab is closed (or the server restarts and discards it). Leaving the pane
 * out instead lost the only way to close it, and left the session running
 * unseen. The mark lets a later load drop the pane once its session is gone
 * (withoutDiscardedPanes), rather than reopen a view of nothing.
 */
export function markEphemeralPanes(document: LayoutDocument, sessions: readonly Session[]): LayoutDocument {
  const ids = new Set(ephemeralSessionIds(sessions));
  if (!ids.size) return document;
  const marked = flattenPanes(document.root).filter(pane => { const id = paneString(pane, "session_id"); return !!id && ids.has(id) && pane.metadata?.ephemeral !== true; });
  if (!marked.length) return document;
  let root = document.root;
  for (const pane of marked) root = updatePane(root, { ...pane, metadata: { ...pane.metadata, ephemeral: true } });
  return { ...document, root };
}

/** `root` without the panes of temporary windows whose session was discarded. */
export function withoutDiscardedPanes(root: LayoutNode, sessions: readonly Session[]): LayoutNode {
  const known = new Set(sessions.map(session => session.id));
  const gone = flattenPanes(root).filter(pane => { const id = paneString(pane, "session_id"); return pane.metadata?.ephemeral === true && !!id && !known.has(id); });
  for (const pane of gone) root = closePane(root, pane.id);
  return root;
}
export function acceptsScopedPane(pane: PaneNode, workspaces: Workspace[], sessions: Session[]): boolean {
  // Its workspace is on the machine it came from.
  if (pane.metadata?.remote_session_id) return true;
  if (!paneWorkspace(pane, workspaces, sessions)) return false;
  const path = pane.metadata?.path;
  return path === undefined || (typeof path === "string" && !!path && !path.startsWith("/") && !path.includes("\0") && !path.split("/").includes(".."));
}

/** One-time migration, in a new document. Old per-workspace layouts stay untouched. */
export function scopeLegacyLayout(document: LayoutDocument, ownerId: string, sessions: Session[]): LayoutDocument {
  if (!validateLayout(document)) throw new Error("Invalid saved layout");
  const restored = restoreCollapsed(document);
  const ids = new Set<string>();
  const renamed = new Map<string, string>();
  function unique(preferred: string) {
    let id = preferred, suffix = 1;
    while (ids.has(id)) id = `${preferred}-${++suffix}`;
    ids.add(id); return id;
  }
  function visit(node: LayoutNode): LayoutNode {
    if (node.type === "split") return { ...node, id: unique(node.id), first: visit(node.first), second: visit(node.second) };
    if (node.type === "stack") {
      const id = unique(node.id), panes = node.panes.map(pane => visit(pane) as PaneNode);
      const { collapsedFrom: _projection, ...stack } = node;
      return { ...stack, id, panes, activePaneId: node.activePaneId ? renamed.get(node.activePaneId) : undefined };
    }
    const sessionId = paneString(node, "session_id");
    const session = sessions.find(session => session.id === sessionId);
    const workspaceId = paneString(node, "workspace_id") ?? session?.workspace_id ?? ownerId;
    const metadata = { ...node.metadata, workspace_id: workspaceId };
    const path = paneString(node, "path");
    const preferred = sessionId ? `session-${sessionId}` : path ? filePane(workspaceId, path).id : node.kind === "git_diff" ? changesPane(workspaceId).id : node.id;
    const id = unique(preferred); renamed.set(node.id, id);
    return { ...node, id, metadata };
  }
  const result = { version: restored.version, root: visit(restored.root) };
  if (!validateLayout(result)) throw new Error("Invalid migrated layout");
  return result;
}

/** The names built-in panes are shown under when they carry no title of their own. */
const PANE_NAMES: Record<string, string> = { agent_chat: "Agent session", git_diff: "Git Changes", editor: "File editor", file_preview: "File preview", terminal: "Terminal" };

/**
 * What a pane is called on screen. A session's or a file's title is its own
 * and shown as it is; built-in names, stored in English so a layout reads the
 * same in any language, are translated.
 */
export function paneDisplayTitle(pane: PaneNode, translate: (text: string) => string): string {
  if (pane.title && (pane.metadata?.session_id || pane.metadata?.path)) return pane.title;
  return translate(pane.title || PANE_NAMES[pane.kind] || pane.kind);
}

/**
 * An open pane `incoming` can take the place of: one of the same kind that
 * shows nothing yet -- no session, no file, no unsent draft -- and belongs to
 * no other workspace. Git changes are never replaced: each workspace keeps its
 * own.
 */
export function reusablePane(open: readonly PaneNode[], incoming: PaneNode): PaneNode | undefined {
  if (incoming.kind === "git_diff") return undefined;
  return open.find(candidate => candidate.kind === incoming.kind
    && !candidate.metadata?.session_id && !candidate.metadata?.remote_session_id && !candidate.metadata?.path && !candidate.metadata?.draft
    && (!candidate.metadata?.workspace_id || candidate.metadata.workspace_id === incoming.metadata?.workspace_id));
}
