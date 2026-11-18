/** In-memory only: host paths are never persisted to browser storage. */
export interface TreeViewState {
  expanded: Set<string>;
  loadedDirectories: Set<string>;
  selectedPath: string;
  focusedPath: string;
  query: string;
}

const views = new Map<string, TreeViewState>();

function copy(state: TreeViewState): TreeViewState {
  return { ...state, expanded: new Set(state.expanded), loadedDirectories: new Set(state.loadedDirectories) };
}

export function readTreeViewState(workspaceId: string): TreeViewState {
  const state = views.get(workspaceId);
  return state
    ? copy(state)
    : { expanded: new Set(), loadedDirectories: new Set(), selectedPath: "", focusedPath: "", query: "" };
}

export function saveTreeViewState(workspaceId: string, state: TreeViewState): void {
  // Remember shape and selection, never file entries; remounts revalidate with the host.
  views.delete(workspaceId);
  views.set(workspaceId, copy(state));
  if (views.size > 100) views.delete(views.keys().next().value!);
}
