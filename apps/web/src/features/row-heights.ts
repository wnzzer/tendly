import type { DisplayItem } from "./tool-runs";

/**
 * How tall a row will be before it has been seen, close enough that the
 * conversation does not have to correct the scroll position much when it is:
 * every correction while scrolling is a forced layout, and a visible hitch on
 * a phone's momentum scroll. Rough on purpose: the real height replaces it.
 */
export function estimateRowHeight(item: DisplayItem, width: number): number {
  if (item.type === "tool_run") return item.tools.length > 1 ? 46 : 46 * item.tools.length;
  if (item.type !== "message") return 64;
  const text = item.text,
    user = item.role === "user";
  // About 7.5px a character at the body size; a user's bubble is narrower and padded.
  const chars = Math.max(20, Math.floor(((user ? width * 0.8 : Math.min(width, 820)) - (user ? 34 : 0)) / 7.5));
  let height = user ? 46 : 58,
    fence = false;
  for (const line of text.split("\n")) {
    if (line.trimStart().startsWith("```")) {
      height += fence ? 22 : 44;
      fence = !fence;
      continue;
    }
    if (fence) {
      height += 20;
      continue;
    }
    if (!line.trim()) {
      height += 8;
      continue;
    }
    if (/^\s*\|/.test(line)) {
      height += 34;
      continue;
    }
    if (/^#{1,6}\s/.test(line)) {
      height += 38;
      continue;
    }
    height += 24 * Math.max(1, Math.ceil(line.length / chars));
  }
  return Math.round(height);
}

/**
 * The heights rows were measured at, by session, row and width: a tab that
 * comes back, or a view rebuilt, lays its rows out at their real size at once.
 * Bounded; the oldest are let go first.
 */
export function createRowHeights(limit = 20000) {
  const heights = new Map<string, number>();
  const key = (session: string, row: string, width: number) => `${session}\u0000${row}\u0000${Math.round(width / 40)}`;
  return {
    get(session: string, row: string, width: number) {
      return heights.get(key(session, row, width));
    },
    remember(session: string, row: string, width: number, height: number) {
      if (!(height > 0)) return;
      const k = key(session, row, width);
      heights.delete(k);
      heights.set(k, height);
      while (heights.size > limit) heights.delete(heights.keys().next().value!);
    },
    get size() {
      return heights.size;
    },
  };
}

export const rowHeights = createRowHeights();
