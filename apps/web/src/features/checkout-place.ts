import { inCheckout, workspacePath } from "./api";

/**
 * Where a view looks: a workspace, in its own directory or in a worktree a
 * session works in. One string, so what the shell keeps per workspace -- Git
 * status, reload counters, file sockets, drafts -- keeps one per place, and a
 * file open in a worktree is never mistaken for the same path at home.
 */
const SEPARATOR = "\u0000";
export const placeKey = (workspaceId: string, checkout?: string | null) => checkout ? `${workspaceId}${SEPARATOR}${checkout}` : workspaceId;
export function placeOf(key: string): { workspaceId: string; checkout: string | null } {
  const cut = key.indexOf(SEPARATOR);
  return cut === -1 ? { workspaceId: key, checkout: null } : { workspaceId: key.slice(0, cut), checkout: key.slice(cut + 1) };
}
/** `/workspaces/:id<rest>`, in the place's checkout. */
export function placeUrl(key: string, rest: string): string {
  const { workspaceId, checkout } = placeOf(key);
  return inCheckout(`${workspacePath(workspaceId)}${rest}`, checkout);
}
