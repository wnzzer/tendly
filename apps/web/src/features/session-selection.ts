/**
 * Multi-select over an ordered list of ids. The order is the list as shown, so
 * a Shift range covers exactly the rows between the two clicks.
 */

/** Toggle `id`, or -- given the previous click as `anchor` -- set every id
 * from the anchor to `id` to the state `id` is toggled into. */
export function selectRange(
  order: readonly string[],
  checked: readonly string[],
  id: string,
  anchor?: string,
): string[] {
  const set = new Set(checked);
  const on = !set.has(id);
  const from = anchor === undefined ? -1 : order.indexOf(anchor),
    to = order.indexOf(id);
  const span = from < 0 || to < 0 ? [id] : order.slice(Math.min(from, to), Math.max(from, to) + 1);
  for (const item of span) {
    if (on) set.add(item);
    else set.delete(item);
  }
  return order.filter(item => set.has(item));
}

/** Everything shown that was not ticked, and nothing that was. */
export function invertSelection(order: readonly string[], checked: readonly string[]): string[] {
  const set = new Set(checked);
  return order.filter(item => !set.has(item));
}
