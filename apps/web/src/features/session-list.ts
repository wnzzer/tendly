import type { ProviderKind, Session, Workspace } from "@tendly/protocol";
import { providerLabel } from "./clients";
import { fuzzyScore } from "./fuzzy-search";

export type SessionArchiveFilter = "current" | "archived" | "all";
/** Temporary windows are kept out of workspace lists, so the library is the only
 * place they can be found again; "all" therefore stays the default here. */
export type SessionEphemeralFilter = "all" | "only" | "hidden";

export interface SessionListOptions {
  query?: string;
  workspaceId?: string;
  provider?: ProviderKind | "";
  status?: Session["status"] | "";
  archive?: SessionArchiveFilter;
  ephemeral?: SessionEphemeralFilter;
  statusLabels?: Partial<Record<Session["status"], string>>;
}

export interface SessionListEntry {
  session: Session;
  workspace?: Workspace;
}

export function isSessionArchived(session: Pick<Session, "archived_at">): boolean {
  return Boolean(session.archived_at);
}

/** An old backend omits the field entirely; absent is never temporary. */
export function isEphemeralSession(session: Pick<Session, "ephemeral">): boolean {
  return session.ephemeral === true;
}

/** Search only the session metadata already loaded by the parent; never fetch history or run a CLI. */
export function filterSessionList(
  sessions: readonly Session[],
  workspaces: readonly Workspace[],
  options: SessionListOptions = {},
): SessionListEntry[] {
  const workspaceById = new Map(workspaces.map(workspace => [workspace.id, workspace]));
  const seen = new Set<string>();
  const archive = options.archive ?? "current",
    ephemeral = options.ephemeral ?? "all";
  const matches: (SessionListEntry & { score: number; index: number; updated: number })[] = [];
  sessions.forEach((session, index) => {
    if (seen.has(session.id)) return;
    seen.add(session.id);
    const archived = isSessionArchived(session);
    if ((archive === "current" && archived) || (archive === "archived" && !archived)) return;
    const temporary = isEphemeralSession(session);
    if ((ephemeral === "only" && !temporary) || (ephemeral === "hidden" && temporary)) return;
    if (options.workspaceId && session.workspace_id !== options.workspaceId) return;
    if (options.provider && session.provider !== options.provider) return;
    if (options.status && session.status !== options.status) return;
    const workspace = workspaceById.get(session.workspace_id);
    const score = fuzzyScore(
      options.query ?? "",
      [
        session.title,
        session.id,
        providerLabel(session.provider),
        session.provider,
        workspace?.name,
        workspace?.root_path,
        session.workspace_id,
        session.status,
        options.statusLabels?.[session.status],
      ]
        .filter(Boolean)
        .join(" "),
    );
    if (score === null) return;
    const timestamp = Date.parse(session.updated_at || session.created_at);
    matches.push({ session, workspace, score, index, updated: Number.isFinite(timestamp) ? timestamp : 0 });
  });
  return matches
    .sort((first, second) => first.score - second.score || second.updated - first.updated || first.index - second.index)
    .map(({ session, workspace }) => ({ session, workspace }));
}

/**
 * The command that continues a session in a terminal of one's own, on the
 * machine Tendly runs on (`tendly resume`, server resume.rs): by the
 * Tendly session ID, which carries its client, endpoint and checkout. A
 * client session needs a conversation to continue; a shell always has one.
 */
export function resumeCommand(session: Pick<Session, "id" | "provider" | "provider_session_id">): string | undefined {
  return session.provider === "terminal" || session.provider_session_id ? `tendly resume ${session.id}` : undefined;
}
