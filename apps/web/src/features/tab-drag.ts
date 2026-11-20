import { ref } from "vue";

/**
 * The tab being dragged, shared by every pane: a drag's payload cannot be
 * read until the drop, and a strip needs its width to open a gap of that size
 * as the drag passes over it, and its id to leave alone a drop that would not
 * move it. Null for drags that are not of a tab (a sidebar row, a file).
 */
export const draggedTab = ref<{ id: string; title: string; width: number } | null>(null);

/**
 * Where a drop at `x` (in the strip's own coordinates) lands among tabs laid
 * out at `slots`: before the first tab whose middle is past it, else last.
 * Measured on the strip's layout, not on the tabs as drawn, so tabs sliding
 * aside to make room do not move the target under a still pointer.
 */
export function tabInsertion(x: number, slots: { id: string; left: number; width: number }[]): string | null {
  return slots.find(slot => x < slot.left + slot.width / 2)?.id ?? null;
}

/** Whether a drop before `before` leaves `dragged` where it already is in `ids`. */
export function insertionIsNoop(ids: string[], dragged: string | undefined, before: string | null): boolean {
  const index = dragged ? ids.indexOf(dragged) : -1;
  if (index < 0) return false;
  return before === dragged || before === (ids[index + 1] ?? null);
}
