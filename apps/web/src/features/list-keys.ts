/**
 * ↑ ↓ Home End between the rows of a list that are not one focusable widget:
 * commits, stashes, sessions. `rows` matches the focusable row elements
 * inside `root`. Returns whether the key moved the focus.
 */
export function moveInList(event: KeyboardEvent, root: ParentNode | null | undefined, rows: string): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
  const list = Array.from(root?.querySelectorAll<HTMLElement>(rows) ?? []);
  const index = list.indexOf(event.currentTarget as HTMLElement);
  const next = event.key === "ArrowDown" ? index + 1 : event.key === "ArrowUp" ? index - 1 : event.key === "Home" ? 0 : event.key === "End" ? list.length - 1 : undefined;
  if (next === undefined || !list.length) return false;
  event.preventDefault();
  list[Math.max(0, Math.min(list.length - 1, next))]?.focus();
  return true;
}
