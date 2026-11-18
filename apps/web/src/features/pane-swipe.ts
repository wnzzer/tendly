/**
 * Swiping sideways between open views on a phone, where only one is shown.
 *
 * The same surface also scrolls, selects text and hosts code blocks that pan
 * sideways, so a swipe has to earn its place: it is decided only once the
 * finger has clearly moved sideways, never from inside something that scrolls
 * or edits horizontally, and it commits only past a distance or a flick.
 */

/** Movement before the gesture picks an axis; below it a touch is still a tap. */
export const SWIPE_LOCK_PX = 12;
/** Holding still this long first is a long press, which is how text gets selected. */
export const SWIPE_LONG_PRESS_MS = 450;
/** A touch this close to a screen edge belongs to the browser's own back gesture. */
export const SWIPE_EDGE_PX = 18;

/** Which way the finger is travelling, once that is clear; `undefined` while it is not. */
export function swipeAxis(dx: number, dy: number): "x" | "y" | undefined {
  if (Math.hypot(dx, dy) < SWIPE_LOCK_PX) return undefined;
  // Sideways has to dominate: a slightly diagonal scroll stays a scroll.
  return Math.abs(dx) > Math.abs(dy) * 1.4 ? "x" : "y";
}

/**
 * How far the view follows the finger. Past the first or last view it gives a
 * little and resists, so the end of the row is felt rather than silent.
 */
export function swipeOffset(dx: number, hasNeighbor: boolean): number {
  return hasNeighbor ? dx : dx * 0.22;
}

/**
 * Where a released swipe goes: `1` is the next view (finger moved left), `-1`
 * the previous one, `0` back to where it was. A short flick counts as much as
 * a long drag, which is how a phone's own pagers behave.
 */
export function swipeOutcome(gesture: {
  dx: number;
  elapsedMs: number;
  width: number;
  hasPrevious: boolean;
  hasNext: boolean;
}): -1 | 0 | 1 {
  const { dx, elapsedMs, width, hasPrevious, hasNext } = gesture;
  const distance = Math.abs(dx);
  const flick = distance >= 36 && distance / Math.max(elapsedMs, 1) >= 0.45;
  if (distance < Math.max(width, 1) * 0.28 && !flick) return 0;
  if (dx < 0) return hasNext ? 1 : 0;
  return hasPrevious ? -1 : 0;
}

export interface SwipeTarget {
  parentElement: SwipeTarget | null;
  tagName?: string;
  isContentEditable?: boolean;
  scrollWidth: number;
  clientWidth: number;
}

/**
 * Whether a touch starting on `target` must be left to the page: a field being
 * typed in, a control that drags, or anything that itself scrolls sideways --
 * a code block, a diff, a wide table. `overflowX` reads the computed style.
 */
export function swipeBlockedAt(
  target: SwipeTarget | null,
  root: SwipeTarget | null,
  overflowX: (element: SwipeTarget) => string,
): boolean {
  for (let element = target; element && element !== root; element = element.parentElement) {
    const tag = element.tagName?.toLowerCase();
    if (tag === "input" || tag === "textarea" || tag === "select" || element.isContentEditable) return true;
    if (element.scrollWidth > element.clientWidth + 1) {
      const overflow = overflowX(element);
      if (overflow === "auto" || overflow === "scroll") return true;
    }
  }
  return false;
}
