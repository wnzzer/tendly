import type { PaneNode, Session, Workspace } from "@tendly/protocol";
import { providerLabel } from "./clients";
import { fuzzyScore } from "./fuzzy-search";
import { isSessionArchived } from "./session-list";

export interface WorkspaceGroupPreferences {
  version: 1;
  pinned: string[];
  /** Only explicit user choices; absent keys retain the selected/active default. */
  expanded: Record<string, boolean>;
  /** Workspaces whose "Earlier" fold is open. Absent in what older versions stored. */
  olderExpanded: Record<string, boolean>;
}

export interface WorkspaceGroup {
  workspace: Workspace;
  sessions: Session[];
  /** `sessions` split by when each was last in use: the recent ones are listed,
   * the older ones wait behind a fold. Searching never folds anything away. */
  recentSessions: Session[];
  olderSessions: Session[];
  totalSessions: number;
  activeCount: number;
  /** Sessions waiting on you: an approval or a question. */
  waitingCount: number;
  pinned: boolean;
  expanded: boolean;
  matchesWorkspace: boolean;
}

export interface WorkspaceGroupOptions {
  query?: string;
  selectedWorkspaceId?: string;
  preferences?: WorkspaceGroupPreferences;
  /** Search expansion is temporary and must not overwrite the normal layout. */
  searchExpanded?: Record<string, boolean>;
  /** The session open on the canvas stays in view however long it has been idle. */
  selectedSessionId?: string;
  /** A session not used for this many days moves behind its workspace's "Earlier" fold; 0 never folds. */
  recentDays?: number;
  /** For tests; defaults to the current time. */
  now?: number;
}

export function emptyWorkspaceGroupPreferences(): WorkspaceGroupPreferences {
  return {
    version: 1,
    pinned: [],
    expanded: Object.create(null) as Record<string, boolean>,
    olderExpanded: Object.create(null) as Record<string, boolean>,
  };
}

const active = (session: Session) => ["starting", "running", "waiting"].includes(session.status);

/** Running, temporary or selected sessions are always recent; so is one whose time cannot be read. */
function isRecent(session: Session, cutoff: number, selectedSessionId?: string): boolean {
  if (active(session) || session.ephemeral === true || session.id === selectedSessionId) return true;
  const at = Date.parse(session.updated_at || session.created_at);
  return !Number.isFinite(at) || at >= cutoff;
}

/** Two levels only: known workspaces and their own sessions. Search stays entirely local. */
export function groupWorkspaces(
  workspaces: readonly Workspace[],
  sessions: readonly Session[],
  options: WorkspaceGroupOptions = {},
): WorkspaceGroup[] {
  const preferences = options.preferences ?? emptyWorkspaceGroupPreferences();
  const query = options.query?.trim() ?? "";
  const grouped = new Map<string, Session[]>();
  const seenSessions = new Set<string>();
  for (const session of sessions) {
    if (seenSessions.has(session.id)) continue;
    seenSessions.add(session.id);
    // Archiving is an explicit request to hide a session. A temporary one is
    // the opposite: it is open and in use right now, and it disappears by
    // itself when closed, so hiding it only made it unmanageable.
    if (isSessionArchived(session)) continue;
    const group = grouped.get(session.workspace_id) ?? [];
    group.push(session);
    grouped.set(session.workspace_id, group);
  }
  const recentDays = options.recentDays ?? 3;
  const cutoff = recentDays > 0 ? (options.now ?? Date.now()) - recentDays * 86_400_000 : -Infinity;
  const pinned = new Map(preferences.pinned.map((id, index) => [id, index]));
  return workspaces
    .map((workspace, index) => {
      const allSessions = grouped.get(workspace.id) ?? [];
      const activeCount = allSessions.filter(active).length;
      const waitingCount = allSessions.filter(session => session.status === "waiting").length;
      const workspaceScore = fuzzyScore(query, `${workspace.name} ${workspace.root_path} ${workspace.id}`);
      const matchesWorkspace = workspaceScore !== null;
      const matches = allSessions
        .map((session, sessionIndex) => ({
          session,
          index: sessionIndex,
          score: fuzzyScore(
            query,
            `${session.title} ${session.id} ${providerLabel(session.provider)} ${session.provider} ${workspace.name} ${workspace.root_path} ${session.status}`,
          ),
        }))
        .filter((item): item is { session: Session; index: number; score: number } => item.score !== null)
        .sort((a, b) => a.score - b.score || a.index - b.index);
      const visibleSessions = matchesWorkspace ? allSessions.slice() : matches.map(item => item.session);
      const defaultExpanded = workspace.id === options.selectedWorkspaceId || activeCount > 0;
      const normalExpanded = Object.hasOwn(preferences.expanded, workspace.id)
        ? preferences.expanded[workspace.id]
        : defaultExpanded;
      const expanded = query
        ? options.searchExpanded && Object.hasOwn(options.searchExpanded, workspace.id)
          ? options.searchExpanded[workspace.id]
          : true
        : normalExpanded;
      const score = Math.min(workspaceScore ?? Infinity, matches[0]?.score ?? Infinity);
      const recentSessions = query
        ? visibleSessions
        : visibleSessions.filter(session => isRecent(session, cutoff, options.selectedSessionId));
      const olderSessions = query
        ? []
        : visibleSessions.filter(session => !isRecent(session, cutoff, options.selectedSessionId));
      const group: WorkspaceGroup = {
        workspace,
        sessions: visibleSessions,
        recentSessions,
        olderSessions,
        totalSessions: allSessions.length,
        activeCount,
        waitingCount,
        pinned: pinned.has(workspace.id),
        expanded,
        matchesWorkspace,
      };
      return { group, index, score };
    })
    .filter(item => !query || item.group.matchesWorkspace || item.group.sessions.length)
    .sort(
      (a, b) =>
        Number(b.group.pinned) - Number(a.group.pinned) ||
        (a.group.pinned && b.group.pinned
          ? pinned.get(a.group.workspace.id)! - pinned.get(b.group.workspace.id)!
          : 0) ||
        (query ? a.score - b.score : 0) ||
        a.index - b.index,
    )
    .map(item => item.group);
}

export function workspaceSessionPane(session: Session): PaneNode {
  return {
    type: "pane",
    id: `session-${session.id}`,
    kind: session.provider === "terminal" ? "terminal" : "agent_chat",
    title: session.title,
    metadata: { workspace_id: session.workspace_id, session_id: session.id, provider: session.provider },
  };
}

type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;
const memory = new Map<string, WorkspaceGroupPreferences>();
const validId = (value: unknown): value is string =>
  typeof value === "string" && value.length > 0 && value.length <= 256 && !value.includes("\0");
const copyPreferences = (value: WorkspaceGroupPreferences): WorkspaceGroupPreferences => ({
  version: 1,
  pinned: [...value.pinned],
  expanded: Object.assign(Object.create(null), value.expanded),
  olderExpanded: Object.assign(Object.create(null), value.olderExpanded),
});

export function parseWorkspaceGroupPreferences(value: unknown): WorkspaceGroupPreferences {
  const result = emptyWorkspaceGroupPreferences();
  if (!value || typeof value !== "object" || Array.isArray(value)) return result;
  const record = value as Record<string, unknown>;
  if (record.version !== 1) return result;
  if (Array.isArray(record.pinned)) result.pinned = [...new Set(record.pinned.filter(validId))].slice(0, 2000);
  for (const field of ["expanded", "olderExpanded"] as const) {
    const value = record[field];
    if (!value || typeof value !== "object" || Array.isArray(value)) continue;
    for (const [id, expanded] of Object.entries(value).slice(0, 2000))
      if (validId(id) && typeof expanded === "boolean") result[field][id] = expanded;
  }
  return result;
}

export function workspaceGroupStorageKey(deployment: string): string {
  return `tendly.workspace-groups.v1:${encodeURIComponent(deployment)}`;
}

/** Persist only opaque IDs, not paths/titles. Denied/quota-limited storage stays usable in memory. */
export function loadWorkspaceGroupPreferences(key: string, storage?: PreferenceStorage): WorkspaceGroupPreferences {
  const remembered = memory.get(key);
  if (remembered) return copyPreferences(remembered);
  let preferences = emptyWorkspaceGroupPreferences();
  try {
    const raw = storage?.getItem(key);
    if (raw && raw.length <= 256_000) preferences = parseWorkspaceGroupPreferences(JSON.parse(raw));
  } catch {
    /* Corrupt or unavailable storage starts with safe defaults. */
  }
  memory.set(key, copyPreferences(preferences));
  return preferences;
}

export function saveWorkspaceGroupPreferences(
  key: string,
  value: WorkspaceGroupPreferences,
  storage?: PreferenceStorage,
): void {
  const preferences = parseWorkspaceGroupPreferences(value);
  memory.set(key, copyPreferences(preferences));
  try {
    storage?.setItem(key, JSON.stringify(preferences));
  } catch {
    /* Memory retains the user's choices for this page lifetime. */
  }
}
