import { reactive } from "vue";

export interface FileDraft { content: string; original: string; version: string; loaded: boolean; }
/** Drafts survive tab/split moves. They are deliberately not written to localStorage. */
const drafts = reactive(new Map<string, FileDraft>());
/** `place`: the workspace and checkout the file is in (`placeKey`). */
export function fileDraft(place: string, path: string): FileDraft {
  const key = `${place}:${path}`;
  if (!drafts.has(key)) drafts.set(key, { content: "", original: "", version: "", loaded: false });
  return drafts.get(key)!;
}
export function hasDirtyDrafts(workspaceId?: string): boolean {
  return [...drafts.entries()].some(([key, value]) => (!workspaceId || key.startsWith(`${workspaceId}:`)) && value.content !== value.original);
}
/** A file or folder renamed in place: its drafts, and those of files under it, go with it. */
export function moveFileDrafts(place: string, from: string, to: string) {
  for (const [key, value] of [...drafts.entries()]) {
    const path = key.startsWith(`${place}:`) ? key.slice(place.length + 1) : undefined;
    const moved = path === from ? to : path?.startsWith(`${from}/`) ? to + path.slice(from.length) : undefined;
    if (moved === undefined) continue;
    drafts.delete(key); drafts.set(`${place}:${moved}`, value);
  }
}
