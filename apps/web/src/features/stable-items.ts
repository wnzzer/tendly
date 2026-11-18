import type { DisplayItem } from "./tool-runs";

/**
 * Rows of the conversation that keep their identity while they are unchanged.
 *
 * The view is rebuilt from the events every time one arrives, which makes a
 * new object for every row; to a renderer that compares by identity, every
 * row then looks changed, and a reply streaming into the last row re-renders
 * all of them. Here a row equal to the one shown before is that same object,
 * so only what actually changed is rendered again.
 */
export function createStableRows() {
  let previous = new Map<string, DisplayItem>();
  return function stabilize(next: readonly DisplayItem[]): { rows: DisplayItem[]; keys: string[] } {
    const seen = new Map<string, number>(),
      current = new Map<string, DisplayItem>(),
      rows: DisplayItem[] = [],
      keys: string[] = [];
    for (const item of next) {
      const base = rowKey(item),
        count = seen.get(base) ?? 0;
      seen.set(base, count + 1);
      // Rows without an id of their own can repeat one: each still gets a key.
      const key = count ? `${base}#${count}` : base;
      const before = previous.get(key);
      const row = before && same(before, item) ? before : item;
      current.set(key, row);
      rows.push(row);
      keys.push(key);
    }
    previous = current;
    return { rows, keys };
  };
}

/** A row's lasting name: what it is and which it is, never where it sits. */
export function rowKey(item: DisplayItem): string {
  return `${item.type}:${item.id}`;
}

/** Structural equality, for the plain data a row is made of. */
export function same(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!same(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;
  const ak = Object.keys(a as object),
    bk = Object.keys(b as object);
  if (ak.length !== bk.length) return false;
  for (const key of ak)
    if (!same((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key])) return false;
  return true;
}
