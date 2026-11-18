import { reactive } from "vue";

/**
 * A line a file pane should show once it has the file.
 *
 * Opening a file and moving to a line happen in different places: the chat
 * pane knows the line, the file pane opens later (or is already open), so the
 * request waits here, keyed by workspace and path, until that pane takes it.
 */
const jumps = reactive(new Map<string, { line: number; at: number }>());
const key = (workspaceId: string, path: string) => `${workspaceId}\u0000${path}`;
export function requestJump(workspaceId: string, path: string, line: number) {
  jumps.set(key(workspaceId, path), { line, at: Date.now() });
}
export function pendingJump(workspaceId: string, path: string) {
  return jumps.get(key(workspaceId, path));
}
export function takeJump(workspaceId: string, path: string) {
  const jump = jumps.get(key(workspaceId, path));
  jumps.delete(key(workspaceId, path));
  return jump?.line;
}

/** Character range of a 1-based line in `text`, clamped to the text. */
export function lineRange(text: string, line: number): [number, number] {
  let start = 0;
  for (let current = 1; current < line; current++) {
    const next = text.indexOf("\n", start);
    if (next < 0) break;
    start = next + 1;
  }
  const end = text.indexOf("\n", start);
  return [start, end < 0 ? text.length : end];
}
