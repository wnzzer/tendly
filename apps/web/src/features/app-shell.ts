/**
 * Decisions the app shell makes, as plain functions: where a file an agent
 * named should open, where Back goes on a phone, how wide the workspace
 * column may be, and whether leaving the page would lose work.
 */

/** Where a file reference resolves: a path inside one workspace -- in its directory, or in the worktree `checkout` -- a read-only host file, or a bare name to look up. */
export type ReferenceTarget =
  | { kind: "workspace"; workspaceId: string; checkout: string | null; path: string }
  | { kind: "host"; path: string }
  | { kind: "search"; workspaceId: string; checkout: string | null; name: string };

/**
 * A path as a Windows host and a reference can disagree on it: `C:\repo`
 * against `C:/repo/`, and in any case. One form, compared without case where
 * there is a drive letter, as that filesystem does.
 */
function folded(value: string): string {
  const plain = value.replace(/\\/g, "/").replace(/\/$/, "");
  return /^[A-Za-z]:/.test(plain) ? plain.toLowerCase() : plain;
}

/**
 * Where a file a chat message pointed at opens. An absolute path inside the
 * workspace (or the session's own checkout) becomes relative to it, and opens
 * in that checkout; one inside another workspace opens there, in the most
 * specific of them; any other absolute path is shown read-only rather than
 * refused, since the agent named it for a reason. A relative path or a bare
 * name is the session's: it opens, or is looked up, in its checkout.
 */
export function resolveReference(
  reference: { path: string; checkout?: string | null },
  workspace: { id: string; root_path: string },
  workspaces: readonly { id: string; root_path: string }[],
): ReferenceTarget {
  const path = reference.path.replace(/^\.\//, "");
  if (/^([A-Za-z]:)?\//.test(path)) {
    const inside = (root?: string | null) => !!root && folded(path).startsWith(folded(root) + "/");
    const base = [workspace.root_path, reference.checkout].find(inside);
    if (base) return { kind: "workspace", workspaceId: workspace.id, checkout: base === workspace.root_path ? null : base, path: path.slice(folded(base).length + 1) };
    const other = workspaces.filter(item => inside(item.root_path)).sort((a, b) => b.root_path.length - a.root_path.length)[0];
    if (other) return { kind: "workspace", workspaceId: other.id, checkout: null, path: path.slice(folded(other.root_path).length + 1) };
    return { kind: "host", path };
  }
  const checkout = reference.checkout ?? null;
  if (!path.includes("/")) return { kind: "search", workspaceId: workspace.id, checkout, name: path };
  return { kind: "workspace", workspaceId: workspace.id, checkout, path };
}

/** The views a phone opens like a pushed page, with Back to where they came from. */
const PUSHED_KINDS = ["git_diff", "editor", "file_preview"];

/** The pane history after moving from `previous` to `current`: each pane once, most recent last, at most twenty. */
export function rememberPane(history: readonly string[], previous: string, current: string | undefined): string[] {
  return [...history.filter(item => item !== previous && item !== current), previous].slice(-20);
}

/** Where Back goes from the pane in view: the most recent other pane still open, for a pushed view only. */
export function backTargetFor(focused: { id: string; kind: string } | undefined, history: readonly string[], openIds: ReadonlySet<string>): string | undefined {
  if (!focused || !PUSHED_KINDS.includes(focused.kind)) return undefined;
  return [...history].reverse().find(id => openIds.has(id) && id !== focused.id);
}

/** The workspace column's width bounds, in CSS pixels. */
export const SIDEBAR_MIN = 184, SIDEBAR_MAX = 380;

/** A remembered column width, if it is one the column can take. */
export function storedSidebarWidth(raw: string | null | undefined): number | undefined {
  const value = Number(raw);
  return value >= SIDEBAR_MIN && value <= SIDEBAR_MAX ? value : undefined;
}

/** The column's width for a drag reaching `pointerX`, from its left edge at `left`. */
export function draggedSidebarWidth(pointerX: number, left: number): number {
  return Math.round(Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, pointerX - left)));
}

/** The canvas states in which leaving the page would lose the layout: still saving, failed, in conflict, or kept nowhere. */
const UNSAVED_LAYOUT = ["Saving…", "Save failed", "Save conflict", "Memory only"];

/** Whether leaving the page now would lose work: an unsaved file or commit draft, or a layout not yet saved. */
export function hasUnsavedWork(state: { fileDrafts: boolean; gitDrafts: boolean; layoutPending: boolean; layoutStatus: string }): boolean {
  return state.fileDrafts || state.gitDrafts || state.layoutPending || UNSAVED_LAYOUT.includes(state.layoutStatus);
}
